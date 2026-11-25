use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::config::{Config, QualityTier};
use crate::discovery::{new_hidden_command, resolve_binary};
use crate::profiles::EncoderProfile;

pub const DEFAULT_SEGMENT_DURATION_SECS: u32 = 1;

/// Retorna la ruta por defecto al buffer circular en memoria RAM: `/dev/shm/sane-rec-<UID>/` en Linux o directorio temp en Windows.
pub fn default_buffer_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        let uid = unsafe { libc::getuid() };
        PathBuf::from(format!("/dev/shm/sane-rec-{}", uid))
    }
    #[cfg(not(target_os = "linux"))]
    {
        std::env::temp_dir().join("sane-rec-buffer")
    }
}

/// Estado actual del motor de grabación y repetición instantánea.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EngineStatus {
    pub is_running: bool,
    pub is_recording: bool,
    pub duration_secs: u32,
    pub current_buffer_secs: u32,
    pub recording_elapsed_secs: u32,
    pub ram_usage_mb: f64,
    pub max_memory_mb: u32,
    pub active_profile: String,
    pub output_dir: PathBuf,
}

/// Backend de captura de video para Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsCaptureBackend {
    /// Desktop Duplication API (DXGI / D3D11) - Captura de pantalla a 60 FPS con aceleración de hardware.
    DdaGrab,
    /// GDI Screen Grabber - Fallback universal de compatibilidad y captura de ventanas individuales.
    GdiGrab,
}

#[derive(Debug)]
pub enum EngineError {
    Io(std::io::Error),
    NoSegments,
    FfmpegError(String),
    AlreadyRunning,
    NotRunning,
    ConfigError(String),
    Custom(String),
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::Io(e) => write!(f, "Error de E/S: {}", e),
            EngineError::NoSegments => write!(
                f,
                "No hay segmentos disponibles en el búfer para guardar el replay"
            ),
            EngineError::FfmpegError(msg) => write!(f, "Error de FFmpeg: {}", msg),
            EngineError::AlreadyRunning => {
                write!(f, "El búfer de repetición o la grabación ya se encuentra en ejecución")
            }
            EngineError::NotRunning => {
                write!(f, "El búfer de repetición o la grabación no está en ejecución")
            }
            EngineError::ConfigError(msg) => write!(f, "Error de configuración: {}", msg),
            EngineError::Custom(msg) => write!(f, "Error del motor: {}", msg),
        }
    }
}

impl std::error::Error for EngineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            EngineError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for EngineError {
    fn from(e: std::io::Error) -> Self {
        EngineError::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, EngineError>;

/// Motor principal de sane-rec para repetición instantánea y grabación continua con `wf-recorder`.
pub struct ReplayEngine {
    config: Arc<Mutex<Config>>,
    buffer_dir: PathBuf,
    replay_process: Mutex<Option<Child>>,
    continuous_process: Mutex<Option<Child>>,
    continuous_output: Mutex<Option<PathBuf>>,
    start_time: Mutex<Option<Instant>>,
    continuous_start_time: Mutex<Option<Instant>>,
    audio_modules: Mutex<Vec<u32>>,
    pruner_running: Arc<AtomicBool>,
    replay_session: Arc<std::sync::atomic::AtomicU64>,
    start_lock: Mutex<()>,
    is_saving: Arc<AtomicBool>,
    #[allow(dead_code)]
    win_ram_buffer: Arc<Mutex<std::collections::VecDeque<(Instant, Arc<Vec<u8>>)>>>,
    #[allow(dead_code)]
    win_replay_session: Arc<std::sync::atomic::AtomicU64>,
    /// Ganancia en vivo (f32 bits) del audio de sistema/mic en Windows.
    #[allow(dead_code)]
    win_gain_system: Arc<std::sync::atomic::AtomicU32>,
    #[allow(dead_code)]
    win_gain_mic: Arc<std::sync::atomic::AtomicU32>,
    #[allow(dead_code)]
    win_replay_audio_stop: Mutex<Option<Arc<AtomicBool>>>,
    #[allow(dead_code)]
    win_continuous_audio_stop: Mutex<Option<Arc<AtomicBool>>>,
    #[allow(dead_code)]
    win_replay_backend: Mutex<WindowsCaptureBackend>,
    #[allow(dead_code)]
    win_continuous_backend: Mutex<WindowsCaptureBackend>,
    #[allow(dead_code)]
    continuous_stderr_log: Arc<Mutex<String>>,
    #[allow(dead_code)]
    win_replay_stderr_log: Arc<Mutex<String>>,
    #[allow(dead_code)]
    win_replay_executed_cmd: Mutex<Option<String>>,
    #[allow(dead_code)]
    win_continuous_executed_cmd: Mutex<Option<String>>,
    #[allow(dead_code)]
    win_last_video_ms: Arc<std::sync::atomic::AtomicU64>,
    #[allow(dead_code)]
    win_replay_start_ms: Arc<std::sync::atomic::AtomicU64>,
    #[allow(dead_code)]
    win_stale_restarts: Mutex<Vec<Instant>>,
    #[allow(dead_code)]
    win_force_gdi_once: AtomicBool,
}

/// Milisegundos monotónicos desde el primer uso en el proceso.
#[cfg(target_os = "windows")]
fn mono_ms() -> u64 {
    static T0: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    T0.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Devuelve `true` si el chunk MPEG-TS contiene un inicio de PES de vídeo (PID 0x100 con PUSI).
#[cfg(any(target_os = "windows", test))]
pub(crate) fn chunk_has_video_pusi(data: &[u8]) -> bool {
    data.chunks_exact(188).any(|pkt| {
        pkt[0] == 0x47
            && (pkt[1] & 0x40) != 0
            && ((((pkt[1] as u16) & 0x1F) << 8) | pkt[2] as u16) == 0x100
    })
}

/// Localiza el primer byte de sincronización MPEG-TS (`0x47`) válido, verificando `pos + 188` cuando hay bytes suficientes.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn find_ts_sync(leftover: &[u8]) -> Option<usize> {
    (0..leftover.len()).find(|&pos| {
        leftover[pos] == 0x47
            && (leftover.len() <= pos + 188 || leftover.get(pos + 188) == Some(&0x47))
    })
}

/// Calcula el límite dinámico de RAM (MB) para `win_ram_buffer` según duración y `QualityTier`.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn win_ram_buffer_max_mb(duration_secs: u32, max_memory_mb: u32, tier: QualityTier) -> usize {
    let factor = match tier {
        QualityTier::Low => 12,
        QualityTier::Medium => 16,
        QualityTier::High => 22,
    };
    ((duration_secs as usize) * factor).max(max_memory_mb as usize)
}

impl ReplayEngine {
    /// Crea un nuevo motor utilizando la configuración dada y el directorio de RAM por defecto `/dev/shm/sane-rec-<UID>/`.
    pub fn new(config: Config) -> Result<Self> {
        Self::with_buffer_dir(config, default_buffer_dir())
    }

    /// Crea un nuevo motor especificando la ruta al directorio de búfer en memoria.
    pub fn with_buffer_dir(config: Config, buffer_dir: PathBuf) -> Result<Self> {
        // Asegura que exista el directorio en RAM
        let _ = std::fs::create_dir_all(&buffer_dir);

        let engine = Self {
            config: Arc::new(Mutex::new(config)),
            buffer_dir,
            replay_process: Mutex::new(None),
            continuous_process: Mutex::new(None),
            continuous_output: Mutex::new(None),
            start_time: Mutex::new(None),
            continuous_start_time: Mutex::new(None),
            audio_modules: Mutex::new(Vec::new()),
            pruner_running: Arc::new(AtomicBool::new(false)),
            replay_session: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            start_lock: Mutex::new(()),
            is_saving: Arc::new(AtomicBool::new(false)),
            win_ram_buffer: Arc::new(Mutex::new(std::collections::VecDeque::new())),
            win_replay_session: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            win_gain_system: Arc::new(std::sync::atomic::AtomicU32::new(1.0f32.to_bits())),
            win_gain_mic: Arc::new(std::sync::atomic::AtomicU32::new(1.0f32.to_bits())),
            win_replay_audio_stop: Mutex::new(None),
            win_continuous_audio_stop: Mutex::new(None),
            win_replay_backend: Mutex::new(WindowsCaptureBackend::DdaGrab),
            win_continuous_backend: Mutex::new(WindowsCaptureBackend::DdaGrab),
            continuous_stderr_log: Arc::new(Mutex::new(String::new())),
            win_replay_stderr_log: Arc::new(Mutex::new(String::new())),
            win_replay_executed_cmd: Mutex::new(None),
            win_continuous_executed_cmd: Mutex::new(None),
            win_last_video_ms: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            win_replay_start_ms: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            win_stale_restarts: Mutex::new(Vec::new()),
            win_force_gdi_once: AtomicBool::new(false),
        };

        // Limpieza automática de segmentos antiguos en RAM al inicializar
        let _ = engine.clean_buffer();

        Ok(engine)
    }

    /// Retorna la ruta al directorio de búfer en RAM.
    pub fn buffer_dir(&self) -> &Path {
        &self.buffer_dir
    }

    /// Retorna una copia de la configuración actual del motor.
    pub fn config(&self) -> Config {
        self.config.lock().unwrap().clone()
    }

    /// Actualiza la configuración interna.
    pub fn update_config(&self, new_config: Config) {
        *self.config.lock().unwrap() = new_config;
    }

    /// Actualiza la duración del búfer en segundos y poda el búfer en RAM de forma inmediata.
    pub fn update_duration(&self, duration_secs: u32) {
        let (max_mb, dur) = {
            let mut cfg = self.config.lock().unwrap();
            cfg.replay.duration_secs = duration_secs;
            let _ = cfg.save();
            (cfg.replay.max_memory_mb, cfg.replay.duration_secs)
        };
        Self::prune_ram_dir(&self.buffer_dir, dur, max_mb);
    }

    /// Actualiza el límite de memoria RAM máximo y poda si es necesario.
    pub fn update_max_memory(&self, max_memory_mb: u32) {
        let dur = {
            let mut cfg = self.config.lock().unwrap();
            cfg.replay.max_memory_mb = max_memory_mb;
            let _ = cfg.save();
            cfg.replay.duration_secs
        };
        Self::prune_ram_dir(&self.buffer_dir, dur, max_memory_mb);
    }

    /// Poda los segmentos antiguos en RAM para respetar tanto el límite temporal como el de memoria.
    pub fn prune_ram_dir(dir: &Path, duration_secs: u32, max_memory_mb: u32) {
        let mut segments: Vec<(PathBuf, Option<std::time::SystemTime>, u64)> = Vec::new();

        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() {
                    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or_default();
                    if (name.starts_with("seg_") || name.starts_with("segment_"))
                        && (ext == "mp4" || ext == "ts")
                        && !name.contains('%')
                    {
                        if let Ok(meta) = entry.metadata() {
                            if meta.len() > 0 {
                                segments.push((p, meta.modified().ok(), meta.len()));
                            }
                        }
                    }
                }
            }
        }

        if segments.is_empty() {
            return;
        }

        // Orden cronológico (más antiguo primero) con desempate por nombre
        segments.sort_by(|a, b| match (a.1, b.1) {
            (Some(ta), Some(tb)) => ta.cmp(&tb).then_with(|| a.0.cmp(&b.0)),
            _ => a.0.cmp(&b.0),
        });

        // 1. Podar si excede la duración requerida + 8 segundos de margen
        let max_keep = (duration_secs as usize) + 8;
        while segments.len() > max_keep {
            let oldest = segments.remove(0);
            let _ = std::fs::remove_file(&oldest.0);
        }

        // 2. Podar si la suma total en RAM excede max_memory_mb
        let max_bytes = (max_memory_mb as u64) * 1024 * 1024;
        let mut total_bytes: u64 = segments.iter().map(|s| s.2).sum();
        while total_bytes > max_bytes && segments.len() > 3 {
            let oldest = segments.remove(0);
            total_bytes = total_bytes.saturating_sub(oldest.2);
            let _ = std::fs::remove_file(&oldest.0);
        }
    }

    /// Carga un módulo pactl y retorna su ID numérico si fue exitoso.
    fn load_pactl_module(args: &[&str]) -> Option<u32> {
        let output = new_hidden_command("pactl")
            .arg("load-module")
            .args(args)
            .output()
            .ok()?;

        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            text.trim().parse::<u32>().ok()
        } else {
            None
        }
    }

    /// Descarga un módulo pactl dado su ID numérico.
    fn unload_pactl_module(id: u32) {
        let _ = new_hidden_command("pactl")
            .args(["unload-module", &id.to_string()])
            .output();
    }

    /// Descarga y limpia todos los módulos PipeWire/PulseAudio cargados por sane-rec,
    /// incluyendo posibles módulos huérfanos de sesiones previas terminadas abruptamente.
    pub fn cleanup_audio_modules(&self) {
        let mut modules = self.audio_modules.lock().unwrap();
        for id in modules.drain(..).rev() {
            Self::unload_pactl_module(id);
        }
        Self::cleanup_orphaned_audio_modules();
    }

    /// Limpia cualquier módulo huérfano de sane-rec dejado en PulseAudio / PipeWire por una terminación anómala previa.
    pub fn cleanup_orphaned_audio_modules() {
        let output = match new_hidden_command("pactl").args(["list", "modules"]).output() {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
            _ => return,
        };

        let mut current_id: Option<u32> = None;
        let mut is_sane_rec = false;

        for line in output.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("Module #") {
                if is_sane_rec {
                    if let Some(id) = current_id {
                        Self::unload_pactl_module(id);
                    }
                }
                current_id = trimmed
                    .trim_start_matches("Module #")
                    .trim()
                    .parse::<u32>()
                    .ok();
                is_sane_rec = false;
            } else if trimmed.contains("sane_rec_mix")
                || trimmed.contains("sane_rec_system")
                || trimmed.contains("sane_rec_mic")
            {
                is_sane_rec = true;
            }
        }

        if is_sane_rec {
            if let Some(id) = current_id {
                Self::unload_pactl_module(id);
            }
        }
    }

    /// Retorna una copia de la lista de IDs de módulos de audio activos actualmente.
    pub fn audio_modules(&self) -> Vec<u32> {
        self.audio_modules.lock().unwrap().clone()
    }

    /// Configura el mezclador dual PipeWire creando un null-sink virtual y loopbacks.
    /// Retorna el nombre del monitor mezclado (`sane_rec_mix.monitor`) si tuvo éxito.
    fn setup_dual_audio(&self, desktop_sink: &str, mic_source: &str) -> Option<String> {
        // Si los módulos ya están cargados (p. ej. por coexistencia con grabación continua), se reutilizan
        {
            let loaded = self.audio_modules.lock().unwrap();
            if !loaded.is_empty() {
                return Some("sane_rec_mix.monitor".to_string());
            }
        }

        self.cleanup_audio_modules();

        let mut loaded = Vec::new();

        // 1. pactl load-module module-null-sink
        // Aislamiento PipeWire / WirePlumber / EasyEffects:
        // - device.intended_roles=record: indica que el sink es exclusivo para grabación interna.
        // - node.passive=true y node.virtual=true: previene que EasyEffects o WirePlumber lo capturen
        //   como un dispositivo de salida estándar o enruten aplicaciones a él.
        // - node.dont-reconnect=true: evita reconexiones dinámicas indeseadas.
        let sink_id = Self::load_pactl_module(&[
            "module-null-sink",
            "sink_name=sane_rec_mix",
            "sink_properties=device.description=SaneRecMix device.intended_roles=record node.passive=true node.virtual=true node.dont-reconnect=true",
        ]);

        loaded.push(sink_id?);

        // 2. pactl load-module module-loopback para el audio del sistema / escritorio
        let desktop_monitor = if desktop_sink.ends_with(".monitor") {
            desktop_sink.to_string()
        } else {
            format!("{}.monitor", desktop_sink)
        };

        let loop1_id = Self::load_pactl_module(&[
            "module-loopback",
            &format!("source={}", desktop_monitor),
            "sink=sane_rec_mix",
            "source_dont_move=true",
            "sink_dont_move=true",
            "latency_msec=40",
            "source_output_properties=media.role=sane_rec_system node.passive=true node.dont-reconnect=true node.virtual=true",
            "sink_input_properties=media.role=sane_rec_system node.passive=true node.dont-reconnect=true node.virtual=true",
        ]);

        match loop1_id {
            Some(id) => loaded.push(id),
            None => {
                for id in loaded.drain(..).rev() {
                    Self::unload_pactl_module(id);
                }
                return None;
            }
        };

        // 3. pactl load-module module-loopback para el micrófono
        let loop2_id = Self::load_pactl_module(&[
            "module-loopback",
            &format!("source={}", mic_source),
            "sink=sane_rec_mix",
            "source_dont_move=true",
            "sink_dont_move=true",
            "latency_msec=40",
            "source_output_properties=media.role=sane_rec_mic node.passive=true node.dont-reconnect=true node.virtual=true",
            "sink_input_properties=media.role=sane_rec_mic node.passive=true node.dont-reconnect=true node.virtual=true",
        ]);

        match loop2_id {
            Some(id) => loaded.push(id),
            None => {
                for id in loaded.drain(..).rev() {
                    Self::unload_pactl_module(id);
                }
                return None;
            }
        };

        *self.audio_modules.lock().unwrap() = loaded;

        // Aplica inmediatamente los volúmenes configurados a los loopbacks
        let (vol_sys, vol_mic) = {
            let c = self.config.lock().unwrap();
            (c.audio.volume_system, c.audio.volume_mic)
        };
        Self::apply_sink_input_volume("sane_rec_system", vol_sys);
        Self::apply_sink_input_volume("sane_rec_mic", vol_mic);

        Some("sane_rec_mix.monitor".to_string())
    }

    /// Aplica la ganancia de volumen (en porcentaje) a todos los Sink Inputs de PipeWire identificados por `media.role`.
    pub fn apply_sink_input_volume(role: &str, volume: f32) {
        let percent = (volume * 100.0).round().clamp(0.0, 150.0) as u32;
        let output = match new_hidden_command("pactl").args(["list", "sink-inputs"]).output() {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
            _ => return,
        };

        let mut current_id: Option<u32> = None;
        for line in output.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("Sink Input #") {
                current_id = trimmed
                    .trim_start_matches("Sink Input #")
                    .trim()
                    .parse::<u32>()
                    .ok();
            } else if trimmed.contains(&format!("media.role = \"{}\"", role)) {
                if let Some(id) = current_id {
                    let _ = new_hidden_command("pactl")
                        .args(["set-sink-input-volume", &id.to_string(), &format!("{}%", percent)])
                        .output();
                    current_id = None;
                }
            }
        }
    }

    /// Actualiza el volumen del sistema/juego tanto en la configuración como en los loopbacks activos.
    pub fn set_volume_system(&self, volume: f32) {
        {
            let mut cfg = self.config.lock().unwrap();
            cfg.audio.volume_system = volume;
        }
        #[cfg(target_os = "windows")]
        self.win_gain_system.store(volume.clamp(0.0, 1.5).to_bits(), Ordering::Relaxed);
        #[cfg(not(target_os = "windows"))]
        Self::apply_sink_input_volume("sane_rec_system", volume);
    }

    /// Actualiza el volumen del micrófono exclusivamente en su canal interno de grabación (sane_rec_mic).
    /// NO altera la ganancia del micrófono físico del sistema operativo.
    pub fn set_volume_mic(&self, volume: f32) {
        {
            let mut cfg = self.config.lock().unwrap();
            cfg.audio.volume_mic = volume;
        }
        #[cfg(target_os = "windows")]
        self.win_gain_mic.store(volume.clamp(0.0, 1.5).to_bits(), Ordering::Relaxed);
        #[cfg(not(target_os = "windows"))]
        Self::apply_sink_input_volume("sane_rec_mic", volume);
    }

    /// Aplica los argumentos de audio al comando wf-recorder.
    fn apply_audio_args(&self, cmd: &mut Command, config: &Config) {
        let capture_desktop = config.audio.capture_desktop;
        let capture_mic = config.audio.capture_mic;

        if capture_desktop && capture_mic {
            let sink = get_default_audio_sink().unwrap_or_else(|| "default".to_string());
            let mic = get_default_audio_source().unwrap_or_else(|| "default".to_string());

            if let Some(mix_source) = self.setup_dual_audio(&sink, &mic) {
                cmd.arg(format!("-a{}", mix_source));
                cmd.args(["-C", "aac"]);
                return;
            }
        } else {
            // Si no se requiere mezcla dual, asegura que cualquier módulo de mezcla previo quede liberado
            self.cleanup_audio_modules();
        }

        if capture_desktop {
            if let Some(sink) = get_default_audio_sink() {
                let monitor = if sink.ends_with(".monitor") {
                    sink
                } else {
                    format!("{}.monitor", sink)
                };
                cmd.arg(format!("-a{}", monitor));
            } else {
                cmd.arg("-a");
            }
            cmd.args(["-C", "aac"]);
        } else if capture_mic {
            if let Some(mic) = get_default_audio_source() {
                cmd.arg(format!("-a{}", mic));
            } else {
                cmd.arg("-a");
            }
            cmd.args(["-C", "aac"]);
        }
    }

    /// Construye el comando wf-recorder para el búfer rodante en memoria.
    fn build_replay_command(&self, config: &Config) -> Command {
        let mut cmd = new_hidden_command("wf-recorder");
        cmd.arg("-y");

        // Configuración de destino (monitor o geometría)
        apply_target_args(&mut cmd, &config.video.target);

        // Audio del sistema / micrófono / dual mixing
        self.apply_audio_args(&mut cmd, config);

        // Perfil de codificación
        apply_profile_args(&mut cmd, config);

        // Segmenter en RAM con microsegmentos de 1 segundo e intervalo keyint exacto de 1 segundo en formato MPEG-TS:
        let wrap_n = config.replay.duration_secs + 10;
        let seg_pattern = format!("{}/seg_%05d.ts", self.buffer_dir.display());

        cmd.args([
            "-m",
            "segment",
            "-p",
            "segment_time=1",
            "-p",
            &format!("segment_wrap={}", wrap_n),
            "-p",
            "reset_timestamps=1",
            "-p",
            "keyint=60",
            "-p",
            "g=60",
            "-p",
            "tune=zerolatency",
            "-f",
            &seg_pattern,
        ]);

        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null());

        if let Ok(f) = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(crate::crash_handler::get_engine_log_path())
        {
            cmd.stderr(std::process::Stdio::from(f));
        } else {
            cmd.stderr(std::process::Stdio::null());
        }

        cmd
    }

    /// Construye el comando wf-recorder para la grabación continua directa a disco.
    fn build_continuous_command(&self, config: &Config, output_path: &Path) -> Command {
        let mut cmd = new_hidden_command("wf-recorder");
        cmd.arg("-y");

        // Tasa de cuadros explícita: wf-recorder no debe esperar eventos de daño de pantalla.
        let fps = if config.video.fps == 0 { 60 } else { config.video.fps };
        cmd.args(["-r", &format!("{}", fps)]);

        // Destino de captura
        apply_target_args(&mut cmd, &config.video.target);

        // Audio del sistema / micrófono / dual mixing
        self.apply_audio_args(&mut cmd, config);

        // Perfil de codificación
        apply_profile_args(&mut cmd, config);

        // Archivo de salida directo
        cmd.arg("-f").arg(output_path);

        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null());

        if let Ok(f) = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(crate::crash_handler::get_engine_log_path())
        {
            cmd.stderr(std::process::Stdio::from(f));
        } else {
            cmd.stderr(std::process::Stdio::null());
        }

        cmd
    }

    /// Mata cualquier instancia huérfana previa de `wf-recorder` que esté escribiendo en `buffer_dir`
    /// sin afectar a otros procesos del sistema.
    #[cfg(target_os = "linux")]
    fn kill_orphaned_wf_recorders(buffer_dir: &Path) {
        let buf_str = buffer_dir.to_string_lossy();
        let my_pid = std::process::id();

        // 1. Escanear /proc para localizar y terminar cualquier proceso wf-recorder que apunte a este buffer_dir
        if let Ok(entries) = std::fs::read_dir("/proc") {
            for entry in entries.flatten() {
                if let Ok(name) = entry.file_name().into_string() {
                    if let Ok(pid) = name.parse::<i32>() {
                        if pid as u32 != my_pid {
                            let cmdline_path = entry.path().join("cmdline");
                            if let Ok(bytes) = std::fs::read(cmdline_path) {
                                let cmd_str = String::from_utf8_lossy(&bytes);
                                if cmd_str.contains("wf-recorder") && cmd_str.contains(&*buf_str) {
                                    unsafe {
                                        libc::kill(pid, libc::SIGKILL);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // 2. Ejecutar pkill -KILL -f acotado exclusivamente a wf-recorder y buffer_dir
        let _ = new_hidden_command("pkill")
            .args(["-KILL", "-f", &format!("wf-recorder.*{}", buf_str)])
            .output();

        std::thread::sleep(Duration::from_millis(50));
    }

    /// Inicia el proceso de grabación en búfer rodante en RAM con `wf-recorder` en Linux.
    #[cfg(target_os = "linux")]
    pub fn start_replay(&self) -> Result<()> {
        let _start_guard = self.start_lock.lock().unwrap();
        if self.is_running() {
            return Err(EngineError::AlreadyRunning);
        }

        // Mata cualquier instancia huérfana previa de wf-recorder que esté escribiendo en buffer_dir
        Self::kill_orphaned_wf_recorders(&self.buffer_dir);

        // Limpieza exhaustiva del buffer (.ts, .mp4 y archivos de lista)
        self.clean_buffer()?;
        let _ = std::fs::create_dir_all(&self.buffer_dir);

        let config = self.config();
        let active_profile = config
            .profile
            .active_profile
            .parse::<EncoderProfile>()
            .unwrap_or(EncoderProfile::SaneSafe);

        let is_hardware = matches!(
            active_profile,
            EncoderProfile::AmdHardware | EncoderProfile::NvidiaHardware
        );

        let mut cmd = self.build_replay_command(&config);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                if !self.is_recording() {
                    self.cleanup_audio_modules();
                }
                return Err(e.into());
            }
        };

        // Manejo seguro de inicio y captura de fallos
        std::thread::sleep(std::time::Duration::from_millis(400));
        let failed = match child.try_wait() {
            Ok(Some(status)) => !status.success(),
            Err(_) => true,
            Ok(None) => false,
        };

        if failed {
            let code = child.try_wait().ok().flatten().and_then(|s| s.code());
            crate::crash_handler::log_engine_failure("Inicio del Búfer de Repetición", code);

            if is_hardware {
                let _ = notify_rust::Notification::new()
                    .appname("sane-rec")
                    .summary("Aviso: Hardware encoder no soportado en esta GPU; usando Sane Safe (CPU)")
                    .body("Se ha cambiado automáticamente al perfil seguro por CPU")
                    .icon("dialog-warning")
                    .show();

                let mut fallback_config = config.clone();
                fallback_config.profile.active_profile = EncoderProfile::SaneSafe.to_string();
                let _ = fallback_config.save();
                *self.config.lock().unwrap() = fallback_config.clone();

                self.cleanup_audio_modules();
                let mut fallback_cmd = self.build_replay_command(&fallback_config);
                child = match fallback_cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                if !self.is_recording() {
                    self.cleanup_audio_modules();
                }
                return Err(e.into());
            }
        };
            } else {
                if !self.is_recording() {
                    self.cleanup_audio_modules();
                }
                return Err(EngineError::Custom("wf-recorder falló al iniciar el búfer de repetición. Revisa ~/.config/sane-rec/crash.log".into()));
            }
        }

        *self.replay_process.lock().unwrap() = Some(child);
        *self.start_time.lock().unwrap() = Some(Instant::now());
        self.pruner_running.store(true, Ordering::SeqCst);
        let session_id = self.replay_session.fetch_add(1, Ordering::SeqCst) + 1;

        // Lanza hilo Garbage Collector circular dedicado en /dev/shm para Linux
        let pruner_flag = Arc::clone(&self.pruner_running);
        let session_tracker = Arc::clone(&self.replay_session);
        let pruner_buf = self.buffer_dir.clone();
        let pruner_cfg = Arc::clone(&self.config);
        let is_saving_flag = Arc::clone(&self.is_saving);
        std::thread::Builder::new()
            .name("sane-rec-shm-gc".into())
            .spawn(move || {
                while pruner_flag.load(Ordering::SeqCst) && session_tracker.load(Ordering::SeqCst) == session_id {
                    // Intervalo de 2 segundos en tramos de 100ms para responder rápido al detener
                    for _ in 0..20 {
                        std::thread::sleep(Duration::from_millis(100));
                        if !pruner_flag.load(Ordering::SeqCst) || session_tracker.load(Ordering::SeqCst) != session_id {
                            return;
                        }
                    }

                    if !pruner_buf.exists() {
                        continue;
                    }

                    if is_saving_flag.load(Ordering::Relaxed) {
                        continue;
                    }

                    let (dur_secs, max_mb) = {
                        let c = pruner_cfg.lock().unwrap();
                        (c.replay.duration_secs, c.replay.max_memory_mb)
                    };
                    Self::prune_ram_dir(&pruner_buf, dur_secs, max_mb);
                }
            })
            .ok();

        play_feedback_sound("device-added");

        Ok(())
    }

    /// Detiene el proceso de repetición instantánea enviando SIGINT en Linux.
    #[cfg(target_os = "linux")]
    pub fn stop_replay(&self) -> Result<()> {
        self.pruner_running.store(false, Ordering::SeqCst);
        self.replay_session.fetch_add(1, Ordering::SeqCst);
        let child = self.replay_process.lock().unwrap().take();
        if let Some(mut child) = child {
            unsafe {
                libc::kill(child.id() as i32, libc::SIGINT);
            }
            let _ = child.wait();
        }
        *self.start_time.lock().unwrap() = None;

        // Descarga de módulos de audio virtual PipeWire únicamente si no hay grabación continua activa
        if !self.is_recording() {
            self.cleanup_audio_modules();
        }

        // Limpieza de segmentos en RAM
        let _ = self.clean_buffer();
        play_feedback_sound("device-removed");
        Ok(())
    }

    /// Inicia el proceso de repetición instantánea en Windows transmitiendo MPEG-TS a la RAM física.
    #[cfg(target_os = "windows")]
    pub fn start_replay(&self) -> Result<()> {
        let _start_guard = self.start_lock.lock().unwrap();
        if self.is_running() {
            return Err(EngineError::AlreadyRunning);
        }

        self.win_ram_buffer.lock().unwrap().clear();

        let config = self.config();
        let active_profile = config
            .profile
            .active_profile
            .parse::<EncoderProfile>()
            .unwrap_or(EncoderProfile::SaneSafe);

        let (mut audio_port, mut audio_stop) = match start_windows_audio_feeder(&config, &self.win_gain_system, &self.win_gain_mic) {
            Ok((port, stop)) => (Some(port), Some(stop)),
            Err(e) => {
                eprintln!("Aviso: No se pudo iniciar el feeder de audio WASAPI: {}", e);
                (None, None)
            }
        };

        let build_replay_cmd = |cfg: &Config, port: Option<u16>, backend: WindowsCaptureBackend| -> (Command, WindowsCaptureBackend) {
            let mut cmd = new_hidden_command(resolve_binary("ffmpeg"));
            cmd.args(["-y", "-stats_period", "5"]);

            let actual_backend = apply_windows_input_args(&mut cmd, cfg, port, backend);
            apply_windows_profile_args(&mut cmd, cfg, port.is_some(), actual_backend);

            let fps = cfg.video.fps.max(15);
            let fps_str = fps.to_string();

            // -bsf:v dump_extra solo aplica a H.264/HEVC; en VP9 o AV1 corrompe el MPEG-TS o falla FFmpeg
            let (_, is_h264, is_hevc) = resolve_windows_encoder(cfg);

            cmd.args(["-g", &fps_str, "-keyint_min", &fps_str]);
            if is_h264 {
                cmd.args(["-bsf:v", "dump_extra"]);
            } else if is_hevc {
                cmd.args(["-bsf:v", "dump_extra=keyframe"]);
            }
            cmd.args([
                "-f",
                "mpegts",
                "-mpegts_flags",
                "+resend_headers+pat_pmt_at_frames",
                "pipe:1",
            ]);

            cmd.stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            (cmd, actual_backend)
        };

        let first_backend = if self.win_force_gdi_once.swap(false, Ordering::SeqCst) {
            WindowsCaptureBackend::GdiGrab
        } else {
            WindowsCaptureBackend::DdaGrab
        };
        let (mut cmd, actual_backend) = build_replay_cmd(&config, audio_port, first_backend);
        *self.win_replay_backend.lock().unwrap() = actual_backend;
        *self.win_replay_executed_cmd.lock().unwrap() = Some(render_ffmpeg_command(&cmd));
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                if let Some(stop) = audio_stop.take() {
                    stop.store(true, Ordering::SeqCst);
                }
                return Err(e.into());
            }
        };

        let stderr_log = Arc::clone(&self.win_replay_stderr_log);
        if let Ok(mut g) = stderr_log.lock() {
            g.clear();
        }
        if let Some(mut err_pipe) = child.stderr.take() {
            let stderr_log_clone = Arc::clone(&stderr_log);
            std::thread::Builder::new()
                .name("sane-rec-win-stderr".into())
                .spawn(move || {
                    use std::io::Read;
                    let mut buf = [0u8; 4096];
                    let mut ring = Vec::new();
                    while let Ok(n) = err_pipe.read(&mut buf) {
                        if n == 0 {
                            break;
                        }
                        ring.extend_from_slice(&buf[..n]);
                        if ring.len() > 16384 {
                            let drain_len = ring.len() - 16384;
                            ring.drain(..drain_len);
                        }
                        if let Ok(mut lock) = stderr_log_clone.lock() {
                            *lock = String::from_utf8_lossy(&ring).into_owned();
                        }
                    }
                })
                .ok();
        }

        // Verificación de salud inicial (600ms) para detectar fallos de codificador GPU o ventana inválida
        std::thread::sleep(Duration::from_millis(600));
        let failed = match child.try_wait() {
            Ok(Some(_)) => true,
            Err(_) => true,
            Ok(None) => false,
        };

        if failed {
            let code = child.try_wait().ok().flatten().and_then(|s| s.code());
            crate::crash_handler::log_engine_failure("Inicio del Búfer de Repetición (Windows)", code);
            let err_text = stderr_log.lock().map(|g| g.clone()).unwrap_or_default();
            if !err_text.is_empty() {
                crate::crash_handler::write_crash_report(
                    "Fallo al iniciar búfer de repetición en Windows (FFmpeg stderr)",
                    &err_text,
                );
            }

            if let Some(stop) = audio_stop.take() {
                stop.store(true, Ordering::SeqCst);
            }

            if active_profile != EncoderProfile::SaneSafe
                || config.video.target.starts_with("window:")
                || config.video.target.starts_with("title=")
            {
                // Fallback no destructivo: no se guarda ni se sobrescribe la configuración del usuario
                let mut fallback_config = config.clone();
                fallback_config.profile.active_profile = "sane-safe".to_string();
                fallback_config.video.codec = "libx264".to_string();
                if fallback_config.video.target.starts_with("window:")
                    || fallback_config.video.target.starts_with("title=")
                {
                    fallback_config.video.target = "desktop".to_string();
                }

                let (retry_port, retry_stop) = match start_windows_audio_feeder(&fallback_config, &self.win_gain_system, &self.win_gain_mic) {
                    Ok((port, stop)) => (Some(port), Some(stop)),
                    Err(_) => (None, None),
                };
                audio_port = retry_port;
                audio_stop = retry_stop;

                let (mut fallback_cmd, fallback_backend) = build_replay_cmd(&fallback_config, audio_port, WindowsCaptureBackend::GdiGrab);
                *self.win_replay_backend.lock().unwrap() = fallback_backend;
                *self.win_replay_executed_cmd.lock().unwrap() = Some(render_ffmpeg_command(&fallback_cmd));
                child = match fallback_cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                if let Some(stop) = audio_stop.take() {
                    stop.store(true, Ordering::SeqCst);
                }
                return Err(e.into());
            }
        };

                if let Ok(mut g) = stderr_log.lock() {
                    g.clear();
                }
                if let Some(mut err_pipe) = child.stderr.take() {
                    let fallback_stderr_clone = Arc::clone(&stderr_log);
                    std::thread::Builder::new()
                        .name("sane-rec-win-fallback-stderr".into())
                        .spawn(move || {
                            use std::io::Read;
                            let mut buf = [0u8; 4096];
                            let mut ring = Vec::new();
                            while let Ok(n) = err_pipe.read(&mut buf) {
                                if n == 0 {
                                    break;
                                }
                                ring.extend_from_slice(&buf[..n]);
                                if ring.len() > 16384 {
                                    let drain_len = ring.len() - 16384;
                                    ring.drain(..drain_len);
                                }
                                if let Ok(mut lock) = fallback_stderr_clone.lock() {
                                    *lock = String::from_utf8_lossy(&ring).into_owned();
                                }
                            }
                        })
                        .ok();
                }

                std::thread::sleep(Duration::from_millis(600));
                if let Ok(Some(status)) = child.try_wait() {
                    if let Some(stop) = audio_stop.take() {
                        stop.store(true, Ordering::SeqCst);
                    }
                    let err_text = stderr_log.lock().map(|g| g.clone()).unwrap_or_default();
                    if !err_text.is_empty() {
                        crate::crash_handler::write_crash_report(
                            "Fallo de fallback en búfer de repetición (FFmpeg stderr)",
                            &err_text,
                        );
                    }
                    return Err(EngineError::Custom(format!(
                        "ffmpeg falló al iniciar el búfer de repetición (código {:?})",
                        status.code()
                    )));
                }
            } else {
                return Err(EngineError::Custom(
                    "ffmpeg falló al iniciar el búfer de repetición en Windows.".into(),
                ));
            }
        }

        let mut stdout = match child.stdout.take() {
            Some(s) => s,
            None => {
                if let Some(stop) = audio_stop.take() {
                    stop.store(true, Ordering::SeqCst);
                }
                let _ = child.kill();
                let _ = child.wait();
                return Err(EngineError::Custom("Fallo al capturar stdout de ffmpeg".into()));
            }
        };
        *self.win_replay_audio_stop.lock().unwrap() = audio_stop;

        *self.replay_process.lock().unwrap() = Some(child);
        *self.start_time.lock().unwrap() = Some(Instant::now());
        let started_ms = mono_ms();
        self.win_replay_start_ms.store(started_ms, Ordering::SeqCst);
        self.win_last_video_ms.store(started_ms, Ordering::SeqCst);
        let session_id = self.win_replay_session.fetch_add(1, Ordering::SeqCst) + 1;

        let ram_buf = Arc::clone(&self.win_ram_buffer);
        let session_tracker = Arc::clone(&self.win_replay_session);
        let last_video = Arc::clone(&self.win_last_video_ms);
        let dur_secs = config.replay.duration_secs;
        // Escalar dinámicamente el límite de RAM según la duración solicitada y el tier de calidad
        // (Low=12 MB/s, Medium=16 MB/s, High=22 MB/s) para garantizar que repeticiones largas no se trunquen
        let max_mb = win_ram_buffer_max_mb(
            dur_secs,
            config.replay.max_memory_mb,
            config.profile.quality_tier,
        );
        let max_bytes = max_mb * 1024 * 1024;

        std::thread::Builder::new()
            .name("sane-rec-win-ram-buffer".into())
            .spawn(move || {
                use std::io::Read;
                let mut raw_buf = vec![0u8; 65424]; // 348 paquetes MPEG-TS de 188 bytes
                let mut leftover: Vec<u8> = Vec::with_capacity(376);
                let mut current_bytes: usize = 0;
                while session_tracker.load(Ordering::Relaxed) == session_id {
                    match stdout.read(&mut raw_buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            leftover.extend_from_slice(&raw_buf[..n]);
                            // Alinear al primer byte de sincronización MPEG-TS (0x47)
                            if let Some(sync_pos) = leftover.iter().position(|&b| b == 0x47) {
                                if sync_pos > 0 {
                                    leftover.drain(..sync_pos);
                                }
                            } else {
                                leftover.clear();
                                continue;
                            }
                            let aligned_len = (leftover.len() / 188) * 188;
                            if aligned_len == 0 {
                                continue;
                            }
                            let chunk: Vec<u8> = leftover.drain(..aligned_len).collect();
                            if chunk_has_video_pusi(&chunk) {
                                last_video.store(mono_ms(), Ordering::SeqCst);
                            }
                            let chunk_arc = Arc::new(chunk);
                            let chunk_len = chunk_arc.len();
                            let now = Instant::now();
                            if let Ok(mut q) = ram_buf.lock() {
                                q.push_back((now, Arc::clone(&chunk_arc)));
                                current_bytes += chunk_len;

                                let cutoff = Duration::from_secs((dur_secs + 3) as u64);
                                while let Some(front) = q.front() {
                                    if now.duration_since(front.0) > cutoff {
                                        if let Some(removed) = q.pop_front() {
                                            current_bytes = current_bytes.saturating_sub(removed.1.len());
                                        }
                                    } else {
                                        break;
                                    }
                                }

                                while current_bytes > max_bytes && q.len() > 10 {
                                    if let Some(removed) = q.pop_front() {
                                        current_bytes = current_bytes.saturating_sub(removed.1.len());
                                    } else {
                                        break;
                                    }
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
            })
            .ok();

        play_feedback_sound("device-added");
        Ok(())
    }

    /// Detiene el proceso de repetición instantánea en Windows y libera el buffer en RAM.
    #[cfg(target_os = "windows")]
    pub fn stop_replay(&self) -> Result<()> {
        self.win_replay_session.fetch_add(1, Ordering::SeqCst);
        if let Some(stop_flag) = self.win_replay_audio_stop.lock().unwrap().take() {
            stop_flag.store(true, Ordering::SeqCst);
        }
        let child = self.replay_process.lock().unwrap().take();
        if let Some(mut child) = child {
            if let Some(mut stdin) = child.stdin.take() {
                use std::io::Write;
                let _ = stdin.write_all(b"q\n");
                let _ = stdin.flush();
            }
            let _ = child.wait();
        }
        *self.start_time.lock().unwrap() = None;
        self.win_ram_buffer.lock().unwrap().clear();
        play_feedback_sound("device-removed");
        Ok(())
    }

    /// Alterna el estado del búfer de repetición instantánea (arranca o detiene).
    pub fn toggle_replay(&self) -> Result<bool> {
        if self.is_running() {
            if let Some(start_time) = *self.start_time.lock().unwrap() {
                if start_time.elapsed() < std::time::Duration::from_millis(1200) {
                    return Ok(true);
                }
            }
            self.stop_replay()?;
            Ok(false)
        } else {
            self.start_replay()?;
            Ok(true)
        }
    }

    /// Indica si el búfer de vídeo de Windows lleva demasiado sin recibir vídeo (stall).
    /// Tras 2 stalls en 60s fuerza GdiGrab en el siguiente inicio del replay.
    pub fn win_video_stale(&self) -> bool {
        #[cfg(not(target_os = "windows"))]
        {
            false
        }
        #[cfg(target_os = "windows")]
        {
            if !self.is_running() {
                return false;
            }
            let now_ms = mono_ms();
            let start_ms = self.win_replay_start_ms.load(Ordering::SeqCst);
            let last_ms = self.win_last_video_ms.load(Ordering::SeqCst);
            if now_ms.saturating_sub(start_ms) <= 6000 || now_ms.saturating_sub(last_ms) <= 4000 {
                return false;
            }
            let now = Instant::now();
            let mut restarts = self.win_stale_restarts.lock().unwrap();
            restarts.retain(|t| now.duration_since(*t) <= Duration::from_secs(60));
            if restarts.last().map_or(true, |t| now.duration_since(*t) >= Duration::from_secs(6)) { restarts.push(now); }
            if restarts.len() >= 2 {
                self.win_force_gdi_once.store(true, Ordering::SeqCst);
            }
            true
        }
    }

    /// Indica si el proceso de repetición instantánea se encuentra vivo y en ejecución.
    pub fn is_running(&self) -> bool {
        let mut guard = self.replay_process.lock().unwrap();
        if let Some(ref mut child) = *guard {
            match child.try_wait() {
                Ok(None) => true,
                Ok(Some(status)) => {
                    if !status.success() {
                        crate::crash_handler::log_engine_failure(
                            "Búfer de Repetición en Segundo Plano",
                            status.code(),
                        );
                    }
                    *guard = None;
                    false
                }
                Err(_) => {
                    *guard = None;
                    false
                }
            }
        } else {
            false
        }
    }

    /// Inicia la grabación continua directa al directorio de almacenamiento con `wf-recorder` en Linux.
    #[cfg(target_os = "linux")]
    pub fn start_continuous_recording(&self) -> Result<PathBuf> {
        let _start_guard = self.start_lock.lock().unwrap();
        if self.is_recording() {
            return Err(EngineError::AlreadyRunning);
        }

        let config = self.config();
        config.ensure_output_dir().map_err(|e| {
            EngineError::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
        })?;

        let timestamp = chrono::Local::now()
            .format("Recording_%Y-%m-%d_%H-%M-%S")
            .to_string();
        let filename = format!("{}.{}", timestamp, config.storage.format);
        let output_path = config.storage.output_dir.join(filename);

        let active_profile = config
            .profile
            .active_profile
            .parse::<EncoderProfile>()
            .unwrap_or(EncoderProfile::SaneSafe);

        let is_hardware = matches!(
            active_profile,
            EncoderProfile::AmdHardware | EncoderProfile::NvidiaHardware
        );

        let mut cmd = self.build_continuous_command(&config, &output_path);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                if !self.is_running() {
                    self.cleanup_audio_modules();
                }
                return Err(e.into());
            }
        };

        // Manejo seguro de inicio y captura de fallos
        std::thread::sleep(std::time::Duration::from_millis(600));
        let failed = match child.try_wait() {
            Ok(Some(status)) => !status.success(),
            Err(_) => true,
            Ok(None) => false,
        };

        if failed {
            let code = child.try_wait().ok().flatten().and_then(|s| s.code());
            crate::crash_handler::log_engine_failure("Inicio de Grabación Continua", code);
            let _ = std::fs::remove_file(&output_path);

            if is_hardware {
                let _ = notify_rust::Notification::new()
                    .appname("sane-rec")
                    .summary("Aviso: Hardware encoder no soportado en esta GPU; usando Sane Safe (CPU)")
                    .body("Se ha cambiado automáticamente al perfil seguro por CPU")
                    .icon("dialog-warning")
                    .show();

                let mut fallback_config = config.clone();
                fallback_config.profile.active_profile = EncoderProfile::SaneSafe.to_string();
                *self.config.lock().unwrap() = fallback_config.clone();

                self.cleanup_audio_modules();
                let mut fallback_cmd = self.build_continuous_command(&fallback_config, &output_path);
                child = match fallback_cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                if !self.is_running() {
                    self.cleanup_audio_modules();
                }
                return Err(e.into());
            }
        };
            } else {
                if !self.is_running() {
                    self.cleanup_audio_modules();
                }
                return Err(EngineError::Custom("wf-recorder falló al iniciar la grabación continua. Revisa ~/.config/sane-rec/crash.log".into()));
            }
        }

        *self.continuous_process.lock().unwrap() = Some(child);
        *self.continuous_output.lock().unwrap() = Some(output_path.clone());
        *self.continuous_start_time.lock().unwrap() = Some(Instant::now());

        let _ = notify_rust::Notification::new()
            .appname("sane-rec")
            .summary("🔴 Grabación Iniciada")
            .body(&format!("Grabando pantalla en {}", output_path.display()))
            .icon("media-record")
            .show();

        play_feedback_sound("message-new-instant");

        Ok(output_path)
    }

    /// Detiene la grabación continua enviando SIGINT en Linux.
    #[cfg(target_os = "linux")]
    pub fn stop_continuous_recording(&self) -> Result<PathBuf> {
        let mut child = self.continuous_process.lock().unwrap().take().ok_or(EngineError::NotRunning)?;
        let output_path = self
            .continuous_output
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| PathBuf::from("recording.mp4"));
        let elapsed = self.continuous_start_time.lock().unwrap().take().map(|t| t.elapsed());

        unsafe {
            libc::kill(child.id() as i32, libc::SIGINT);
        }
        let _ = child.wait();

        // Descarga de módulos de audio virtual PipeWire únicamente si no hay búfer de replay activo
        if !self.is_running() {
            self.cleanup_audio_modules();
        }

        let _ = notify_rust::Notification::new()
            .appname("sane-rec")
            .summary("⏹️ Grabación Finalizada")
            .body(&format!("Vídeo guardado en {}", output_path.display()))
            .icon("video-x-generic")
            .show();

        play_feedback_sound("complete");

        let current_cfg = Config::load().unwrap_or_else(|_| self.config());
        if current_cfg.storage.benchmark_mode {
            let mut cmd_probe = Command::new("wf-recorder");
            let fps = if current_cfg.video.fps == 0 { 60 } else { current_cfg.video.fps };
            cmd_probe.arg("-y").args(["-r", &fps.to_string()]);
            apply_target_args(&mut cmd_probe, &current_cfg.video.target);
            self.apply_audio_args(&mut cmd_probe, &current_cfg);
            apply_profile_args(&mut cmd_probe, &current_cfg);
            cmd_probe.arg("-f").arg(&output_path);
            let executed_cmd = format!(
                "wf-recorder {}",
                cmd_probe
                    .get_args()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join(" ")
            );

            let stderr_log = std::fs::read(crate::crash_handler::get_engine_log_path())
                .map(|bytes| {
                    let start = bytes.len().saturating_sub(8192);
                    String::from_utf8_lossy(&bytes[start..]).into_owned()
                })
                .unwrap_or_default();
            let ctx = crate::benchmark::BenchmarkContext {
                output_path: &output_path,
                config: &current_cfg,
                encoder_stderr: if stderr_log.is_empty() { None } else { Some(&stderr_log) },
                executed_command: Some(&executed_cmd),
                capture_backend: "Wayland / PipeWire (wf-recorder)",
                is_replay_clip: false,
                elapsed_duration: elapsed,
                ram_buffer_stats: None,
            };
            if let Err(e) = crate::benchmark::generate_benchmark_report(&ctx) {
                eprintln!("Aviso: No se pudo generar reporte de benchmark: {}", e);
            }
        }

        let is_valid = output_path.metadata().map(|m| m.len() >= 1024).unwrap_or(false);
        if !is_valid {
            let _ = std::fs::remove_file(&output_path);
        }

        Ok(output_path)
    }

    /// Inicia la grabación continua directa en Windows con ffmpeg.
    #[cfg(target_os = "windows")]
    pub fn start_continuous_recording(&self) -> Result<PathBuf> {
        let _start_guard = self.start_lock.lock().unwrap();
        if self.is_recording() {
            return Err(EngineError::AlreadyRunning);
        }

        let config = self.config();
        config.ensure_output_dir().map_err(|e| {
            EngineError::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
        })?;

        let timestamp = chrono::Local::now()
            .format("Recording_%Y-%m-%d_%H-%M-%S")
            .to_string();
        let filename = format!("{}.{}", timestamp, config.storage.format);
        let output_path = config.storage.output_dir.join(filename);

        let active_profile = config
            .profile
            .active_profile
            .parse::<EncoderProfile>()
            .unwrap_or(EncoderProfile::SaneSafe);

        let (mut audio_port, mut audio_stop) = match start_windows_audio_feeder(&config, &self.win_gain_system, &self.win_gain_mic) {
            Ok((port, stop)) => (Some(port), Some(stop)),
            Err(e) => {
                eprintln!("Aviso: No se pudo iniciar el feeder de audio WASAPI: {}", e);
                (None, None)
            }
        };

        let build_cont_cmd = |cfg: &Config, port: Option<u16>, out: &Path, backend: WindowsCaptureBackend| -> (Command, WindowsCaptureBackend) {
            let mut cmd = new_hidden_command(resolve_binary("ffmpeg"));
            cmd.args(["-y", "-stats_period", "5"]);

            let actual_backend = apply_windows_input_args(&mut cmd, cfg, port, backend);
            apply_windows_profile_args(&mut cmd, cfg, port.is_some(), actual_backend);

            if out.extension().map_or(false, |ext| ext == "mp4") {
                let (_, _, is_hevc) = resolve_windows_encoder(cfg);
                if is_hevc {
                    cmd.args(["-tag:v", "hvc1"]);
                }
                cmd.args(["-movflags", "+faststart"]);
            }
            cmd.arg(out);

            cmd.stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped());
            (cmd, actual_backend)
        };

        *self.continuous_stderr_log.lock().unwrap() = String::new();
        let (mut cmd, actual_backend) = build_cont_cmd(&config, audio_port, &output_path, WindowsCaptureBackend::DdaGrab);
        *self.win_continuous_backend.lock().unwrap() = actual_backend;
        *self.win_continuous_executed_cmd.lock().unwrap() = Some(render_ffmpeg_command(&cmd));
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                if let Some(stop) = audio_stop.take() {
                    stop.store(true, Ordering::SeqCst);
                }
                return Err(e.into());
            }
        };

        // Captura stderr para telemetría y crash reports (no descartamos errores del encoder)
        if let Some(mut err_pipe) = child.stderr.take() {
            let log_clone = Arc::clone(&self.continuous_stderr_log);
            std::thread::Builder::new()
                .name("sane-rec-cont-stderr".into())
                .spawn(move || {
                    use std::io::Read;
                    let mut buf = [0u8; 4096];
                    let mut ring = Vec::new();
                    while let Ok(n) = err_pipe.read(&mut buf) {
                        if n == 0 {
                            break;
                        }
                        ring.extend_from_slice(&buf[..n]);
                        if ring.len() > 16384 {
                            let drain_len = ring.len() - 16384;
                            ring.drain(..drain_len);
                        }
                        if let Ok(mut guard) = log_clone.lock() {
                            *guard = String::from_utf8_lossy(&ring).into_owned();
                        }
                    }
                })
                .ok();
        }

        // Verificación de salud inicial (600ms) y fallback automático a SaneSafe CPU con GDI
        std::thread::sleep(Duration::from_millis(600));
        let failed = match child.try_wait() {
            Ok(Some(_)) => true,
            Err(_) => true,
            Ok(None) => false,
        };

        if failed {
            let code = child.try_wait().ok().flatten().and_then(|s| s.code());
            crate::crash_handler::log_engine_failure("Inicio de Grabación Continua (Windows)", code);
            let err_text = self.continuous_stderr_log.lock().map(|g| g.clone()).unwrap_or_default();
            if !err_text.is_empty() {
                crate::crash_handler::write_crash_report(
                    "Fallo al iniciar grabación continua en Windows (FFmpeg stderr)",
                    &err_text,
                );
            }

            if let Some(stop) = audio_stop.take() {
                stop.store(true, Ordering::SeqCst);
            }

            if active_profile != EncoderProfile::SaneSafe
                || config.video.target.starts_with("window:")
                || config.video.target.starts_with("title=")
            {
                // Fallback no destructivo: no se guarda ni se sobrescribe la configuración del usuario
                let mut fallback_config = config.clone();
                fallback_config.profile.active_profile = "sane-safe".to_string();
                fallback_config.video.codec = "libx264".to_string();
                if fallback_config.video.target.starts_with("window:")
                    || fallback_config.video.target.starts_with("title=")
                {
                    fallback_config.video.target = "desktop".to_string();
                }

                let (retry_port, retry_stop) = match start_windows_audio_feeder(&fallback_config, &self.win_gain_system, &self.win_gain_mic) {
                    Ok((port, stop)) => (Some(port), Some(stop)),
                    Err(_) => (None, None),
                };
                audio_port = retry_port;
                audio_stop = retry_stop;

                let (mut fallback_cmd, fallback_backend) = build_cont_cmd(&fallback_config, audio_port, &output_path, WindowsCaptureBackend::GdiGrab);
                *self.win_continuous_backend.lock().unwrap() = fallback_backend;
                *self.win_continuous_executed_cmd.lock().unwrap() = Some(render_ffmpeg_command(&fallback_cmd));
                child = match fallback_cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                if let Some(stop) = audio_stop.take() {
                    stop.store(true, Ordering::SeqCst);
                }
                return Err(e.into());
            }
        };

                if let Some(mut err_pipe) = child.stderr.take() {
                    let log_clone = Arc::clone(&self.continuous_stderr_log);
                    std::thread::Builder::new()
                        .name("sane-rec-cont-fallback-stderr".into())
                        .spawn(move || {
                            use std::io::Read;
                            let mut buf = [0u8; 4096];
                            let mut ring = Vec::new();
                            while let Ok(n) = err_pipe.read(&mut buf) {
                                if n == 0 {
                                    break;
                                }
                                ring.extend_from_slice(&buf[..n]);
                                if ring.len() > 16384 {
                                    let drain_len = ring.len() - 16384;
                                    ring.drain(..drain_len);
                                }
                                if let Ok(mut guard) = log_clone.lock() {
                                    *guard = String::from_utf8_lossy(&ring).into_owned();
                                }
                            }
                        })
                        .ok();
                }

                std::thread::sleep(Duration::from_millis(600));
                if let Ok(Some(status)) = child.try_wait() {
                    if let Some(stop) = audio_stop.take() {
                        stop.store(true, Ordering::SeqCst);
                    }
                    return Err(EngineError::Custom(format!(
                        "ffmpeg falló al iniciar la grabación continua (código {:?})",
                        status.code()
                    )));
                }
            } else {
                return Err(EngineError::Custom(
                    "ffmpeg falló al iniciar la grabación continua en Windows.".into(),
                ));
            }
        }

        *self.win_continuous_audio_stop.lock().unwrap() = audio_stop;
        *self.continuous_process.lock().unwrap() = Some(child);
        *self.continuous_output.lock().unwrap() = Some(output_path.clone());
        *self.continuous_start_time.lock().unwrap() = Some(Instant::now());

        #[cfg(not(target_os = "windows"))]
        let _ = notify_rust::Notification::new()
            .appname("sane-rec")
            .summary("🔴 Grabación Iniciada")
            .body(&format!("Grabando pantalla en {}", output_path.display()))
            .icon("media-record")
            .show();

        play_feedback_sound("message-new-instant");
        Ok(output_path)
    }

    /// Detiene la grabación continua en Windows enviando 'q' a stdin de ffmpeg y finalizando el archivo MP4 limpiamente.
    #[cfg(target_os = "windows")]
    pub fn stop_continuous_recording(&self) -> Result<PathBuf> {
        if let Some(stop_flag) = self.win_continuous_audio_stop.lock().unwrap().take() {
            stop_flag.store(true, Ordering::SeqCst);
        }
        let mut child = self.continuous_process.lock().unwrap().take().ok_or(EngineError::NotRunning)?;
        let output_path = self
            .continuous_output
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| PathBuf::from("recording.mp4"));
        let elapsed = self.continuous_start_time.lock().unwrap().take().map(|t| t.elapsed());

        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(b"q\n");
            let _ = stdin.flush();
        }

        let mut exited = false;
        for _ in 0..30 {
            match child.try_wait() {
                Ok(Some(_)) => {
                    exited = true;
                    break;
                }
                _ => std::thread::sleep(Duration::from_millis(100)),
            }
        }
        if !exited {
            let _ = child.kill();
            let _ = child.wait();
        }

        let _ = notify_rust::Notification::new()
            .appname("sane-rec")
            .summary("⏹️ Grabación Finalizada")
            .body(&format!("Vídeo guardado en {}", output_path.display()))
            .icon("video-x-generic")
            .show();

        play_feedback_sound("complete");

        let current_cfg = Config::load().unwrap_or_else(|_| self.config());
        if current_cfg.storage.benchmark_mode {
            let stderr_str = self.continuous_stderr_log.lock().map(|g| g.clone()).unwrap_or_default();
            let backend = *self.win_continuous_backend.lock().unwrap();
            let backend_name = match backend {
                WindowsCaptureBackend::DdaGrab => "Desktop Duplication API (ddagrab / DXGI)",
                WindowsCaptureBackend::GdiGrab => "GDI Screen Grabber (gdigrab)",
            };
            let executed_cmd = self
                .win_continuous_executed_cmd
                .lock()
                .unwrap()
                .take()
                .unwrap_or_default();
            let ctx = crate::benchmark::BenchmarkContext {
                output_path: &output_path,
                config: &current_cfg,
                encoder_stderr: if stderr_str.is_empty() { None } else { Some(&stderr_str) },
                executed_command: if executed_cmd.is_empty() {
                    None
                } else {
                    Some(&executed_cmd)
                },
                capture_backend: backend_name,
                is_replay_clip: false,
                elapsed_duration: elapsed,
                ram_buffer_stats: None,
            };
            if let Err(e) = crate::benchmark::generate_benchmark_report(&ctx) {
                eprintln!("Aviso: No se pudo generar reporte de benchmark: {}", e);
            }
        }

        Ok(output_path)
    }

    /// Alterna el estado de la grabación continua.
    pub fn toggle_continuous_recording(&self) -> Result<bool> {
        if self.is_recording() {
            if let Some(start_time) = *self.continuous_start_time.lock().unwrap() {
                if start_time.elapsed() < std::time::Duration::from_millis(1200) {
                    return Ok(true);
                }
            }
            self.stop_continuous_recording()?;
            Ok(false)
        } else {
            self.start_continuous_recording()?;
            Ok(true)
        }
    }

    /// Indica si la grabación continua se encuentra viva y activa.
    pub fn is_recording(&self) -> bool {
        let mut guard = self.continuous_process.lock().unwrap();
        if let Some(ref mut child) = *guard {
            match child.try_wait() {
                Ok(None) => true,
                Ok(Some(status)) => {
                    if !status.success() {
                        crate::crash_handler::log_engine_failure(
                            "Grabación Continua en Segundo Plano",
                            status.code(),
                        );
                    }
                    *guard = None;
                    false
                }
                Err(_) => {
                    *guard = None;
                    false
                }
            }
        } else {
            false
        }
    }

    /// Guarda un replay del búfer actual concatenando segmentos en RAM a alta velocidad con ffmpeg en Linux.
    #[cfg(target_os = "linux")]
    pub fn save_clip(&self) -> Result<PathBuf> {
        // 1. Escanea los segmentos seg_* (ts, mp4, mkv) en el directorio de búfer con tamaño > 0
        let mut segments = self.scan_segments()?;
        if segments.is_empty() {
            return Err(EngineError::NoSegments);
        }

        // Orden cronológico por tiempo de modificación
        segments.sort_by(|a, b| {
            let mod_a = a.metadata().and_then(|m| m.modified()).ok();
            let mod_b = b.metadata().and_then(|m| m.modified()).ok();
            match (mod_a, mod_b) {
                (Some(ta), Some(tb)) => ta.cmp(&tb),
                _ => a.cmp(b),
            }
        });

        self.is_saving.store(true, Ordering::SeqCst);

        let duration_secs = self.config().replay.duration_secs;

        // 2. Prepara rutas de archivos únicos
        let unique_id = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let list_file = self.buffer_dir.join(format!("concat_list_{}.txt", unique_id));
        let staging_file = self
            .buffer_dir
            .join(format!(".staging_concat_{}.{}", unique_id, self.config().storage.format));

        // 3. Determina el destino final
        let config = self.config();
        if let Err(e) = config.ensure_output_dir() {
            self.is_saving.store(false, Ordering::SeqCst);
            return Err(EngineError::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())));
        }

        let timestamp = chrono::Local::now()
            .format(&config.storage.filename_format)
            .to_string();
        let filename = format!("{}.{}", timestamp, config.storage.format);
        let output_path = config.storage.output_dir.join(filename);

        // 4. Concatena los segmentos en RAM hacia el archivo staging con recuperación automática
        let run_concat = |segs: &[PathBuf]| -> std::result::Result<(), String> {
            let mut list_content = String::new();
            for seg in segs {
                let seg_str = seg.to_string_lossy().replace('\\', "/");
                list_content.push_str(&format!("file '{}'\n", seg_str));
            }
            if let Err(e) = std::fs::write(&list_file, list_content) {
                return Err(format!("Error al escribir lista concat: {}", e));
            }

            // Primer intento con -bsf:a aac_adtstoasc (requerido para empaquetar AAC de MPEG-TS en MP4)
            let res = new_hidden_command(resolve_binary("ffmpeg"))
                .args(["-y", "-f", "concat", "-safe", "0", "-i"])
                .arg(&list_file)
                .args(["-c", "copy", "-bsf:a", "aac_adtstoasc", "-movflags", "+faststart"])
                .arg(&staging_file)
                .output();

            let success = match res {
                Ok(ref o) if o.status.success() => {
                    std::fs::metadata(&staging_file).map(|m| m.len() > 0).unwrap_or(false)
                }
                _ => false,
            };

            if success {
                return Ok(());
            }

            // Segundo intento sin -bsf:a (por si el formato ya está paquetizado o no es AAC)
            let res2 = new_hidden_command(resolve_binary("ffmpeg"))
                .args(["-y", "-f", "concat", "-safe", "0", "-i"])
                .arg(&list_file)
                .args(["-c", "copy", "-movflags", "+faststart"])
                .arg(&staging_file)
                .output();

            let success2 = match res2 {
                Ok(ref o) if o.status.success() => {
                    std::fs::metadata(&staging_file).map(|m| m.len() > 0).unwrap_or(false)
                }
                _ => false,
            };

            if success2 {
                return Ok(());
            }

            let err_msg = res2
                .map(|o| String::from_utf8_lossy(&o.stderr).into_owned())
                .or_else(|_| res.map(|o| String::from_utf8_lossy(&o.stderr).into_owned()))
                .unwrap_or_default();

            Err(err_msg)
        };

        let mut selected_segments = segments.clone();
        let concat_res = match run_concat(&selected_segments) {
            Ok(()) => Ok(()),
            Err(err_msg) => {
                // Si la concatenación falló (ej. 'moov atom not found' por el último segmento MP4
                // en el que wf-recorder sigue escribiendo sin haber cerrado el contenedor) y hay más de un segmento,
                // reintentamos automáticamente excluyendo el último segmento abierto.
                if selected_segments.len() > 1 {
                    eprintln!(
                        "Aviso: Falló la concatenación inicial con todos los segmentos ({}); reintentando excluyendo el último segmento en escritura activa...",
                        err_msg
                    );
                    selected_segments.pop();
                    run_concat(&selected_segments)
                } else {
                    Err(err_msg)
                }
            }
        };

        let _ = std::fs::remove_file(&list_file);

        if let Err(err_msg) = concat_res {
            let _ = std::fs::remove_file(&staging_file);
            self.is_saving.store(false, Ordering::SeqCst);
            crate::crash_handler::write_crash_report(
                "Error de concatenación en ffmpeg al guardar replay",
                &err_msg,
            );
            return Err(EngineError::FfmpegError(format!(
                "ffmpeg concat error: {}",
                err_msg
            )));
        }

        // 5. Recorta de forma exacta los últimos duration_secs usando -sseof (0 pérdida, ultra-rápido)
        let trim_arg = format!("-{}", duration_secs);
        let trim_res = new_hidden_command(resolve_binary("ffmpeg"))
            .args(["-y", "-sseof", &trim_arg, "-i"])
            .arg(&staging_file)
            .args(["-c", "copy", "-movflags", "+faststart"])
            .arg(&output_path)
            .output();

        let _ = std::fs::remove_file(&staging_file);
        self.is_saving.store(false, Ordering::SeqCst);

        let trim_out = match trim_res {
            Ok(o) => o,
            Err(e) => {
                return Err(EngineError::Io(e));
            }
        };

        if !trim_out.status.success() {
            let err_msg = String::from_utf8_lossy(&trim_out.stderr);
            crate::crash_handler::write_crash_report("Error de recorte (-sseof) en ffmpeg al guardar replay", &err_msg);
            return Err(EngineError::FfmpegError(format!(
                "ffmpeg trim error: {}",
                err_msg
            )));
        }

        // 5. Lanza la notificación de escritorio y reproduce sonido de confirmación
        let _ = notify_rust::Notification::new()
            .appname("sane-rec")
            .summary("🎬 Replay Guardado")
            .body(&format!("Clip guardado en {}", output_path.display()))
            .icon("video-x-generic")
            .show();

        play_feedback_sound("screen-capture");

        // Si el modo benchmark está activo, genera el informe exhaustivo .benchmark.txt
        let current_cfg = Config::load().unwrap_or_else(|_| self.config());
        if current_cfg.storage.benchmark_mode {
            let ctx = crate::benchmark::BenchmarkContext {
                output_path: &output_path,
                config: &current_cfg,
                encoder_stderr: None,
                executed_command: None,
                capture_backend: "Wayland / PipeWire (RAM Ring Buffer)",
                is_replay_clip: true,
                elapsed_duration: Some(Duration::from_secs(current_cfg.replay.duration_secs as u64)),
                ram_buffer_stats: None,
            };
            if let Err(e) = crate::benchmark::generate_benchmark_report(&ctx) {
                eprintln!("Aviso: No se pudo generar reporte de benchmark: {}", e);
            }
        }

        // Si el modo "doble clip" está activo, genera en segundo plano la copia optimizada para Discord
        if current_cfg.storage.dual_clip {
            let target_mb = current_cfg.storage.discord_target_mb;
            crate::compressor::spawn_discord_compression(output_path.clone(), target_mb);
        }

        Ok(output_path)
    }

    /// Guarda un replay del búfer actual tomando los chunks en RAM física y escribiéndolos instantáneamente a MP4 sin recodificar en Windows.
    #[cfg(target_os = "windows")]
    pub fn save_clip(&self) -> Result<PathBuf> {
        let (chunks, ram_stats) = {
            let q = self.win_ram_buffer.lock().unwrap();
            if q.is_empty() {
                return Err(EngineError::NoSegments);
            }
            let dur = self.config().replay.duration_secs;
            let now = Instant::now();
            let cutoff = Duration::from_secs((dur + 1) as u64);
            let list: Vec<Arc<Vec<u8>>> = q
                .iter()
                .filter(|(t, _)| now.duration_since(*t) <= cutoff)
                .map(|(_, bytes)| Arc::clone(bytes))
                .collect();

            let total_bytes: u64 = list.iter().map(|c| c.len() as u64).sum();
            let oldest = q.front().map(|(t, _)| {
                let elapsed = now.saturating_duration_since(*t);
                std::time::SystemTime::now().checked_sub(elapsed).unwrap_or(std::time::SystemTime::now())
            });
            let newest = q.back().map(|(t, _)| {
                let elapsed = now.saturating_duration_since(*t);
                std::time::SystemTime::now().checked_sub(elapsed).unwrap_or(std::time::SystemTime::now())
            });
            let stats = crate::benchmark::RamBufferStats {
                segments: list.len(),
                bytes: total_bytes,
                oldest,
                newest,
                truncated: false,
            };
            (list, stats)
        };

        if chunks.is_empty() {
            return Err(EngineError::NoSegments);
        }
        if !chunks.iter().any(|c| chunk_has_video_pusi(c)) {
            return Err(EngineError::Custom(
                "El búfer en RAM no contiene vídeo válido".into(),
            ));
        }

        self.is_saving.store(true, Ordering::SeqCst);

        let config = self.config();
        if let Err(e) = config.ensure_output_dir() {
            self.is_saving.store(false, Ordering::SeqCst);
            return Err(EngineError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                e.to_string(),
            )));
        }

        let timestamp = chrono::Local::now()
            .format(&config.storage.filename_format)
            .to_string();
        let filename = format!("{}.{}", timestamp, config.storage.format);
        let output_path = config.storage.output_dir.join(filename);

        // Localizar inicio limpio del stream MPEG-TS sin copiar buffers:
        // Buscando el primer paquete donde packet[0] == 0x47 y (packet[1] & 0x40) != 0 (PUSI),
        // dando prioridad a PID 0x0000 (PAT).
        let mut start_chunk = 0;
        let mut start_offset = 0;
        let mut found_pat = false;
        let mut first_pusi: Option<(usize, usize)> = None;

        'find_start: for (c_idx, chunk) in chunks.iter().enumerate() {
            let total_pkts = chunk.len() / 188;
            for p in 0..total_pkts {
                let offset = p * 188;
                let pkt = &chunk[offset..offset + 188];
                if pkt[0] == 0x47 && (pkt[1] & 0x40) != 0 {
                    let pid = (((pkt[1] & 0x1F) as u16) << 8) | (pkt[2] as u16);
                    if pid == 0 {
                        start_chunk = c_idx;
                        start_offset = offset;
                        found_pat = true;
                        break 'find_start;
                    } else if first_pusi.is_none() {
                        first_pusi = Some((c_idx, offset));
                    }
                }
            }
        }

        if !found_pat {
            if let Some((c_idx, offset)) = first_pusi {
                start_chunk = c_idx;
                start_offset = offset;
            }
        }

        let stream_chunks = Arc::new(chunks);
        let stream_to_pipe = |mut stdin: std::process::ChildStdin,
                              data: Arc<Vec<Arc<Vec<u8>>>>,
                              s_chunk: usize,
                              s_offset: usize| {
            std::thread::spawn(move || {
                use std::io::Write;
                for (i, chunk) in data.iter().enumerate().skip(s_chunk) {
                    let slice = if i == s_chunk {
                        &chunk[s_offset..]
                    } else {
                        &chunk[..]
                    };
                    if stdin.write_all(slice).is_err() {
                        break;
                    }
                }
                let _ = stdin.flush();
            });
        };

        // 1. Remux primario directo con ffmpeg leyendo desde pipe:0 en RAM por streaming de chunks
        let mut remux_cmd = new_hidden_command(resolve_binary("ffmpeg"));
        remux_cmd.args([
            "-y",
            "-avoid_negative_ts",
            "make_zero",
            "-fflags",
            "+genpts",
            "-f",
            "mpegts",
            "-i",
            "pipe:0",
            "-c:v",
            "copy",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-af",
            "aresample=async=1:first_pts=0",
            "-movflags",
            "+faststart",
        ]);
        let (_, _, is_hevc) = resolve_windows_encoder(&config);
        if is_hevc && output_path.extension().map_or(false, |ext| ext == "mp4") {
            remux_cmd.args(["-tag:v", "hvc1"]);
        }
        remux_cmd.arg(&output_path);
        remux_cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let copy_succeeded = match remux_cmd.spawn() {
            Ok(mut child) => {
                if let Some(stdin) = child.stdin.take() {
                    stream_to_pipe(stdin, Arc::clone(&stream_chunks), start_chunk, start_offset);
                }
                match child.wait_with_output() {
                    Ok(out) => {
                        let stderr_text = String::from_utf8_lossy(&out.stderr);
                        out.status.success()
                            && std::fs::metadata(&output_path)
                                .map(|m| m.len() >= 1024)
                                .unwrap_or(false)
                            && !stderr_text.contains("video:0kB")
                            && !stderr_text.contains("video:0KiB")
                    }
                    Err(_) => false,
                }
            }
            Err(_) => false,
        };

        // 2. Fallback de transcodificación de rescate alimentando pipe:0 en RAM si el remux directo falla o pesa < 1024 bytes
        if !copy_succeeded {
            eprintln!("Aviso: Remux directo en pipe devolvió error o archivo <1KB; ejecutando transcodificación de rescate.");
            let mut transcode_cmd = new_hidden_command(resolve_binary("ffmpeg"));
            transcode_cmd.args([
                "-y",
                "-avoid_negative_ts",
                "make_zero",
                "-fflags",
                "+genpts",
                "-f",
                "mpegts",
                "-i",
                "pipe:0",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-tune",
                "zerolatency",
                "-threads",
                "4",
                "-crf",
                "23",
                "-bf",
                "0",
                "-pix_fmt",
                "yuv420p",
                "-vf",
                "scale=trunc(iw/2)*2:trunc(ih/2)*2",
                "-c:a",
                "aac",
                "-b:a",
                "192k",
                "-movflags",
                "+faststart",
            ]);
            transcode_cmd.arg(&output_path);
            transcode_cmd
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped());

            let transcode_res = match transcode_cmd.spawn() {
                Ok(mut child) => {
                    if let Some(stdin) = child.stdin.take() {
                        stream_to_pipe(stdin, Arc::clone(&stream_chunks), start_chunk, start_offset);
                    }
                    child.wait_with_output()
                }
                Err(e) => {
                    self.is_saving.store(false, Ordering::SeqCst);
                    return Err(EngineError::Io(e));
                }
            };

            let transcode_succeeded = match transcode_res {
                Ok(ref out) if out.status.success() => {
                    std::fs::metadata(&output_path)
                        .map(|m| m.len() >= 1024)
                        .unwrap_or(false)
                }
                _ => false,
            };

            if !transcode_succeeded {
                self.is_saving.store(false, Ordering::SeqCst);
                let err_detail = transcode_res
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stderr).into_owned())
                    .unwrap_or_default();
                crate::crash_handler::write_crash_report(
                    "Fallo al transcodificar clip de rescate en Windows",
                    &err_detail,
                );
                return Err(EngineError::FfmpegError(
                    "Error al guardar MP4 desde el buffer MPEG-TS en RAM".into(),
                ));
            }
        }

        self.is_saving.store(false, Ordering::SeqCst);

        #[cfg(not(target_os = "windows"))]
        let _ = notify_rust::Notification::new()
            .appname("sane-rec")
            .summary("🎬 Replay Guardado")
            .body(&format!("Clip guardado en {}", output_path.display()))
            .icon("video-x-generic")
            .show();

        play_feedback_sound("screen-capture");

        let current_cfg = Config::load().unwrap_or_else(|_| self.config());
        if current_cfg.storage.benchmark_mode {
            let backend_name = match *self.win_replay_backend.lock().unwrap() {
                WindowsCaptureBackend::DdaGrab => "Desktop Duplication API (ddagrab / DXGI) -> RAM Ring Buffer",
                WindowsCaptureBackend::GdiGrab => "GDI Screen Grabber (gdigrab) -> RAM Ring Buffer",
            };
            let stderr_str = self.win_replay_stderr_log.lock().map(|g| g.clone()).unwrap_or_default();
            let executed_cmd = self
                .win_replay_executed_cmd
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_default();
            let ctx = crate::benchmark::BenchmarkContext {
                output_path: &output_path,
                config: &current_cfg,
                encoder_stderr: if stderr_str.is_empty() { None } else { Some(&stderr_str) },
                executed_command: if executed_cmd.is_empty() {
                    None
                } else {
                    Some(&executed_cmd)
                },
                capture_backend: backend_name,
                is_replay_clip: true,
                elapsed_duration: Some(Duration::from_secs(current_cfg.replay.duration_secs as u64)),
                ram_buffer_stats: Some(ram_stats),
            };
            if let Err(e) = crate::benchmark::generate_benchmark_report(&ctx) {
                eprintln!("Aviso: No se pudo generar reporte de benchmark: {}", e);
            }
        }

        if current_cfg.storage.dual_clip {
            let target_mb = current_cfg.storage.discord_target_mb;
            crate::compressor::spawn_discord_compression(output_path.clone(), target_mb);
        }

        Ok(output_path)
    }

    /// Retorna el estado actual del motor de grabación y memoria en Linux.
    #[cfg(target_os = "linux")]
    pub fn get_status(&self) -> EngineStatus {
        let is_running = self.is_running();
        let is_recording = self.is_recording();
        let config = self.config();

        let (current_buffer_secs, ram_usage_mb) = if is_running {
            let segments = self.scan_segments().unwrap_or_default();
            let secs = (segments.len() as u32 * DEFAULT_SEGMENT_DURATION_SECS)
                .min(config.replay.duration_secs);

            let mut total_bytes: u64 = 0;
            for seg in &segments {
                if let Ok(meta) = seg.metadata() {
                    total_bytes += meta.len();
                }
            }
            let mb = (total_bytes as f64 / (1024.0 * 1024.0) * 10.0).round() / 10.0;
            (secs, mb)
        } else {
            (0, 0.0)
        };

        let recording_elapsed_secs = if is_recording {
            self.continuous_start_time
                .lock()
                .unwrap()
                .map(|t| t.elapsed().as_secs() as u32)
                .unwrap_or(0)
        } else {
            0
        };

        EngineStatus {
            is_running,
            is_recording,
            duration_secs: config.replay.duration_secs,
            current_buffer_secs,
            recording_elapsed_secs,
            ram_usage_mb,
            max_memory_mb: config.replay.max_memory_mb,
            active_profile: config.profile.active_profile,
            output_dir: config.storage.output_dir,
        }
    }

    /// Retorna el estado actual del motor de grabación y memoria en Windows.
    #[cfg(target_os = "windows")]
    pub fn get_status(&self) -> EngineStatus {
        let is_running = self.is_running();
        let is_recording = self.is_recording();
        let config = self.config();

        let (current_buffer_secs, ram_usage_mb) = if is_running {
            let q = self.win_ram_buffer.lock().unwrap();
            let total_bytes: usize = q.iter().map(|(_, c)| c.len()).sum();
            let mb = (total_bytes as f64 / (1024.0 * 1024.0) * 10.0).round() / 10.0;
            let secs = if let (Some(first), Some(last)) = (q.front(), q.back()) {
                last.0.duration_since(first.0).as_secs() as u32
            } else {
                0
            };
            (secs.min(config.replay.duration_secs), mb)
        } else {
            (0, 0.0)
        };

        let recording_elapsed_secs = if is_recording {
            self.continuous_start_time
                .lock()
                .unwrap()
                .map(|t| t.elapsed().as_secs() as u32)
                .unwrap_or(0)
        } else {
            0
        };

        EngineStatus {
            is_running,
            is_recording,
            duration_secs: config.replay.duration_secs,
            current_buffer_secs,
            recording_elapsed_secs,
            ram_usage_mb,
            max_memory_mb: config.replay.max_memory_mb,
            active_profile: config.profile.active_profile,
            output_dir: config.storage.output_dir,
        }
    }

    /// Escanea los segmentos válidos presentes en la RAM (seg_* o segment_* con extensión .ts, .mp4 o .mkv y tamaño > 0).
    pub fn scan_segments(&self) -> Result<Vec<PathBuf>> {
        let mut segments = Vec::new();

        if self.buffer_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&self.buffer_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        let name = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or_default();
                        let ext = path
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or_default();

                        if (name.starts_with("seg_") || name.starts_with("segment_"))
                            && (ext == "ts" || ext == "mp4" || ext == "mkv")
                            && !name.contains('%')
                        {
                            if let Ok(meta) = entry.metadata() {
                                if meta.len() > 0 {
                                    segments.push(path);
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(segments)
    }

    /// Limpia todos los segmentos rodantes y temporales en RAM.
    pub fn clean_buffer(&self) -> Result<()> {
        Self::clean_directory(&self.buffer_dir);
        Ok(())
    }

    fn clean_directory(dir: &Path) {
        if !dir.exists() {
            return;
        }
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or_default();
                    let ext = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or_default();
                    if name.starts_with("seg_")
                        || name.starts_with("segment_")
                        || name.starts_with("concat_")
                        || name.starts_with(".staging_")
                        || ext == "ts"
                        || ext == "mp4"
                        || ext == "mkv"
                        || name == "concat_list.txt"
                        || name == "list.txt"
                    {
                        let _ = std::fs::remove_file(path);
                    }
                }
            }
        }
    }
}

impl Drop for ReplayEngine {
    fn drop(&mut self) {
        if self.is_recording() {
            let _ = self.stop_continuous_recording();
        }
        if self.is_running() {
            let _ = self.stop_replay();
        }

        self.pruner_running.store(false, Ordering::SeqCst);
        self.replay_session.fetch_add(1, Ordering::SeqCst);
        self.win_replay_session.fetch_add(1, Ordering::SeqCst);

        #[cfg(target_os = "windows")]
        {
            if let Ok(mut lock) = self.win_continuous_audio_stop.lock() {
                if let Some(stop) = lock.take() {
                    stop.store(true, Ordering::SeqCst);
                }
            }
            if let Ok(mut lock) = self.win_replay_audio_stop.lock() {
                if let Some(stop) = lock.take() {
                    stop.store(true, Ordering::SeqCst);
                }
            }
        }

        // Asegurar que cualquier Child restante en continuous_process reciba kill() y wait()
        if let Ok(mut guard) = self.continuous_process.lock() {
            if let Some(mut child) = guard.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }

        // Asegurar que cualquier Child restante en replay_process reciba kill() y wait()
        if let Ok(mut guard) = self.replay_process.lock() {
            if let Some(mut child) = guard.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }

        #[cfg(target_os = "linux")]
        {
            self.cleanup_audio_modules();
            let _ = self.clean_buffer();
        }
    }
}

/// Resampler lineal y conversor de canales / formatos para mezcla de audio WASAPI en Windows.
pub struct WindowsAudioResampler {
    from_rate: f64,
    to_rate: f64,
    channels: usize,
    volume: f32,
    ratio: f64,
    pos: f64,
    prev_left: f32,
    prev_right: f32,
    curr_channel: usize,
    curr_frame_left: f32,
    curr_frame_right: f32,
    staging: Vec<f32>,
}

impl WindowsAudioResampler {
    pub fn new(sample_rate: u32, channels: usize, volume: f32) -> Self {
        let from_rate = sample_rate.max(1) as f64;
        let to_rate = 48000.0;
        let ratio = from_rate / to_rate;
        Self {
            from_rate,
            to_rate,
            channels: channels.max(1),
            volume,
            ratio,
            pos: 0.0,
            prev_left: 0.0,
            prev_right: 0.0,
            curr_channel: 0,
            curr_frame_left: 0.0,
            curr_frame_right: 0.0,
            staging: Vec::with_capacity(3840),
        }
    }

    pub fn push_sample_staged(&mut self, s: f32) {
        if self.curr_channel == 0 {
            self.curr_frame_left = s;
            if self.channels == 1 {
                self.curr_frame_right = s;
                self.feed_frame_staged();
            }
        } else if self.curr_channel == 1 {
            self.curr_frame_right = s;
            if self.channels == 2 {
                self.feed_frame_staged();
            }
        } else if self.curr_channel == self.channels - 1 {
            self.feed_frame_staged();
        }
        self.curr_channel = (self.curr_channel + 1) % self.channels;
    }

    pub fn push_sample(&mut self, s: f32, out: &Arc<Mutex<std::collections::VecDeque<f32>>>) {
        self.push_sample_staged(s);
        self.flush(out);
    }

    fn feed_frame_staged(&mut self) {
        let next_left = self.curr_frame_left;
        let next_right = self.curr_frame_right;

        if (self.from_rate - self.to_rate).abs() < 1.0 {
            self.staging.push(next_left * self.volume);
            self.staging.push(next_right * self.volume);
            return;
        }

        while self.pos < 1.0 {
            let t = self.pos as f32;
            let l = (1.0 - t) * self.prev_left + t * next_left;
            let r = (1.0 - t) * self.prev_right + t * next_right;
            self.staging.push(l * self.volume);
            self.staging.push(r * self.volume);
            self.pos += self.ratio;
        }
        self.pos -= 1.0;
        self.prev_left = next_left;
        self.prev_right = next_right;
    }

    pub fn flush(&mut self, out: &Arc<Mutex<std::collections::VecDeque<f32>>>) {
        if self.staging.is_empty() {
            return;
        }
        if let Ok(mut q) = out.lock() {
            q.extend(self.staging.drain(..));
            if q.len() > 96000 {
                let excess = q.len() - 48000;
                q.drain(..excess);
            }
        }
    }

    pub fn process_f32(&mut self, data: &[f32], out: &Arc<Mutex<std::collections::VecDeque<f32>>>) {
        for &s in data {
            self.push_sample_staged(s);
        }
        self.flush(out);
    }

    pub fn process_i16(&mut self, data: &[i16], out: &Arc<Mutex<std::collections::VecDeque<f32>>>) {
        for &s in data {
            let f = s as f32 / 32768.0;
            self.push_sample_staged(f);
        }
        self.flush(out);
    }

    pub fn process_u16(&mut self, data: &[u16], out: &Arc<Mutex<std::collections::VecDeque<f32>>>) {
        for &s in data {
            let f = (s as f32 - 32768.0) / 32768.0;
            self.push_sample_staged(f);
        }
        self.flush(out);
    }

    pub fn process_i32(&mut self, data: &[i32], out: &Arc<Mutex<std::collections::VecDeque<f32>>>) {
        for &s in data {
            self.push_sample_staged(s as f32 / 2147483648.0);
        }
        self.flush(out);
    }

    pub fn process_f64(&mut self, data: &[f64], out: &Arc<Mutex<std::collections::VecDeque<f32>>>) {
        for &s in data {
            self.push_sample_staged(s as f32);
        }
        self.flush(out);
    }
}

/// Decide si hay que reconstruir los streams de un endpoint de audio (desktop o mic).
#[cfg(any(target_os = "windows", test))]
fn audio_endpoint_needs_rebuild(
    enabled: bool,
    bound: Option<&(String, u32, u16)>,
    dead: bool,
    current: Option<&(String, u32, u16)>,
) -> bool {
    enabled && (dead || bound != current)
}

/// Streams WASAPI activos (loopback + mic) junto con los dispositivos a los que están ligados.
#[cfg(target_os = "windows")]
struct WinCapture {
    streams: Vec<cpal::Stream>,
    dead: Arc<AtomicBool>,
    /// 0 = loopback de escritorio, 1 = micrófono: (nombre, sample_rate, canales).
    keys: [Option<(String, u32, u16)>; 2],
}

#[cfg(target_os = "windows")]
impl WinCapture {
    fn key(device: &cpal::Device, output: bool) -> Option<(String, u32, u16)> {
        use cpal::traits::DeviceTrait;
        let res = if output {
            device.default_output_config()
        } else {
            device.default_input_config()
        };
        let sc = res.ok()?;
        Some((device.name().unwrap_or_default(), sc.sample_rate().0, sc.channels()))
    }

    fn open(
        device: &cpal::Device,
        sc: cpal::SupportedStreamConfig,
        buf: &Arc<Mutex<std::collections::VecDeque<f32>>>,
        dead: &Arc<AtomicBool>,
    ) -> Option<cpal::Stream> {
        use cpal::traits::{DeviceTrait, StreamTrait};
        let format = sc.sample_format();
        let resampler = Arc::new(Mutex::new(WindowsAudioResampler::new(
            sc.sample_rate().0,
            sc.channels() as usize,
            1.0, // la ganancia en vivo se aplica en la mezcla del feeder
        )));
        let cfg: cpal::StreamConfig = sc.into();

        macro_rules! build {
            ($t:ty, $m:ident) => {{
                let (r, b, d) = (Arc::clone(&resampler), Arc::clone(buf), Arc::clone(dead));
                device.build_input_stream(
                    &cfg,
                    move |data: &[$t], _| {
                        if let Ok(mut r) = r.lock() {
                            r.$m(data, &b);
                        }
                    },
                    move |err| {
                        eprintln!("Aviso en stream WASAPI: {}", err);
                        d.store(true, Ordering::Relaxed);
                    },
                    None,
                )
            }};
        }

        let res = match format {
            cpal::SampleFormat::F32 => build!(f32, process_f32),
            cpal::SampleFormat::I16 => build!(i16, process_i16),
            cpal::SampleFormat::U16 => build!(u16, process_u16),
            cpal::SampleFormat::I32 => build!(i32, process_i32),
            cpal::SampleFormat::F64 => build!(f64, process_f64),
            other => {
                eprintln!("Formato WASAPI no soportado: {:?}", other);
                return None;
            }
        };
        let stream = match res {
            Ok(s) => s,
            Err(e) => {
                eprintln!("No se pudo abrir stream WASAPI: {}", e);
                return None;
            }
        };
        stream.play().ok()?;
        Some(stream)
    }

    fn start(
        host: &cpal::Host,
        config: &Config,
        sys_buf: &Arc<Mutex<std::collections::VecDeque<f32>>>,
        mic_buf: &Arc<Mutex<std::collections::VecDeque<f32>>>,
    ) -> Self {
        use cpal::traits::{DeviceTrait, HostTrait};
        let dead = Arc::new(AtomicBool::new(false));
        let mut streams = Vec::new();
        let mut keys: [Option<(String, u32, u16)>; 2] = [None, None];

        if config.audio.capture_desktop {
            if let Some(dev) = host.default_output_device() {
                if let (Some(key), Ok(sc)) = (Self::key(&dev, true), dev.default_output_config()) {
                    if let Some(s) = Self::open(&dev, sc, sys_buf, &dead) {
                        streams.push(s);
                        keys[0] = Some(key);
                    }
                }
            }
        }
        if config.audio.capture_mic {
            if let Some(dev) = host.default_input_device() {
                if let (Some(key), Ok(sc)) = (Self::key(&dev, false), dev.default_input_config()) {
                    if let Some(s) = Self::open(&dev, sc, mic_buf, &dead) {
                        streams.push(s);
                        keys[1] = Some(key);
                    }
                }
            }
        }
        Self { streams, dead, keys }
    }

    /// `true` si algún dispositivo por defecto cambió, apareció/desapareció o su stream reportó error.
    fn is_stale(&self, host: &cpal::Host, cfg: &Config) -> bool {
        use cpal::traits::HostTrait;
        let dead = self.dead.load(Ordering::Relaxed);
        let desktop = if cfg.audio.capture_desktop {
            host.default_output_device().and_then(|d| Self::key(&d, true))
        } else {
            None
        };
        let mic = if cfg.audio.capture_mic {
            host.default_input_device().and_then(|d| Self::key(&d, false))
        } else {
            None
        };
        audio_endpoint_needs_rebuild(cfg.audio.capture_desktop, self.keys[0].as_ref(), dead, desktop.as_ref())
            || audio_endpoint_needs_rebuild(cfg.audio.capture_mic, self.keys[1].as_ref(), dead, mic.as_ref())
    }
}

/// Inicia el feeder de audio WASAPI Loopback y Micrófono con cpal transmitiendo PCM crudo estéreo a 48 kHz vía TCP local.
#[cfg(target_os = "windows")]
fn start_windows_audio_feeder(
    config: &Config,
    gain_system: &Arc<std::sync::atomic::AtomicU32>,
    gain_mic: &Arc<std::sync::atomic::AtomicU32>,
) -> std::io::Result<(u16, Arc<AtomicBool>)> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_flag_clone = Arc::clone(&stop_flag);

    let system_buffer = Arc::new(Mutex::new(std::collections::VecDeque::<f32>::new()));
    let mic_buffer = Arc::new(Mutex::new(std::collections::VecDeque::<f32>::new()));
    let config_clone = config.clone();
    gain_system.store(config.audio.volume_system.clamp(0.0, 1.5).to_bits(), Ordering::Relaxed);
    gain_mic.store(config.audio.volume_mic.clamp(0.0, 1.5).to_bits(), Ordering::Relaxed);
    let (gain_system, gain_mic) = (Arc::clone(gain_system), Arc::clone(gain_mic));

    // Feeder Thread continuo hacia FFmpeg a 48000 Hz estéreo cada 20ms
    std::thread::Builder::new()
        .name("sane-rec-win-audio-feeder".into())
        .spawn(move || {
            let host = cpal::default_host();
            let mut cap = WinCapture::start(&host, &config_clone, &system_buffer, &mic_buffer);


            let _ = listener.set_nonblocking(true);
            let mut stream_opt = None;
            while !stop_flag_clone.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((conn, _)) => {
                        stream_opt = Some(conn);
                        break;
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    Err(_) => return,
                }
            }
            let mut stream = match stream_opt {
                Some(s) => s,
                None => return,
            };
            let _ = stream.set_nonblocking(false);
            let _ = stream.set_nodelay(true);

            #[cfg(target_os = "windows")]
            #[link(name = "winmm")]
            extern "system" {
                fn timeBeginPeriod(uperiod: u32) -> u32;
                fn timeEndPeriod(uperiod: u32) -> u32;
            }

            struct WinHighPrecisionTimer;
            impl WinHighPrecisionTimer {
                fn new() -> Self {
                    #[cfg(target_os = "windows")]
                    unsafe {
                        timeBeginPeriod(1);
                    }
                    Self
                }
            }
            impl Drop for WinHighPrecisionTimer {
                fn drop(&mut self) {
                    #[cfg(target_os = "windows")]
                    unsafe {
                        timeEndPeriod(1);
                    }
                }
            }

            let _precision_timer = WinHighPrecisionTimer::new();

            // Al conectar FFmpeg, purgar cualquier muestra residual acumulada durante el arranque
            if let Ok(mut q) = system_buffer.lock() {
                q.clear();
            }
            if let Ok(mut q) = mic_buffer.lock() {
                q.clear();
            }

            let start_time = Instant::now();
            let mut total_frames_sent: u64 = 0;
            let mut send_buf = vec![0u8; 3840]; // 960 frames * 2 canales * 2 bytes = 3840 bytes
            let mut next_health = Instant::now() + Duration::from_millis(1000);

            while !stop_flag_clone.load(Ordering::Relaxed) {
                let elapsed = start_time.elapsed();
                let expected_frames = (elapsed.as_secs_f64() * 48000.0) as u64;

                // Protección contra drift / congelación prolongada de reloj (> 1s)
                if expected_frames > total_frames_sent + 48000 {
                    total_frames_sent = expected_frames;
                }

                // Tick de salud 1 Hz: reconecta WASAPI si cambió el dispositivo por defecto o el stream murió.
                if Instant::now() >= next_health {
                    next_health = Instant::now() + Duration::from_millis(1000);
                    if cap.is_stale(&host, &config_clone) {
                        // Soltar los streams ANTES de tocar los buffers (evita deadlock con cpal::Stream::drop).
                        drop(cap);
                        if let Ok(mut q) = system_buffer.lock() {
                            q.clear();
                        }
                        if let Ok(mut q) = mic_buffer.lock() {
                            q.clear();
                        }
                        cap = WinCapture::start(&host, &config_clone, &system_buffer, &mic_buffer);
                        eprintln!("Audio WASAPI: dispositivo reconectado ({} stream(s))", cap.streams.len());
                    }
                }

                // Control estricto de ritmo (48 kHz estéreo):
                // Si ya enviamos todos los fotogramas esperados hasta el tiempo real transcurrido,
                // dormimos de forma proporcional evitando spin-wait de 100% CPU y sin sobre-dormir.
                if total_frames_sent >= expected_frames {
                    let diff_frames = total_frames_sent - expected_frames;
                    let diff_ms = diff_frames / 48; // 48 frames por milisegundo a 48000 Hz
                    if diff_ms >= 2 {
                        let sleep_ms = ((diff_ms - 1).min(10)).max(1);
                        std::thread::sleep(Duration::from_millis(sleep_ms));
                    } else {
                        std::thread::yield_now();
                    }
                    continue;
                }

                {
                    let mut sys_q = system_buffer.lock().unwrap();
                    let mut mic_q = mic_buffer.lock().unwrap();
                    let gain_sys = f32::from_bits(gain_system.load(Ordering::Relaxed));
                    let gain_mic = f32::from_bits(gain_mic.load(Ordering::Relaxed));

                    // Purgar acumulación si el buffer excede 100ms (4800 frames = 9600 muestras)
                    if sys_q.len() > 9600 {
                        let excess = sys_q.len() - 4800;
                        sys_q.drain(..excess);
                    }
                    if mic_q.len() > 9600 {
                        let excess = mic_q.len() - 4800;
                        mic_q.drain(..excess);
                    }

                    let mut offset = 0;
                    for _ in 0..960 {
                        let sys_l = sys_q.pop_front().unwrap_or(0.0);
                        let sys_r = sys_q.pop_front().unwrap_or(0.0);
                        let mic_l = mic_q.pop_front().unwrap_or(0.0);
                        let mic_r = mic_q.pop_front().unwrap_or(0.0);

                        let mixed_l = (sys_l * gain_sys + mic_l * gain_mic).clamp(-1.0, 1.0);
                        let mixed_r = (sys_r * gain_sys + mic_r * gain_mic).clamp(-1.0, 1.0);

                        let s16_l = (mixed_l * 32767.0) as i16;
                        let s16_r = (mixed_r * 32767.0) as i16;

                        let b_l = s16_l.to_le_bytes();
                        let b_r = s16_r.to_le_bytes();

                        send_buf[offset] = b_l[0];
                        send_buf[offset + 1] = b_l[1];
                        send_buf[offset + 2] = b_r[0];
                        send_buf[offset + 3] = b_r[1];
                        offset += 4;
                    }
                }

                use std::io::Write;
                if stream.write_all(&send_buf).is_err() {
                    break;
                }

                total_frames_sent += 960;
            }
        })
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

    Ok((port, stop_flag))
}

/// Resuelve el códec de vídeo efectivo que usará FFmpeg en Windows y si es H.264 o HEVC.
/// Refleja exactamente la selección de códec de `apply_windows_profile_args`.
#[cfg(any(target_os = "windows", test))]
fn resolve_windows_encoder(config: &Config) -> (&'static str, bool, bool) {
    let profile = config
        .profile
        .active_profile
        .parse::<EncoderProfile>()
        .unwrap_or(EncoderProfile::SaneSafe);

    match profile {
        EncoderProfile::SaneSafe => ("libx264", true, false),
        EncoderProfile::AmdHardware => ("h264_amf", true, false),
        EncoderProfile::NvidiaHardware => ("h264_nvenc", true, false),
        EncoderProfile::IntelHardware => ("h264_qsv", true, false),
        EncoderProfile::Custom(_) => {
            let codec = if config.video.codec.is_empty() {
                "libx264"
            } else {
                config.video.codec.as_str()
            };
            let is_hevc = codec.contains("hevc") || codec.contains("265");

            if codec.contains("nvenc") {
                if is_hevc {
                    ("hevc_nvenc", false, true)
                } else {
                    ("h264_nvenc", true, false)
                }
            } else if codec.contains("amf") || codec.contains("vaapi") {
                if is_hevc {
                    ("hevc_amf", false, true)
                } else {
                    ("h264_amf", true, false)
                }
            } else if codec.contains("qsv") {
                if is_hevc {
                    ("hevc_qsv", false, true)
                } else {
                    ("h264_qsv", true, false)
                }
            } else if codec.contains("vp9") {
                ("libvpx-vp9", false, false)
            } else if codec == "libx265" || is_hevc {
                ("libx265", false, true)
            } else {
                ("libx264", true, false)
            }
        }
    }
}

/// Renderiza el comando FFmpeg completo para telemetría de benchmark.
#[cfg(target_os = "windows")]
fn render_ffmpeg_command(cmd: &Command) -> String {
    format!(
        "ffmpeg {}",
        cmd.get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    )
}

/// Aplica los argumentos de entrada y captura de pantalla y audio en Windows.
/// Utiliza Desktop Duplication API (ddagrab / DXGI) para captura nativa a 60 FPS acelerada por hardware,
/// con fallback automático o selector para gdigrab cuando se capturan ventanas individuales.
#[cfg(any(target_os = "windows", test))]
pub fn apply_windows_input_args(
    cmd: &mut Command,
    config: &Config,
    audio_port: Option<u16>,
    requested_backend: WindowsCaptureBackend,
) -> WindowsCaptureBackend {
    let fps_str = config.video.fps.to_string();
    let target = config.video.target.trim();

    // Input 0: Video (baja latencia y búfer ágil para 60 FPS en resoluciones altas)
    cmd.args([
        "-probesize",
        "32M",
        "-analyzeduration",
        "0",
        "-fflags",
        "+nobuffer+flush_packets",
        "-thread_queue_size",
        "2048",
    ]);

    let actual_backend = if target.starts_with("title=") || target.starts_with("window:") {
        let win_title = target
            .strip_prefix("title=")
            .or_else(|| target.strip_prefix("window:"))
            .unwrap_or(target)
            .trim();

        // 1. Intentar captura por hardware acelerada DXGI con recorte DWM exacto a 60 FPS
        if let Some(crop_info) = crate::discovery::get_window_monitor_crop_by_title(win_title) {
            let outputs = crate::discovery::get_connected_outputs();
            let output_idx = outputs
                .iter()
                .position(|o| o.name.eq_ignore_ascii_case(&crop_info.device_name))
                .unwrap_or(0);

            cmd.args([
                "-f",
                "lavfi",
                "-i",
                &format!(
                    "ddagrab=output_idx={}:framerate={}:draw_mouse=1",
                    output_idx, fps_str
                ),
            ]);
            WindowsCaptureBackend::DdaGrab
        } else if let Some((x, y, w, h)) = crate::discovery::get_window_capture_rect(win_title) {
            // 2. Fallback defensivo a GDI con coordenadas normalizadas si DWM no estuvo disponible
            if w >= 64 && h >= 64 {
                cmd.args([
                    "-f",
                    "gdigrab",
                    "-framerate",
                    &fps_str,
                    "-offset_x",
                    &x.to_string(),
                    "-offset_y",
                    &y.to_string(),
                    "-video_size",
                    &format!("{}x{}", w, h),
                    "-draw_mouse",
                    "1",
                    "-i",
                    "desktop",
                ]);
            } else {
                cmd.args([
                    "-f",
                    "gdigrab",
                    "-framerate",
                    &fps_str,
                    "-draw_mouse",
                    "1",
                    "-i",
                    "desktop",
                ]);
            }
            WindowsCaptureBackend::GdiGrab
        } else {
            cmd.args([
                "-f",
                "gdigrab",
                "-framerate",
                &fps_str,
                "-draw_mouse",
                "1",
                "-i",
                "desktop",
            ]);
            WindowsCaptureBackend::GdiGrab
        }
    } else {
        match requested_backend {
            WindowsCaptureBackend::DdaGrab => {
                let outputs = crate::discovery::get_connected_outputs();
                let output_idx = if !target.is_empty() && !target.eq_ignore_ascii_case("desktop") {
                    outputs
                        .iter()
                        .position(|o| o.name.eq_ignore_ascii_case(target))
                        .unwrap_or(0)
                } else {
                    outputs
                        .iter()
                        .position(|o| o.is_current)
                        .unwrap_or(0)
                };

                cmd.args([
                    "-f",
                    "lavfi",
                    "-i",
                    &format!(
                        "ddagrab=output_idx={}:framerate={}:draw_mouse=1",
                        output_idx, fps_str
                    ),
                ]);
                WindowsCaptureBackend::DdaGrab
            }
            WindowsCaptureBackend::GdiGrab => {
                let outputs = crate::discovery::get_connected_outputs();
                let matched_output = outputs
                    .iter()
                    .find(|o| o.name.eq_ignore_ascii_case(target))
                    .or_else(|| outputs.iter().find(|o| o.is_current))
                    .or_else(|| outputs.first());

                if let Some(o) = matched_output {
                    if o.width > 0 && o.height > 0 {
                        let w = (o.width & !1).max(2);
                        let h = (o.height & !1).max(2);
                        cmd.args([
                            "-f",
                            "gdigrab",
                            "-framerate",
                            &fps_str,
                            "-offset_x",
                            &o.x.to_string(),
                            "-offset_y",
                            &o.y.to_string(),
                            "-video_size",
                            &format!("{}x{}", w, h),
                            "-draw_mouse",
                            "1",
                            "-i",
                            "desktop",
                        ]);
                    } else {
                        cmd.args([
                            "-f",
                            "gdigrab",
                            "-framerate",
                            &fps_str,
                            "-draw_mouse",
                            "1",
                            "-i",
                            "desktop",
                        ]);
                    }
                } else {
                    cmd.args([
                        "-f",
                        "gdigrab",
                        "-framerate",
                        &fps_str,
                        "-draw_mouse",
                        "1",
                        "-i",
                        "desktop",
                    ]);
                }
                WindowsCaptureBackend::GdiGrab
            }
        }
    };

    // Input 1: Audio PCM crudo continuo s16le a 48000 Hz estéreo vía TCP local
    if let Some(port) = audio_port {
        let tcp_url = format!("tcp://127.0.0.1:{}", port);
        cmd.args([
            "-thread_queue_size",
            "1024",
            "-fflags",
            "+nobuffer",
            "-f",
            "s16le",
            "-ar",
            "48000",
            "-ac",
            "2",
            "-i",
            &tcp_url,
        ]);
    }

    actual_backend
}

/// Genera el filtro de video de Windows optimizado para resoluciones escaladas o nativas
/// respetando la relación de aspecto real del monitor sin forzar DAR 16:9.
#[cfg(any(target_os = "windows", test))]
pub fn build_windows_scale_filter(scale: &str) -> String {
    let s = scale.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("none") || s == "Nativa (100%)" {
        "scale=trunc(iw/2)*2:trunc(ih/2)*2".to_string()
    } else {
        let target_scale = s.replace('x', ":");
        format!("scale={},setsar=1/1", target_scale)
    }
}

/// Retorna `(preset_or_quality, qp_or_crf)` para los 4 encoders base en Windows según `QualityTier`.
#[cfg(any(target_os = "windows", test))]
fn windows_tier_params(profile: &EncoderProfile, tier: QualityTier) -> (&'static str, &'static str) {
    match (profile, tier) {
        (EncoderProfile::SaneSafe, QualityTier::Low) => ("ultrafast", "23"),
        (EncoderProfile::SaneSafe, QualityTier::Medium) => ("superfast", "21"),
        (EncoderProfile::SaneSafe, QualityTier::High) => ("faster", "18"),
        (EncoderProfile::AmdHardware, QualityTier::Low) => ("speed", "22"),
        (EncoderProfile::AmdHardware, QualityTier::Medium) => ("balanced", "20"),
        (EncoderProfile::AmdHardware, QualityTier::High) => ("quality", "18"),
        (EncoderProfile::NvidiaHardware, QualityTier::Low) => ("p1", "23"),
        (EncoderProfile::NvidiaHardware, QualityTier::Medium) => ("p4", "21"),
        (EncoderProfile::NvidiaHardware, QualityTier::High) => ("p5", "19"),
        (EncoderProfile::IntelHardware, QualityTier::Low) => ("veryfast", "23"),
        (EncoderProfile::IntelHardware, QualityTier::Medium) => ("faster", "21"),
        (EncoderProfile::IntelHardware, QualityTier::High) => ("medium", "18"),
        (EncoderProfile::Custom(_), _) => ("", ""),
    }
}

/// Configura los parámetros de codificación de hardware y CPU en Windows según el perfil activo.
#[cfg(any(target_os = "windows", test))]
pub fn apply_windows_profile_args(
    cmd: &mut Command,
    config: &Config,
    has_audio: bool,
    backend: WindowsCaptureBackend,
) {
    let profile = config
        .profile
        .active_profile
        .parse::<EncoderProfile>()
        .unwrap_or(EncoderProfile::SaneSafe);

    let scale_filter = build_windows_scale_filter(&config.video.scale);
    let target = config.video.target.trim();
    let crop_filter = if (target.starts_with("title=") || target.starts_with("window:"))
        && backend == WindowsCaptureBackend::DdaGrab
    {
        let win_title = target
            .strip_prefix("title=")
            .or_else(|| target.strip_prefix("window:"))
            .unwrap_or(target)
            .trim();
        crate::discovery::get_window_monitor_crop_by_title(win_title).map(|c| c.crop_filter)
    } else {
        None
    };

    let full_filter = if backend == WindowsCaptureBackend::DdaGrab {
        if let Some(crop) = crop_filter {
            format!("hwdownload,format=bgra,{},{}", crop, scale_filter)
        } else {
            format!("hwdownload,format=bgra,{}", scale_filter)
        }
    } else {
        scale_filter
    };
    let fps_str = config.video.fps.to_string();
    cmd.args(["-vf", &full_filter, "-r", &fps_str, "-fps_mode", "cfr"]);

    let cpu_threads = std::thread::available_parallelism()
        .map(|n| n.get().clamp(4, 8))
        .unwrap_or(4)
        .to_string();

    let (tier_preset, tier_qp) = windows_tier_params(&profile, config.profile.quality_tier);

    match profile {
        EncoderProfile::SaneSafe => {
            cmd.args([
                "-c:v",
                "libx264",
                "-preset",
                tier_preset,
                "-tune",
                "zerolatency",
                "-threads",
                &cpu_threads,
                "-crf",
                tier_qp,
                "-bf",
                "0",
                "-pix_fmt",
                "yuv420p",
            ]);
        }
        EncoderProfile::AmdHardware => {
            cmd.args([
                "-c:v",
                "h264_amf",
                "-usage",
                "lowlatency",
                "-quality",
                tier_preset,
                "-rc",
                "cqp",
                "-qp_i",
                tier_qp,
                "-qp_p",
                tier_qp,
            ]);
            cmd.arg("-pix_fmt").arg("nv12");
            cmd.args(["-header_insertion_mode", "gop"]);
        }
        EncoderProfile::NvidiaHardware => {
            cmd.args([
                "-c:v",
                "h264_nvenc",
                "-preset",
                tier_preset,
                "-tune",
                "ull",
                "-zerolatency",
                "1",
                "-rc",
                "constqp",
                "-qp",
                tier_qp,
                "-bf",
                "0",
            ]);
            cmd.arg("-pix_fmt").arg("yuv420p");
            cmd.args(["-forced-idr", "1", "-aud", "1"]);
        }
        EncoderProfile::IntelHardware => {
            cmd.args([
                "-c:v",
                "h264_qsv",
                "-preset",
                tier_preset,
                "-global_quality",
                tier_qp,
                "-pix_fmt",
                "nv12",
                "-bf",
                "0",
                "-async_depth",
                "1",
                "-look_ahead",
                "0",
                "-forced_idr",
                "1",
            ]);
        }
        EncoderProfile::Custom(_) => {
            let codec = if config.video.codec.is_empty() {
                "libx264"
            } else {
                config.video.codec.as_str()
            };
            let crf_str = config.video.crf.to_string();

            if codec.contains("nvenc") {
                let nv_preset = match config.video.preset.as_str() {
                    "ultrafast" | "p1" => "p1",
                    "superfast" | "p2" => "p2",
                    "veryfast" | "p3" => "p3",
                    "fast" | "p4" => "p4",
                    _ => "p1",
                };
                let win_codec = if codec.contains("hevc") || codec.contains("265") {
                    "hevc_nvenc"
                } else {
                    "h264_nvenc"
                };
                cmd.args([
                    "-c:v",
                    win_codec,
                    "-preset",
                    nv_preset,
                    "-tune",
                    "ull",
                    "-zerolatency",
                    "1",
                    "-rc",
                    "constqp",
                    "-qp",
                    &crf_str,
                    "-bf",
                    "0",
                    "-pix_fmt",
                    "yuv420p",
                    "-forced-idr",
                    "1",
                    "-aud",
                    "1",
                ]);
            } else if codec.contains("amf") || codec.contains("vaapi") {
                let amf_quality = match config.video.preset.as_str() {
                    "ultrafast" | "speed" => "speed",
                    "veryfast" | "balanced" => "balanced",
                    "fast" | "quality" => "quality",
                    _ => "speed",
                };
                let win_codec = if codec.contains("hevc") || codec.contains("265") {
                    "hevc_amf"
                } else {
                    "h264_amf"
                };
                let amf_qp = if config.video.crf > 0 {
                    config.video.crf
                } else {
                    22
                }
                .to_string();
                cmd.args([
                    "-c:v",
                    win_codec,
                    "-usage",
                    "lowlatency",
                    "-quality",
                    amf_quality,
                    "-rc",
                    "cqp",
                    "-qp_i",
                    &amf_qp,
                    "-qp_p",
                    &amf_qp,
                    "-pix_fmt",
                    "nv12",
                    "-header_insertion_mode",
                    "gop",
                ]);
            } else if codec.contains("qsv") {
                let qsv_preset = match config.video.preset.as_str() {
                    "ultrafast" => "veryfast",
                    "superfast" | "faster" => "faster",
                    "veryfast" => "fast",
                    "fast" | "medium" => "medium",
                    _ => "veryfast",
                };
                let win_codec = if codec.contains("hevc") || codec.contains("265") {
                    "hevc_qsv"
                } else {
                    "h264_qsv"
                };
                cmd.args([
                    "-c:v",
                    win_codec,
                    "-preset",
                    qsv_preset,
                    "-global_quality",
                    &crf_str,
                    "-pix_fmt",
                    "nv12",
                    "-bf",
                    "0",
                    "-async_depth",
                    "1",
                    "-look_ahead",
                    "0",
                    "-forced_idr",
                    "1",
                ]);
            } else if codec.contains("vp9") {
                let cpu_used = match config.video.preset.as_str() {
                    "ultrafast" => "8",
                    "superfast" => "7",
                    "veryfast" => "6",
                    "fast" => "5",
                    _ => "6",
                };
                cmd.args([
                    "-c:v",
                    "libvpx-vp9",
                    "-deadline",
                    "realtime",
                    "-cpu-used",
                    cpu_used,
                    "-crf",
                    &crf_str,
                    "-b:v",
                    "0",
                    "-pix_fmt",
                    "yuv420p",
                ]);
            } else if codec == "libx265" || codec.contains("265") || codec.contains("hevc") {
                let preset = if config.video.preset.is_empty() {
                    "ultrafast"
                } else {
                    config.video.preset.as_str()
                };
                cmd.args([
                    "-c:v",
                    "libx265",
                    "-preset",
                    preset,
                    "-crf",
                    &crf_str,
                    "-tune",
                    "zerolatency",
                    "-threads",
                    "4",
                    "-bf",
                    "0",
                    "-pix_fmt",
                    "yuv420p",
                ]);
            } else {
                let preset = if config.video.preset.is_empty() {
                    "ultrafast"
                } else {
                    config.video.preset.as_str()
                };
                cmd.args([
                    "-c:v",
                    codec,
                    "-preset",
                    preset,
                    "-crf",
                    &crf_str,
                    "-tune",
                    "zerolatency",
                    "-threads",
                    "4",
                    "-bf",
                    "0",
                    "-pix_fmt",
                    "yuv420p",
                ]);
            }
        }
    }

    if has_audio {
        cmd.args([
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-ar",
            "48000",
            "-ac",
            "2",
            "-af",
            "aresample=async=1000",
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
        ]);
    } else {
        cmd.args([
            "-an",
            "-map", "0:v:0",
        ]);
    }
}

/// Aplica los argumentos de destino (monitor o geometría) al comando wf-recorder.
fn apply_target_args(cmd: &mut Command, target: &str) {
    let t = target.trim();
    if t.starts_with("area:") || t.starts_with("geom:") {
        let geom = t.split_once(':').map(|(_, g)| g.trim()).unwrap_or(t);
        cmd.args(["-g", geom]);
    } else if t.contains(',') && t.contains('x') {
        cmd.args(["-g", t]);
    } else {
        let outputs = crate::discovery::get_connected_outputs();
        if !t.is_empty() && t != "portal" && t != "desktop" && !t.starts_with("window:") {
            if outputs.is_empty() || outputs.iter().any(|o| o.name == t) {
                cmd.args(["-o", t]);
            } else if let Some(fallback_out) = outputs.iter().find(|o| o.is_current).or_else(|| outputs.first()) {
                cmd.args(["-o", &fallback_out.name]);
            }
        } else if let Some(default_out) = outputs.iter().find(|o| o.is_current).or_else(|| outputs.first()) {
            cmd.args(["-o", &default_out.name]);
        }
    }
}

/// Retorna `(preset, qp_or_crf)` para los 4 encoders base en Linux según `QualityTier`.
/// En VA-API (AMD/Intel) el preset es vacío porque solo varía `qp` (Polaris safe).
fn linux_tier_qp(profile: &EncoderProfile, tier: QualityTier) -> (&'static str, &'static str) {
    match (profile, tier) {
        (EncoderProfile::SaneSafe, QualityTier::Low) => ("ultrafast", "23"),
        (EncoderProfile::SaneSafe, QualityTier::Medium) => ("superfast", "21"),
        (EncoderProfile::SaneSafe, QualityTier::High) => ("faster", "18"),
        (EncoderProfile::AmdHardware | EncoderProfile::IntelHardware, QualityTier::Low) => ("", "22"),
        (EncoderProfile::AmdHardware | EncoderProfile::IntelHardware, QualityTier::Medium) => ("", "20"),
        (EncoderProfile::AmdHardware | EncoderProfile::IntelHardware, QualityTier::High) => ("", "18"),
        (EncoderProfile::NvidiaHardware, QualityTier::Low) => ("p3", "23"),
        (EncoderProfile::NvidiaHardware, QualityTier::Medium) => ("p4", "21"),
        (EncoderProfile::NvidiaHardware, QualityTier::High) => ("p5", "19"),
        (EncoderProfile::Custom(_), _) => ("", ""),
    }
}

/// Aplica los argumentos de perfil de encoder al comando wf-recorder.
fn apply_profile_args(cmd: &mut Command, config: &Config) {
    let profile = config
        .profile
        .active_profile
        .parse::<EncoderProfile>()
        .unwrap_or(EncoderProfile::SaneSafe);

    let (tier_preset, tier_qp) = linux_tier_qp(&profile, config.profile.quality_tier);

    match profile {
        EncoderProfile::SaneSafe => {
            let preset_arg = format!("preset={}", tier_preset);
            let crf_arg = format!("crf={}", tier_qp);
            cmd.args([
                "-c",
                "libx264",
                "-p",
                &preset_arg,
                "-p",
                &crf_arg,
                "-p",
                "tune=zerolatency",
                "-p",
                "keyint=60",
                "-p",
                "g=60",
                "-p",
                "x264-params=keyint=60:min-keyint=60:no-scenecut=1",
                "-b",
                "0",
                "--no-dmabuf",
            ]);
        }
        EncoderProfile::AmdHardware | EncoderProfile::IntelHardware => {
            let qp_arg = format!("qp={}", tier_qp);
            if std::path::Path::new("/dev/dri/renderD128").exists() {
                cmd.args(["-d", "/dev/dri/renderD128"]);
            }
            cmd.args(["-c", "h264_vaapi", "-b", "0", "-p", &qp_arg]);
        }
        EncoderProfile::NvidiaHardware => {
            let preset_arg = format!("preset={}", tier_preset);
            let cq_arg = format!("cq={}", tier_qp);
            cmd.args(["-c", "h264_nvenc", "-b", "0", "-p", &preset_arg, "-p", &cq_arg]);
        }
        EncoderProfile::Custom(_) => {
            let codec = if config.video.codec.is_empty() {
                "libx264"
            } else {
                config.video.codec.as_str()
            };
            let crf_str = config.video.crf.to_string();

            if codec.contains("vaapi") || codec.contains("amf") || codec.contains("qsv") {
                let linux_codec = if codec.contains("hevc") || codec.contains("265") {
                    "hevc_vaapi"
                } else {
                    "h264_vaapi"
                };
                let qp_arg = format!("qp={}", crf_str);
                if codec.contains("vaapi") && std::path::Path::new("/dev/dri/renderD128").exists() {
                    cmd.args(["-d", "/dev/dri/renderD128"]);
                }
                cmd.args(["-c", linux_codec, "-b", "0", "-p", &qp_arg]);
            } else if codec.contains("nvenc") {
                let nv_preset = match config.video.preset.as_str() {
                    "ultrafast" | "p1" => "p1",
                    "superfast" | "p2" => "p2",
                    "veryfast" | "p3" => "p3",
                    "fast" | "p4" => "p4",
                    _ => "p3",
                };
                let linux_codec = if codec.contains("hevc") || codec.contains("265") {
                    "hevc_nvenc"
                } else {
                    "h264_nvenc"
                };
                let preset_arg = format!("preset={}", nv_preset);
                let cq_arg = format!("cq={}", crf_str);
                cmd.args(["-c", linux_codec, "-b", "0", "-p", &preset_arg, "-p", &cq_arg]);
            } else if codec.contains("vp9") {
                let cpu_used = match config.video.preset.as_str() {
                    "ultrafast" => "8",
                    "superfast" => "7",
                    "veryfast" => "6",
                    "fast" => "5",
                    _ => "6",
                };
                let cpu_arg = format!("cpu-used={}", cpu_used);
                let crf_arg = format!("crf={}", crf_str);
                cmd.args([
                    "-c",
                    "libvpx-vp9",
                    "-b",
                    "0",
                    "-p",
                    "deadline=realtime",
                    "-p",
                    &cpu_arg,
                    "-p",
                    &crf_arg,
                ]);
            } else {
                let preset = if config.video.preset.is_empty() {
                    "ultrafast"
                } else {
                    config.video.preset.as_str()
                };
                let preset_arg = format!("preset={}", preset);
                let crf_arg = format!("crf={}", crf_str);
                cmd.args([
                    "-c",
                    codec,
                    "-p",
                    &preset_arg,
                    "-p",
                    &crf_arg,
                    "-p",
                    "tune=zerolatency",
                    "-p",
                    "keyint=60",
                    "-p",
                    "g=60",
                    "-p",
                    "x264-params=keyint=60:min-keyint=60:no-scenecut=1",
                    "-b",
                    "0",
                    "--no-dmabuf",
                ]);
            }
        }
    }
}

/// Comprueba si una fuente de audio específica está disponible en PulseAudio / PipeWire en Linux.
#[cfg(target_os = "linux")]
pub fn is_source_available(source_name: &str) -> bool {
    let output = match new_hidden_command("pactl")
        .args(["list", "short", "sources"])
        .output()
    {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => return false,
    };

    output.lines().any(|line| {
        let parts: Vec<&str> = line.split_whitespace().collect();
        parts.get(1).map_or(false, |name| *name == source_name)
    })
}

#[cfg(not(target_os = "linux"))]
pub fn is_source_available(source_name: &str) -> bool {
    !source_name.is_empty()
}

/// Obtiene el monitor o nombre del sink de audio por defecto usando pactl en Linux o WASAPI en Windows.
#[cfg(target_os = "linux")]
pub fn get_default_audio_sink() -> Option<String> {
    let output = new_hidden_command("pactl")
        .arg("get-default-sink")
        .output()
        .ok()?;

    if output.status.success() {
        let sink = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !sink.is_empty() {
            return Some(sink);
        }
    }
    None
}

#[cfg(not(target_os = "linux"))]
pub fn get_default_audio_sink() -> Option<String> {
    Some("default_output".to_string())
}

/// Obtiene la fuente de entrada (micrófono) por defecto usando pactl en Linux o WASAPI en Windows.
#[cfg(target_os = "linux")]
pub fn get_default_audio_source() -> Option<String> {
    if is_source_available("easyeffects_source") {
        return Some("easyeffects_source".to_string());
    }

    let output = new_hidden_command("pactl")
        .arg("get-default-source")
        .output()
        .ok()?;

    if output.status.success() {
        let source = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !source.is_empty() {
            return Some(source);
        }
    }
    None
}

#[cfg(not(target_os = "linux"))]
pub fn get_default_audio_source() -> Option<String> {
    Some("default_input".to_string())
}

/// Reproduce un efecto sonoro de confirmación utilizando el servidor PipeWire/pw-play de forma desacoplada y sin procesos zombi.
#[cfg(target_os = "linux")]
pub fn play_feedback_sound(sound_name: &str) {
    let path = format!("/usr/share/sounds/freedesktop/stereo/{}.oga", sound_name);
    if std::path::Path::new(&path).exists() {
        std::thread::spawn(move || {
            if let Ok(mut child) = new_hidden_command("pw-play")
                .arg(&path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                let _ = child.wait();
            }
        });
    }
}

#[cfg(not(target_os = "linux"))]
pub fn play_feedback_sound(_sound_name: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts_pkt(pusi: bool, pid: u16) -> Vec<u8> {
        let mut p = vec![0u8; 188];
        p[0] = 0x47;
        p[1] = (if pusi { 0x40 } else { 0 }) | ((pid >> 8) as u8 & 0x1F);
        p[2] = pid as u8;
        p
    }

    #[test]
    fn test_chunk_has_video_pusi() {
        assert!(chunk_has_video_pusi(&ts_pkt(true, 0x100)));
        let mut two = ts_pkt(true, 0x101);
        two.extend(ts_pkt(true, 0x100));
        assert!(chunk_has_video_pusi(&two));
        assert!(!chunk_has_video_pusi(&ts_pkt(true, 0x101)));
        assert!(!chunk_has_video_pusi(&ts_pkt(false, 0x100)));
        assert!(!chunk_has_video_pusi(&[]));
        assert!(!chunk_has_video_pusi(&ts_pkt(true, 0x100)[..187]));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn test_default_buffer_dir_contains_uid() {
        let dir = default_buffer_dir();
        assert!(dir.to_string_lossy().starts_with("/dev/shm/sane-rec-"));
    }

    #[test]
    fn test_engine_status_serialization() {
        let status = EngineStatus {
            is_running: true,
            is_recording: false,
            duration_secs: 30,
            current_buffer_secs: 14,
            recording_elapsed_secs: 0,
            ram_usage_mb: 28.5,
            max_memory_mb: 150,
            active_profile: "sane-safe".to_string(),
            output_dir: std::env::temp_dir().join("sane-rec-test-replays"),
        };

        let serialized = serde_json::to_string(&status).unwrap();
        let deserialized: EngineStatus = serde_json::from_str(&serialized).unwrap();
        assert_eq!(status, deserialized);
    }

    #[test]
    fn test_clean_buffer_and_scan_segments() {
        let temp_dir = std::env::temp_dir()
            .join(format!("sane_rec_engine_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);

        let mut config = Config::default();
        config.replay.duration_secs = 20;

        let engine = ReplayEngine::with_buffer_dir(config, temp_dir.clone()).unwrap();

        // Crear 3 segmentos ficticios
        let seg0 = temp_dir.join("seg_00.mp4");
        let seg1 = temp_dir.join("seg_01.mp4");
        let seg2 = temp_dir.join("seg_02.mp4");
        let placeholder = temp_dir.join("seg_%02d.mp4");
        let other = temp_dir.join("other.txt");

        std::fs::write(&seg0, b"data0").unwrap();
        std::fs::write(&seg1, b"data1").unwrap();
        std::fs::write(&seg2, b"data2").unwrap();
        std::fs::write(&placeholder, b"").unwrap(); // Tamaño 0
        std::fs::write(&other, b"ignore").unwrap();

        let segments = engine.scan_segments().unwrap();
        assert_eq!(segments.len(), 3);

        let status = engine.get_status();
        assert_eq!(status.duration_secs, 20);

        engine.clean_buffer().unwrap();
        let remaining = engine.scan_segments().unwrap();
        assert_eq!(remaining.len(), 0);
        assert!(other.exists(), "Other files should not be deleted");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_save_clip_concatenation_with_ffmpeg() {
        let temp_dir =
            std::env::temp_dir().join(format!("sane_rec_clip_test_{}", std::process::id()));
        let buffer_dir = temp_dir.join("buffer");
        let output_dir = temp_dir.join("output");
        let _ = std::fs::create_dir_all(&buffer_dir);
        let _ = std::fs::create_dir_all(&output_dir);

        let mut config = Config::default();
        config.storage.output_dir = output_dir.clone();
        config.storage.format = "mp4".to_string();

        let engine = ReplayEngine::with_buffer_dir(config, buffer_dir.clone()).unwrap();

        // Generar 2 segmentos válidos de 1 segundo usando lavfi testsrc con ffmpeg
        let seg0 = buffer_dir.join("seg_00.mp4");
        let seg1 = buffer_dir.join("seg_01.mp4");

        let cmd0 = new_hidden_command("ffmpeg")
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=1:size=320x240:rate=30",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
            ])
            .arg(&seg0)
            .output();

        let cmd1 = new_hidden_command("ffmpeg")
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=1:size=320x240:rate=30",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
            ])
            .arg(&seg1)
            .output();

        if let (Ok(r0), Ok(r1)) = (cmd0, cmd1) {
            if r0.status.success() && r1.status.success() {
                assert!(seg0.exists());
                assert!(seg1.exists());
                let clip_path = engine.save_clip().expect("save_clip should succeed");

                assert!(clip_path.exists());
                assert!(clip_path.metadata().unwrap().len() > 0);
                assert_eq!(clip_path.extension().unwrap(), "mp4");
            }
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_audio_modules_tracking_and_cleanup() {
        let temp_dir = std::env::temp_dir()
            .join(format!("sane_rec_audio_mod_test_{}", std::process::id()));
        let config = Config::default();
        let engine = ReplayEngine::with_buffer_dir(config, temp_dir.clone()).unwrap();

        // Registrar módulos ficticios en audio_modules
        {
            let mut modules = engine.audio_modules.lock().unwrap();
            modules.push(888111);
            modules.push(888222);
        }
        assert_eq!(engine.audio_modules().len(), 2);

        // cleanup_audio_modules debe vaciar la lista completamente
        engine.cleanup_audio_modules();
        assert_eq!(engine.audio_modules().len(), 0);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_save_clip_exact_duration_selection() {
        let temp_dir =
            std::env::temp_dir().join(format!("sane_rec_duration_test_{}", std::process::id()));
        let buffer_dir = temp_dir.join("buffer");
        let output_dir = temp_dir.join("output");
        let _ = std::fs::create_dir_all(&buffer_dir);
        let _ = std::fs::create_dir_all(&output_dir);

        let mut config = Config::default();
        config.storage.output_dir = output_dir.clone();
        config.storage.format = "mp4".to_string();
        config.replay.duration_secs = 2; // Solo debe seleccionar los últimos 2 de 3

        let engine = ReplayEngine::with_buffer_dir(config, buffer_dir.clone()).unwrap();

        let seg0 = buffer_dir.join("seg_00.mp4");
        let seg1 = buffer_dir.join("seg_01.mp4");
        let seg2 = buffer_dir.join("seg_02.mp4");

        let _ = new_hidden_command("ffmpeg")
            .args(["-y", "-f", "lavfi", "-i", "testsrc=duration=1:size=160x120:rate=30", "-c:v", "libx264", "-preset", "ultrafast"])
            .arg(&seg0)
            .output();
        std::thread::sleep(std::time::Duration::from_millis(15));

        let _ = new_hidden_command("ffmpeg")
            .args(["-y", "-f", "lavfi", "-i", "testsrc=duration=1:size=160x120:rate=30", "-c:v", "libx264", "-preset", "ultrafast"])
            .arg(&seg1)
            .output();
        std::thread::sleep(std::time::Duration::from_millis(15));

        let _ = new_hidden_command("ffmpeg")
            .args(["-y", "-f", "lavfi", "-i", "testsrc=duration=1:size=160x120:rate=30", "-c:v", "libx264", "-preset", "ultrafast"])
            .arg(&seg2)
            .output();

        if seg0.exists() && seg1.exists() && seg2.exists() {
            let clip = engine.save_clip().expect("save_clip should succeed with exact segments");
            assert!(clip.exists());
            assert!(clip.metadata().unwrap().len() > 0);
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn test_is_source_available_nonexistent() {
        assert!(!super::is_source_available("nonexistent_device_source_xyz_12345"));
    }

    #[test]
    fn test_mpegts_packet_alignment_simulation() {
        // Simular buffer entrante desalineado: 10 bytes de basura + 2 paquetes de 188 bytes + 50 bytes parciales
        let mut raw = Vec::new();
        raw.extend_from_slice(&[0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99]); // 10 bytes basura

        let mut packet1 = vec![0x12u8; 188];
        packet1[0] = 0x47; // Sync byte
        raw.extend_from_slice(&packet1);

        let mut packet2 = vec![0x34u8; 188];
        packet2[0] = 0x47; // Sync byte
        raw.extend_from_slice(&packet2);

        let mut partial = vec![0x56u8; 50];
        partial[0] = 0x47; // Inicio del tercer paquete
        raw.extend_from_slice(&partial);

        let mut leftover = Vec::new();
        leftover.extend_from_slice(&raw);

        // Algoritmo de alineación implementado en el hilo de captura
        if let Some(sync_pos) = leftover.iter().position(|&b| b == 0x47) {
            if sync_pos > 0 {
                leftover.drain(..sync_pos);
            }
        }
        let aligned_len = (leftover.len() / 188) * 188;
        assert_eq!(aligned_len, 376); // 2 paquetes completos

        let chunk: Vec<u8> = leftover.drain(..aligned_len).collect();
        assert_eq!(chunk.len(), 376);
        assert_eq!(chunk[0], 0x47);
        assert_eq!(chunk[188], 0x47);
        assert_eq!(leftover.len(), 50);
        assert_eq!(leftover[0], 0x47);
    }

    #[test]
    fn test_windows_audio_resampler_stereo_48k() {
        let out = Arc::new(Mutex::new(std::collections::VecDeque::new()));
        let mut resampler = WindowsAudioResampler::new(48000, 2, 0.5);

        let input_samples: Vec<f32> = vec![1.0, -1.0, 0.5, -0.5];
        resampler.process_f32(&input_samples, &out);

        let q = out.lock().unwrap();
        assert_eq!(q.len(), 4);
        assert!((q[0] - 0.5).abs() < 1e-4);
        assert!((q[1] - (-0.5)).abs() < 1e-4);
        assert!((q[2] - 0.25).abs() < 1e-4);
        assert!((q[3] - (-0.25)).abs() < 1e-4);
    }

    #[test]
    fn test_windows_audio_resampler_i32_and_f64() {
        let out = Arc::new(Mutex::new(std::collections::VecDeque::new()));
        let mut resampler = WindowsAudioResampler::new(48000, 2, 1.0);
        resampler.process_i32(&[i32::MIN, 1 << 30], &out);
        resampler.process_f64(&[0.25f64, -0.5f64], &out);

        let q = out.lock().unwrap();
        assert_eq!(q.len(), 4);
        assert!((q[0] - (-1.0)).abs() < 1e-4);
        assert!((q[1] - 0.5).abs() < 1e-4);
        assert!((q[2] - 0.25).abs() < 1e-4);
        assert!((q[3] - (-0.5)).abs() < 1e-4);
    }

    #[test]
    fn test_audio_endpoint_needs_rebuild() {
        let a = ("Speakers".to_string(), 48000u32, 2u16);
        let b = ("Headset".to_string(), 44100u32, 2u16);
        assert!(!audio_endpoint_needs_rebuild(true, Some(&a), false, Some(&a)));
        assert!(audio_endpoint_needs_rebuild(true, Some(&a), false, Some(&b)));
        assert!(audio_endpoint_needs_rebuild(true, Some(&a), true, Some(&a)));
        assert!(audio_endpoint_needs_rebuild(true, None, false, Some(&a)));
        assert!(!audio_endpoint_needs_rebuild(false, Some(&a), true, Some(&b)));
    }

    #[test]
    fn test_windows_audio_resampler_mono_resample_44100() {
        let out = Arc::new(Mutex::new(std::collections::VecDeque::new()));
        let mut resampler = WindowsAudioResampler::new(44100, 1, 1.0);

        // Generar 4410 samples mono a 44100 Hz (0.1 segundos)
        let mono_samples = vec![0.5f32; 4410];
        resampler.process_f32(&mono_samples, &out);

        let q = out.lock().unwrap();
        // 0.1s a 48000 Hz estéreo da aproximadamente 4800 frames * 2 = 9600 samples
        assert!(q.len() >= 9500 && q.len() <= 9700);
        // Cada sample resultante debe ser ~0.5 una vez estabilizada la interpolación inicial
        for &s in q.iter().skip(4) {
            assert!((s - 0.5).abs() < 0.05);
        }
    }

    #[test]
    fn test_apply_profile_args_hevc_vaapi() {
        let mut config = Config::default();
        config.profile.active_profile = "custom".to_string();
        config.video.codec = "hevc_vaapi".to_string();
        config.video.crf = 24;

        let mut cmd = Command::new("wf-recorder");
        apply_profile_args(&mut cmd, &config);

        let args: Vec<String> = cmd
            .get_args()
            .map(|s| s.to_string_lossy().to_string())
            .collect();
        let c_idx = args.iter().position(|a| a == "-c").expect("should have -c flag");
        assert_eq!(args[c_idx + 1], "hevc_vaapi");
        assert!(args.contains(&"qp=24".to_string()));
    }

    #[test]
    fn test_apply_profile_args_hevc_nvenc() {
        let mut config = Config::default();
        config.profile.active_profile = "custom".to_string();
        config.video.codec = "hevc_nvenc".to_string();
        config.video.crf = 20;

        let mut cmd = Command::new("wf-recorder");
        apply_profile_args(&mut cmd, &config);

        let args: Vec<String> = cmd
            .get_args()
            .map(|s| s.to_string_lossy().to_string())
            .collect();
        let c_idx = args.iter().position(|a| a == "-c").expect("should have -c flag");
        assert_eq!(args[c_idx + 1], "hevc_nvenc");
        assert!(args.contains(&"cq=20".to_string()));
    }

    #[test]
    fn test_apply_target_args_geometry_and_fallback() {
        let mut cmd1 = Command::new("wf-recorder");
        apply_target_args(&mut cmd1, "area: 100,100 800x600");
        let args1: Vec<String> = cmd1.get_args().map(|s| s.to_string_lossy().to_string()).collect();
        assert_eq!(args1, vec!["-g", "100,100 800x600"]);

        let mut cmd2 = Command::new("wf-recorder");
        apply_target_args(&mut cmd2, "portal");
        let args2: Vec<String> = cmd2.get_args().map(|s| s.to_string_lossy().to_string()).collect();
        let outputs = crate::discovery::get_connected_outputs();
        if !outputs.is_empty() {
            assert!(args2.contains(&"-o".to_string()));
            let o_idx = args2.iter().position(|a| a == "-o").unwrap();
            let selected_name = &args2[o_idx + 1];
            assert!(outputs.iter().any(|o| &o.name == selected_name));
        }

        let mut cmd3 = Command::new("wf-recorder");
        apply_target_args(&mut cmd3, "NONEXISTENT_MONITOR_XYZ");
        let args3: Vec<String> = cmd3.get_args().map(|s| s.to_string_lossy().to_string()).collect();
        if !outputs.is_empty() {
            assert!(args3.contains(&"-o".to_string()));
            let o_idx = args3.iter().position(|a| a == "-o").unwrap();
            let selected_name = &args3[o_idx + 1];
            assert_ne!(selected_name, "NONEXISTENT_MONITOR_XYZ");
            assert!(outputs.iter().any(|o| &o.name == selected_name));
        }
    }

    #[test]
    fn test_windows_scale_filter_stretched_res() {
        let f_native = build_windows_scale_filter("none");
        assert_eq!(f_native, "scale=trunc(iw/2)*2:trunc(ih/2)*2");

        let f_empty = build_windows_scale_filter("");
        assert_eq!(f_empty, "scale=trunc(iw/2)*2:trunc(ih/2)*2");

        let f_nativa_str = build_windows_scale_filter("Nativa (100%)");
        assert_eq!(f_nativa_str, "scale=trunc(iw/2)*2:trunc(ih/2)*2");

        let f_1080p = build_windows_scale_filter("1920x1080");
        assert_eq!(f_1080p, "scale=1920:1080,setsar=1/1");

        let f_stretched = build_windows_scale_filter("1440x1080");
        assert_eq!(f_stretched, "scale=1440:1080,setsar=1/1");
    }

    #[test]
    fn test_windows_scale_filter_never_forces_16_9_dar() {
        let f_native = build_windows_scale_filter("none");
        assert!(!f_native.contains("setdar") && !f_native.contains("16/9") && !f_native.contains("16:9"));

        let f_custom = build_windows_scale_filter("1280x1024");
        assert!(!f_custom.contains("setdar") && !f_custom.contains("16/9") && !f_custom.contains("16:9"));
    }

    #[test]
    fn test_apply_windows_input_args_ddagrab_and_window_fallback() {
        let mut config = Config::default();
        config.video.target = "desktop".to_string();
        config.video.fps = 60;

        let mut cmd = Command::new("ffmpeg");
        let backend = apply_windows_input_args(&mut cmd, &config, None, WindowsCaptureBackend::DdaGrab);
        assert_eq!(backend, WindowsCaptureBackend::DdaGrab);

        let args: Vec<String> = cmd.get_args().map(|s| s.to_string_lossy().to_string()).collect();
        assert!(args.contains(&"-f".to_string()));
        assert!(args.contains(&"lavfi".to_string()));
        assert!(args.iter().any(|a| a.starts_with("ddagrab=")));
        assert!(args.iter().any(|a| a.contains("framerate=60")));

        // Test window target automatically uses GDI grab
        let mut win_config = Config::default();
        win_config.video.target = "title=Fortnite".to_string();
        let mut win_cmd = Command::new("ffmpeg");
        let win_backend = apply_windows_input_args(&mut win_cmd, &win_config, None, WindowsCaptureBackend::DdaGrab);
        assert_eq!(win_backend, WindowsCaptureBackend::GdiGrab);

        let win_args: Vec<String> = win_cmd.get_args().map(|s| s.to_string_lossy().to_string()).collect();
        assert!(win_args.contains(&"gdigrab".to_string()));
    }

    #[test]
    fn test_apply_windows_profile_args_filters_and_threads() {
        let mut config = Config::default();
        config.profile.active_profile = "sane-safe".to_string();
        config.video.fps = 60;
        config.video.scale = "1920x1080".to_string();

        // 1. Con DdaGrab, el filtro DEBE comenzar con hwdownload,format=bgra
        let mut dda_cmd = Command::new("ffmpeg");
        apply_windows_profile_args(&mut dda_cmd, &config, false, WindowsCaptureBackend::DdaGrab);
        let dda_args: Vec<String> = dda_cmd.get_args().map(|s| s.to_string_lossy().to_string()).collect();
        let vf_idx = dda_args.iter().position(|a| a == "-vf").expect("should have -vf");
        assert!(dda_args[vf_idx + 1].starts_with("hwdownload,format=bgra,scale=1920:1080"));
        assert!(dda_args.contains(&"libx264".to_string()));
        assert!(dda_args.contains(&"-threads".to_string()));

        // 2. Con GdiGrab, el filtro NO debe tener hwdownload
        let mut gdi_cmd = Command::new("ffmpeg");
        apply_windows_profile_args(&mut gdi_cmd, &config, false, WindowsCaptureBackend::GdiGrab);
        let gdi_args: Vec<String> = gdi_cmd.get_args().map(|s| s.to_string_lossy().to_string()).collect();
        let gdi_vf_idx = gdi_args.iter().position(|a| a == "-vf").expect("should have -vf");
        assert!(!gdi_args[gdi_vf_idx + 1].contains("hwdownload"));
        assert_eq!(gdi_args[gdi_vf_idx + 1], "scale=1920:1080,setsar=1/1");
    }

    fn win_args(cmd: &Command) -> Vec<String> {
        cmd.get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    fn win_flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .map(|s| s.as_str())
    }

    #[test]
    fn test_resolve_windows_encoder_aliases() {
        let check = |profile: &str, codec: &str, expected: (&str, bool, bool)| {
            let mut config = Config::default();
            config.profile.active_profile = profile.to_string();
            config.video.codec = codec.to_string();
            assert_eq!(
                resolve_windows_encoder(&config),
                expected,
                "perfil={profile} códec={codec}"
            );
        };

        check("sane-safe", "", ("libx264", true, false));
        check("amd-hardware", "", ("h264_amf", true, false));
        check("amd-vaapi", "hevc_amf", ("h264_amf", true, false));
        check("nvidia-hardware", "", ("h264_nvenc", true, false));
        check("nvidia-nvenc", "hevc_nvenc", ("h264_nvenc", true, false));
        check("intel-hardware", "", ("h264_qsv", true, false));
        check("intel-qsv", "h264_nvenc", ("h264_qsv", true, false));

        check("custom", "h264_nvenc", ("h264_nvenc", true, false));
        check("custom", "hevc_nvenc", ("hevc_nvenc", false, true));
        check("custom", "h264_amf", ("h264_amf", true, false));
        check("custom", "hevc_amf", ("hevc_amf", false, true));
        check("custom", "h264_vaapi", ("h264_amf", true, false));
        check("custom", "h264_qsv", ("h264_qsv", true, false));
        check("custom", "hevc_qsv", ("hevc_qsv", false, true));
        check("custom", "libvpx-vp9", ("libvpx-vp9", false, false));
        check("custom", "libx265", ("libx265", false, true));
        check("custom", "libx264", ("libx264", true, false));
    }

    #[test]
    fn test_apply_windows_profile_args_hardware_encoders() {
        let mut qsv_config = Config::default();
        qsv_config.profile.active_profile = "intel-hardware".to_string();
        qsv_config.video.target = "desktop".to_string();
        qsv_config.video.crf = 23;
        let mut qsv_cmd = Command::new("ffmpeg");
        apply_windows_profile_args(&mut qsv_cmd, &qsv_config, true, WindowsCaptureBackend::GdiGrab);
        let qsv = win_args(&qsv_cmd);

        assert!(qsv.contains(&"h264_qsv".to_string()));
        assert_eq!(win_flag_value(&qsv, "-pix_fmt"), Some("nv12"));
        assert_eq!(win_flag_value(&qsv, "-bf"), Some("0"));
        assert_eq!(win_flag_value(&qsv, "-async_depth"), Some("1"));
        assert_eq!(win_flag_value(&qsv, "-look_ahead"), Some("0"));
        assert_eq!(win_flag_value(&qsv, "-forced_idr"), Some("1"));
        assert_eq!(
            win_flag_value(&qsv, "-af"),
            Some("aresample=async=1000")
        );

        let mut amf_config = Config::default();
        amf_config.profile.active_profile = "amd-hardware".to_string();
        amf_config.video.target = "desktop".to_string();
        let mut amf_cmd = Command::new("ffmpeg");
        apply_windows_profile_args(&mut amf_cmd, &amf_config, false, WindowsCaptureBackend::GdiGrab);
        let amf = win_args(&amf_cmd);

        assert!(amf.contains(&"h264_amf".to_string()));
        assert_eq!(win_flag_value(&amf, "-usage"), Some("lowlatency"));
        assert_eq!(win_flag_value(&amf, "-pix_fmt"), Some("nv12"));
        assert_eq!(win_flag_value(&amf, "-header_insertion_mode"), Some("gop"));
        assert!(!amf.contains(&"-bf".to_string()), "AMF no debe usar -bf 0");
        assert!(!amf.contains(&"ultralowlatency".to_string()));

        let mut nvenc_config = Config::default();
        nvenc_config.profile.active_profile = "nvidia-hardware".to_string();
        nvenc_config.video.target = "desktop".to_string();
        let mut nvenc_cmd = Command::new("ffmpeg");
        apply_windows_profile_args(&mut nvenc_cmd, &nvenc_config, false, WindowsCaptureBackend::GdiGrab);
        let nvenc = win_args(&nvenc_cmd);

        assert!(nvenc.contains(&"h264_nvenc".to_string()));
        assert_eq!(win_flag_value(&nvenc, "-forced-idr"), Some("1"));
        assert_eq!(win_flag_value(&nvenc, "-aud"), Some("1"));
        assert_eq!(win_flag_value(&nvenc, "-zerolatency"), Some("1"));
    }

    #[test]
    fn test_apply_windows_input_args_audio_nobuffer() {
        let mut config = Config::default();
        config.video.target = "desktop".to_string();
        config.video.fps = 60;

        let mut cmd = Command::new("ffmpeg");
        apply_windows_input_args(&mut cmd, &config, Some(12345), WindowsCaptureBackend::GdiGrab);
        let args = win_args(&cmd);

        assert!(
            !args.contains(&"-use_wallclock_as_timestamps".to_string()),
            "no debe usar wallclock en el audio TCP: {:?}",
            args
        );
        assert!(
            args.windows(2).any(|w| w[0] == "-fflags" && w[1] == "+nobuffer"),
            "el audio TCP debe desactivar el buffering: {:?}",
            args
        );
        assert!(args.contains(&"tcp://127.0.0.1:12345".to_string()));
    }

    #[test]
    fn test_windows_quality_tiers_all_base_encoders_and_custom() {
        let run_win = |profile: &str, tier: QualityTier| -> Vec<String> {
            let mut cfg = Config::default();
            cfg.profile.active_profile = profile.to_string();
            cfg.profile.quality_tier = tier;
            let mut cmd = Command::new("ffmpeg");
            apply_windows_profile_args(&mut cmd, &cfg, false, WindowsCaptureBackend::GdiGrab);
            win_args(&cmd)
        };

        // SaneSafe: Low=ultrafast/23, Medium=superfast/21, High=faster/18
        for (tier, preset, crf) in [
            (QualityTier::Low, "ultrafast", "23"),
            (QualityTier::Medium, "superfast", "21"),
            (QualityTier::High, "faster", "18"),
        ] {
            let args = run_win("sane-safe", tier);
            assert_eq!(win_flag_value(&args, "-c:v"), Some("libx264"));
            assert_eq!(win_flag_value(&args, "-preset"), Some(preset));
            assert_eq!(win_flag_value(&args, "-crf"), Some(crf));
            assert_eq!(win_flag_value(&args, "-tune"), Some("zerolatency"));
            assert_eq!(win_flag_value(&args, "-bf"), Some("0"));
        }

        // AmdHardware (AMF): Low=speed/22, Medium=balanced/20, High=quality/18
        for (tier, quality, qp) in [
            (QualityTier::Low, "speed", "22"),
            (QualityTier::Medium, "balanced", "20"),
            (QualityTier::High, "quality", "18"),
        ] {
            let args = run_win("amd-hardware", tier);
            assert_eq!(win_flag_value(&args, "-c:v"), Some("h264_amf"));
            assert_eq!(win_flag_value(&args, "-usage"), Some("lowlatency"));
            assert_eq!(win_flag_value(&args, "-quality"), Some(quality));
            assert_eq!(win_flag_value(&args, "-qp_i"), Some(qp));
            assert_eq!(win_flag_value(&args, "-qp_p"), Some(qp));
            assert_eq!(win_flag_value(&args, "-header_insertion_mode"), Some("gop"));
            assert!(!args.contains(&"-bf".to_string()));
        }

        // NvidiaHardware (NVENC): Low=p1/23, Medium=p4/21, High=p5/19
        for (tier, preset, qp) in [
            (QualityTier::Low, "p1", "23"),
            (QualityTier::Medium, "p4", "21"),
            (QualityTier::High, "p5", "19"),
        ] {
            let args = run_win("nvidia-hardware", tier);
            assert_eq!(win_flag_value(&args, "-c:v"), Some("h264_nvenc"));
            assert_eq!(win_flag_value(&args, "-preset"), Some(preset));
            assert_eq!(win_flag_value(&args, "-qp"), Some(qp));
            assert_eq!(win_flag_value(&args, "-tune"), Some("ull"));
            assert_eq!(win_flag_value(&args, "-rc"), Some("constqp"));
            assert_eq!(win_flag_value(&args, "-bf"), Some("0"));
            assert_eq!(win_flag_value(&args, "-forced-idr"), Some("1"));
            assert_eq!(win_flag_value(&args, "-aud"), Some("1"));
        }

        // IntelHardware (QSV): Low=veryfast/23, Medium=faster/21, High=medium/18
        for (tier, preset, gq) in [
            (QualityTier::Low, "veryfast", "23"),
            (QualityTier::Medium, "faster", "21"),
            (QualityTier::High, "medium", "18"),
        ] {
            let args = run_win("intel-hardware", tier);
            assert_eq!(win_flag_value(&args, "-c:v"), Some("h264_qsv"));
            assert_eq!(win_flag_value(&args, "-preset"), Some(preset));
            assert_eq!(win_flag_value(&args, "-global_quality"), Some(gq));
            assert_eq!(win_flag_value(&args, "-bf"), Some("0"));
            assert_eq!(win_flag_value(&args, "-async_depth"), Some("1"));
            assert_eq!(win_flag_value(&args, "-look_ahead"), Some("0"));
            assert_eq!(win_flag_value(&args, "-forced_idr"), Some("1"));
        }

        // Custom sigue usando config.video.preset y config.video.crf incluso en High
        let mut custom_cfg = Config::default();
        custom_cfg.profile.active_profile = "custom".to_string();
        custom_cfg.profile.quality_tier = QualityTier::High;
        custom_cfg.video.codec = "libx264".to_string();
        custom_cfg.video.preset = "veryfast".to_string();
        custom_cfg.video.crf = 26;
        let mut custom_cmd = Command::new("ffmpeg");
        apply_windows_profile_args(&mut custom_cmd, &custom_cfg, false, WindowsCaptureBackend::GdiGrab);
        let custom_args = win_args(&custom_cmd);
        assert_eq!(win_flag_value(&custom_args, "-preset"), Some("veryfast"));
        assert_eq!(win_flag_value(&custom_args, "-crf"), Some("26"));
    }

    #[test]
    fn test_linux_quality_tiers_all_base_encoders_and_custom() {
        let run_linux = |profile: &str, tier: QualityTier| -> Vec<String> {
            let mut cfg = Config::default();
            cfg.profile.active_profile = profile.to_string();
            cfg.profile.quality_tier = tier;
            let mut cmd = Command::new("wf-recorder");
            apply_profile_args(&mut cmd, &cfg);
            win_args(&cmd)
        };

        // SaneSafe: Low=ultrafast/23, Medium=superfast/21, High=faster/18
        for (tier, preset, crf) in [
            (QualityTier::Low, "preset=ultrafast", "crf=23"),
            (QualityTier::Medium, "preset=superfast", "crf=21"),
            (QualityTier::High, "preset=faster", "crf=18"),
        ] {
            let args = run_linux("sane-safe", tier);
            assert_eq!(win_flag_value(&args, "-c"), Some("libx264"));
            assert_eq!(win_flag_value(&args, "-b"), Some("0"));
            assert!(args.contains(&preset.to_string()));
            assert!(args.contains(&crf.to_string()));
            assert!(args.contains(&"tune=zerolatency".to_string()));
        }

        // AmdHardware & IntelHardware (VA-API): solo varía qp (22/20/18), mantiene -b 0, sin preset
        for profile in ["amd-hardware", "intel-hardware"] {
            for (tier, qp) in [
                (QualityTier::Low, "qp=22"),
                (QualityTier::Medium, "qp=20"),
                (QualityTier::High, "qp=18"),
            ] {
                let args = run_linux(profile, tier);
                assert_eq!(win_flag_value(&args, "-c"), Some("h264_vaapi"));
                assert_eq!(win_flag_value(&args, "-b"), Some("0"));
                assert!(args.contains(&qp.to_string()));
                assert!(
                    !args.iter().any(|a| a.starts_with("preset=")),
                    "VA-API no debe incluir preset en {}",
                    profile
                );
            }
        }

        // NvidiaHardware (Linux NVENC): Low=p3/23, Medium=p4/21, High=p5/19
        for (tier, preset, cq) in [
            (QualityTier::Low, "preset=p3", "cq=23"),
            (QualityTier::Medium, "preset=p4", "cq=21"),
            (QualityTier::High, "preset=p5", "cq=19"),
        ] {
            let args = run_linux("nvidia-hardware", tier);
            assert_eq!(win_flag_value(&args, "-c"), Some("h264_nvenc"));
            assert_eq!(win_flag_value(&args, "-b"), Some("0"));
            assert!(args.contains(&preset.to_string()));
            assert!(args.contains(&cq.to_string()));
        }

        // Custom sigue usando config.video.preset y crf
        let mut custom_cfg = Config::default();
        custom_cfg.profile.active_profile = "custom".to_string();
        custom_cfg.profile.quality_tier = QualityTier::High;
        custom_cfg.video.codec = "libx264".to_string();
        custom_cfg.video.preset = "veryfast".to_string();
        custom_cfg.video.crf = 25;
        let mut custom_cmd = Command::new("wf-recorder");
        apply_profile_args(&mut custom_cmd, &custom_cfg);
        let custom_args = win_args(&custom_cmd);
        assert!(custom_args.contains(&"preset=veryfast".to_string()));
        assert!(custom_args.contains(&"crf=25".to_string()));
    }

    #[test]
    fn test_win_ram_buffer_max_mb_scaling_and_ts_sync_validation() {
        assert_eq!(win_ram_buffer_max_mb(60, 500, QualityTier::Low), 720);
        assert_eq!(win_ram_buffer_max_mb(60, 500, QualityTier::Medium), 960);
        assert_eq!(win_ram_buffer_max_mb(60, 500, QualityTier::High), 1320);
        assert_eq!(win_ram_buffer_max_mb(10, 500, QualityTier::Low), 500);

        // Resincronización MPEG-TS segura: ignora 0x47 falso en basura/payload cuando pos+188 != 0x47
        let mut buf = vec![0x00, 0x47, 0x99, 0x11]; // 0x47 falso en pos=1
        let mut pkt1 = vec![0xAAu8; 188];
        pkt1[0] = 0x47;
        pkt1[7] = 0x47; // 0x47 falso dentro del payload
        let mut pkt2 = vec![0xBBu8; 188];
        pkt2[0] = 0x47;
        buf.extend_from_slice(&pkt1);
        buf.extend_from_slice(&pkt2);

        assert_eq!(find_ts_sync(&buf), Some(4));

        // Si no hay suficientes bytes para pos+188, acepta el primer 0x47 candidato
        assert_eq!(find_ts_sync(&[0x00, 0x47, 0x12]), Some(1));

        // Si hay >= 189 bytes pero pos+188 != 0x47 y no hay otro 0x47, retorna None
        let mut invalid = vec![0u8; 200];
        invalid[0] = 0x47;
        assert_eq!(find_ts_sync(&invalid), None);
    }
}
