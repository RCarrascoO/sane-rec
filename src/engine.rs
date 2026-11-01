use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::config::Config;
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
    start_lock: Mutex<()>,
    is_saving: Arc<AtomicBool>,
    #[allow(dead_code)]
    win_ram_buffer: Arc<Mutex<std::collections::VecDeque<(Instant, Vec<u8>)>>>,
    #[allow(dead_code)]
    win_buffer_running: Arc<AtomicBool>,
    #[allow(dead_code)]
    win_replay_audio_stop: Mutex<Option<Arc<AtomicBool>>>,
    #[allow(dead_code)]
    win_continuous_audio_stop: Mutex<Option<Arc<AtomicBool>>>,
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

        // Si `/dev/shm/sane-rec` no existe, intentamos crear un symlink hacia `buffer_dir`
        let alias = PathBuf::from("/dev/shm/sane-rec");
        if !alias.exists() && alias != buffer_dir {
            #[cfg(unix)]
            let _ = std::os::unix::fs::symlink(&buffer_dir, &alias);
        }

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
            start_lock: Mutex::new(()),
            is_saving: Arc::new(AtomicBool::new(false)),
            win_ram_buffer: Arc::new(Mutex::new(std::collections::VecDeque::new())),
            win_buffer_running: Arc::new(AtomicBool::new(false)),
            win_replay_audio_stop: Mutex::new(None),
            win_continuous_audio_stop: Mutex::new(None),
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

        // Orden cronológico (más antiguo primero)
        segments.sort_by(|a, b| match (a.1, b.1) {
            (Some(ta), Some(tb)) => ta.cmp(&tb),
            _ => a.0.cmp(&b.0),
        });

        // 1. Podar si excede la duración requerida + 3 segundos de margen
        let max_keep = (duration_secs + 3) as usize;
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
        Self::apply_sink_input_volume("sane_rec_system", volume);
    }

    /// Actualiza el volumen del micrófono exclusivamente en su canal interno de grabación (sane_rec_mic).
    /// NO altera la ganancia del micrófono físico del sistema operativo.
    pub fn set_volume_mic(&self, volume: f32) {
        {
            let mut cfg = self.config.lock().unwrap();
            cfg.audio.volume_mic = volume;
        }
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

        // Segmenter en RAM con microsegmentos de 1 segundo e intervalo keyint exacto de 1 segundo:
        let wrap_n = config.replay.duration_secs + 10;
        let seg_pattern = format!("{}/seg_%04d.mp4", self.buffer_dir.display());

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

    /// Inicia el proceso de grabación en búfer rodante en RAM con `wf-recorder` en Linux.
    #[cfg(target_os = "linux")]
    pub fn start_replay(&self) -> Result<()> {
        let _start_guard = self.start_lock.lock().unwrap();
        if self.is_running() {
            return Err(EngineError::AlreadyRunning);
        }

        // Limpieza preventiva del buffer
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
        let mut child = cmd.spawn()?;

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
                child = fallback_cmd.spawn()?;
            } else {
                return Err(EngineError::Custom("wf-recorder falló al iniciar el búfer de repetición. Revisa ~/.config/sane-rec/crash.log".into()));
            }
        }

        *self.replay_process.lock().unwrap() = Some(child);
        *self.start_time.lock().unwrap() = Some(Instant::now());
        self.pruner_running.store(true, Ordering::SeqCst);

        // Lanza hilo podador activo para garantizar límites estrictos de RAM y duración
        let pruner_flag = self.pruner_running.clone();
        let pruner_buf = self.buffer_dir.clone();
        let pruner_cfg = self.config.clone();
        let is_saving_flag = self.is_saving.clone();
        std::thread::Builder::new()
            .name("sane-rec-ram-pruner".to_string())
            .spawn(move || {
                while pruner_flag.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(1000));
                    if !pruner_flag.load(Ordering::SeqCst) {
                        break;
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
        let mut guard = self.replay_process.lock().unwrap();
        if let Some(mut child) = guard.take() {
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

        let mut config = self.config();
        let active_profile = config
            .profile
            .active_profile
            .parse::<EncoderProfile>()
            .unwrap_or(EncoderProfile::SaneSafe);

        let (mut audio_port, mut audio_stop) = match start_windows_audio_feeder(&config) {
            Ok((port, stop)) => (Some(port), Some(stop)),
            Err(e) => {
                eprintln!("Aviso: No se pudo iniciar el feeder de audio WASAPI: {}", e);
                (None, None)
            }
        };

        let build_replay_cmd = |cfg: &Config, port: Option<u16>| -> Command {
            let mut cmd = new_hidden_command(resolve_binary("ffmpeg"));
            cmd.arg("-y");

            apply_windows_input_args(&mut cmd, cfg, port);
            apply_windows_profile_args(&mut cmd, cfg, port.is_some());

            let fps = cfg.video.fps.max(15);
            let fps_str = fps.to_string();

            cmd.args([
                "-g",
                &fps_str,
                "-keyint_min",
                &fps_str,
                "-f",
                "mpegts",
                "-mpegts_flags",
                "resend_headers",
                "pipe:1",
            ]);

            cmd.stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            cmd
        };

        let mut cmd = build_replay_cmd(&config, audio_port);
        let mut child = cmd.spawn()?;

        // Verificación de salud inicial (350ms) para detectar fallos de codificador GPU o ventana inválida
        std::thread::sleep(Duration::from_millis(350));
        let failed = match child.try_wait() {
            Ok(Some(_)) => true,
            Err(_) => true,
            Ok(None) => false,
        };

        if failed {
            let code = child.try_wait().ok().flatten().and_then(|s| s.code());
            crate::crash_handler::log_engine_failure("Inicio del Búfer de Repetición (Windows)", code);

            if let Some(stop) = audio_stop.take() {
                stop.store(true, Ordering::SeqCst);
            }

            if active_profile != EncoderProfile::SaneSafe
                || config.video.target.starts_with("window:")
                || config.video.target.starts_with("title=")
            {
                config.profile.active_profile = "sane-safe".to_string();
                config.video.codec = "libx264".to_string();
                if config.video.target.starts_with("window:")
                    || config.video.target.starts_with("title=")
                {
                    config.video.target = "desktop".to_string();
                }
                let _ = config.save();
                *self.config.lock().unwrap() = config.clone();

                let (retry_port, retry_stop) = match start_windows_audio_feeder(&config) {
                    Ok((port, stop)) => (Some(port), Some(stop)),
                    Err(_) => (None, None),
                };
                audio_port = retry_port;
                audio_stop = retry_stop;

                let mut fallback_cmd = build_replay_cmd(&config, audio_port);
                child = fallback_cmd.spawn()?;

                std::thread::sleep(Duration::from_millis(350));
                if let Ok(Some(status)) = child.try_wait() {
                    if let Some(stop) = audio_stop.take() {
                        stop.store(true, Ordering::SeqCst);
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

        *self.win_replay_audio_stop.lock().unwrap() = audio_stop;

        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| EngineError::Custom("Fallo al capturar stdout de ffmpeg".into()))?;

        *self.replay_process.lock().unwrap() = Some(child);
        *self.start_time.lock().unwrap() = Some(Instant::now());
        self.win_buffer_running.store(true, Ordering::SeqCst);

        let ram_buf = Arc::clone(&self.win_ram_buffer);
        let running_flag = Arc::clone(&self.win_buffer_running);
        let dur_secs = config.replay.duration_secs;
        let max_bytes = (config.replay.max_memory_mb as usize) * 1024 * 1024;

        std::thread::Builder::new()
            .name("sane-rec-win-ram-buffer".into())
            .spawn(move || {
                use std::io::Read;
                let mut buffer = vec![0u8; 65536];
                while running_flag.load(Ordering::Relaxed) {
                    match stdout.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(n) => {
                            let now = Instant::now();
                            let chunk = buffer[..n].to_vec();
                            if let Ok(mut q) = ram_buf.lock() {
                                q.push_back((now, chunk));

                                let cutoff = Duration::from_secs((dur_secs + 3) as u64);
                                while let Some(front) = q.front() {
                                    if now.duration_since(front.0) > cutoff {
                                        q.pop_front();
                                    } else {
                                        break;
                                    }
                                }

                                let mut total_len: usize = q.iter().map(|(_, c)| c.len()).sum();
                                while total_len > max_bytes && q.len() > 10 {
                                    if let Some(removed) = q.pop_front() {
                                        total_len = total_len.saturating_sub(removed.1.len());
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
        self.win_buffer_running.store(false, Ordering::SeqCst);
        if let Some(stop_flag) = self.win_replay_audio_stop.lock().unwrap().take() {
            stop_flag.store(true, Ordering::SeqCst);
        }
        let mut guard = self.replay_process.lock().unwrap();
        if let Some(mut child) = guard.take() {
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
            self.stop_replay()?;
            Ok(false)
        } else {
            self.start_replay()?;
            Ok(true)
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
        let mut child = cmd.spawn()?;

        // Manejo seguro de inicio y captura de fallos
        std::thread::sleep(std::time::Duration::from_millis(400));
        let failed = match child.try_wait() {
            Ok(Some(status)) => !status.success(),
            Err(_) => true,
            Ok(None) => false,
        };

        if failed {
            let code = child.try_wait().ok().flatten().and_then(|s| s.code());
            crate::crash_handler::log_engine_failure("Inicio de Grabación Continua", code);

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
                let mut fallback_cmd = self.build_continuous_command(&fallback_config, &output_path);
                child = fallback_cmd.spawn()?;
            } else {
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
        let mut guard = self.continuous_process.lock().unwrap();
        let mut child = guard.take().ok_or(EngineError::NotRunning)?;
        let output_path = self
            .continuous_output
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| PathBuf::from("recording.mp4"));
        *self.continuous_start_time.lock().unwrap() = None;

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

        // Si el modo "doble clip" está activo, genera en segundo plano la copia optimizada para Discord
        let current_cfg = Config::load().unwrap_or_else(|_| self.config());
        if current_cfg.storage.dual_clip {
            let target_mb = current_cfg.storage.discord_target_mb;
            crate::compressor::spawn_discord_compression(output_path.clone(), target_mb);
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

        let mut config = self.config();
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

        let (mut audio_port, mut audio_stop) = match start_windows_audio_feeder(&config) {
            Ok((port, stop)) => (Some(port), Some(stop)),
            Err(e) => {
                eprintln!("Aviso: No se pudo iniciar el feeder de audio WASAPI: {}", e);
                (None, None)
            }
        };

        let build_cont_cmd = |cfg: &Config, port: Option<u16>, out: &Path| -> Command {
            let mut cmd = new_hidden_command(resolve_binary("ffmpeg"));
            cmd.arg("-y");

            apply_windows_input_args(&mut cmd, cfg, port);
            apply_windows_profile_args(&mut cmd, cfg, port.is_some());

            cmd.args(["-movflags", "+faststart"]);
            cmd.arg(out);

            cmd.stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            cmd
        };

        let mut cmd = build_cont_cmd(&config, audio_port, &output_path);
        let mut child = cmd.spawn()?;

        // Verificación de salud inicial (350ms) y fallback automático a SaneSafe CPU
        std::thread::sleep(Duration::from_millis(350));
        let failed = match child.try_wait() {
            Ok(Some(_)) => true,
            Err(_) => true,
            Ok(None) => false,
        };

        if failed {
            let code = child.try_wait().ok().flatten().and_then(|s| s.code());
            crate::crash_handler::log_engine_failure("Inicio de Grabación Continua (Windows)", code);

            if let Some(stop) = audio_stop.take() {
                stop.store(true, Ordering::SeqCst);
            }

            if active_profile != EncoderProfile::SaneSafe
                || config.video.target.starts_with("window:")
                || config.video.target.starts_with("title=")
            {
                config.profile.active_profile = "sane-safe".to_string();
                config.video.codec = "libx264".to_string();
                if config.video.target.starts_with("window:")
                    || config.video.target.starts_with("title=")
                {
                    config.video.target = "desktop".to_string();
                }
                let _ = config.save();
                *self.config.lock().unwrap() = config.clone();

                let (retry_port, retry_stop) = match start_windows_audio_feeder(&config) {
                    Ok((port, stop)) => (Some(port), Some(stop)),
                    Err(_) => (None, None),
                };
                audio_port = retry_port;
                audio_stop = retry_stop;

                let mut fallback_cmd = build_cont_cmd(&config, audio_port, &output_path);
                child = fallback_cmd.spawn()?;

                std::thread::sleep(Duration::from_millis(350));
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
        let mut guard = self.continuous_process.lock().unwrap();
        let mut child = guard.take().ok_or(EngineError::NotRunning)?;
        let output_path = self
            .continuous_output
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| PathBuf::from("recording.mp4"));
        *self.continuous_start_time.lock().unwrap() = None;

        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(b"q\n");
            let _ = stdin.flush();
        }
        let _ = child.wait();

        let _ = notify_rust::Notification::new()
            .appname("sane-rec")
            .summary("⏹️ Grabación Finalizada")
            .body(&format!("Vídeo guardado en {}", output_path.display()))
            .icon("video-x-generic")
            .show();

        play_feedback_sound("complete");

        let current_cfg = Config::load().unwrap_or_else(|_| self.config());
        if current_cfg.storage.dual_clip {
            let target_mb = current_cfg.storage.discord_target_mb;
            crate::compressor::spawn_discord_compression(output_path.clone(), target_mb);
        }

        Ok(output_path)
    }

    /// Alterna el estado de la grabación continua.
    pub fn toggle_continuous_recording(&self) -> Result<bool> {
        if self.is_recording() {
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
        // 1. Escanea los segmentos seg_*.mp4 en el directorio de búfer con tamaño > 0
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

        // 2. Crea el archivo de lista único con líneas "file '<abs_path>'"
        let unique_id = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let list_file = self.buffer_dir.join(format!("concat_list_{}.txt", unique_id));
        let staging_file = self
            .buffer_dir
            .join(format!(".staging_concat_{}.{}", unique_id, self.config().storage.format));

        let mut list_content = String::new();
        for seg in &segments {
            list_content.push_str(&format!("file '{}'\n", seg.display()));
        }
        if let Err(e) = std::fs::write(&list_file, list_content) {
            self.is_saving.store(false, Ordering::SeqCst);
            return Err(EngineError::Io(e));
        }

        // 3. Determina el destino final
        let config = self.config();
        if let Err(e) = config.ensure_output_dir() {
            let _ = std::fs::remove_file(&list_file);
            self.is_saving.store(false, Ordering::SeqCst);
            return Err(EngineError::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())));
        }

        let timestamp = chrono::Local::now()
            .format(&config.storage.filename_format)
            .to_string();
        let filename = format!("{}.{}", timestamp, config.storage.format);
        let output_path = config.storage.output_dir.join(filename);

        // 4. Concatena los segmentos en RAM hacia el archivo staging
        let concat_res = new_hidden_command(resolve_binary("ffmpeg"))
            .args(["-y", "-f", "concat", "-safe", "0", "-i"])
            .arg(&list_file)
            .args(["-c", "copy"])
            .arg(&staging_file)
            .output();

        let _ = std::fs::remove_file(&list_file);

        let concat_out = match concat_res {
            Ok(o) => o,
            Err(e) => {
                let _ = std::fs::remove_file(&staging_file);
                self.is_saving.store(false, Ordering::SeqCst);
                return Err(EngineError::Io(e));
            }
        };

        if !concat_out.status.success() {
            let err_msg = String::from_utf8_lossy(&concat_out.stderr);
            let _ = std::fs::remove_file(&staging_file);
            self.is_saving.store(false, Ordering::SeqCst);
            crate::crash_handler::write_crash_report("Error de concatenación en ffmpeg al guardar replay", &err_msg);
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

        // Si el modo "doble clip" está activo, genera en segundo plano la copia optimizada para Discord
        let current_cfg = Config::load().unwrap_or_else(|_| self.config());
        if current_cfg.storage.dual_clip {
            let target_mb = current_cfg.storage.discord_target_mb;
            crate::compressor::spawn_discord_compression(output_path.clone(), target_mb);
        }

        Ok(output_path)
    }

    /// Guarda un replay del búfer actual tomando los chunks en RAM física y escribiéndolos instantáneamente a MP4 sin recodificar en Windows.
    #[cfg(target_os = "windows")]
    pub fn save_clip(&self) -> Result<PathBuf> {
        let chunks = {
            let q = self.win_ram_buffer.lock().unwrap();
            if q.is_empty() {
                return Err(EngineError::NoSegments);
            }
            let dur = self.config().replay.duration_secs;
            let now = Instant::now();
            let cutoff = Duration::from_secs((dur + 1) as u64);
            let list: Vec<Vec<u8>> = q
                .iter()
                .filter(|(t, _)| now.duration_since(*t) <= cutoff)
                .map(|(_, bytes)| bytes.clone())
                .collect();
            list
        };

        if chunks.is_empty() {
            return Err(EngineError::NoSegments);
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

        let mut cmd = new_hidden_command(resolve_binary("ffmpeg"));
        cmd.args([
            "-y",
            "-f",
            "mpegts",
            "-i",
            "pipe:0",
            "-c",
            "copy",
            "-movflags",
            "+faststart",
        ]);
        cmd.arg(&output_path);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let mut child = cmd.spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            for chunk in chunks {
                let _ = stdin.write_all(&chunk);
            }
            let _ = stdin.flush();
        }

        let status = child.wait()?;
        self.is_saving.store(false, Ordering::SeqCst);

        if !status.success() {
            return Err(EngineError::FfmpegError(
                "Error al guardar MP4 desde el buffer MPEG-TS en RAM".into(),
            ));
        }

        #[cfg(not(target_os = "windows"))]
        let _ = notify_rust::Notification::new()
            .appname("sane-rec")
            .summary("🎬 Replay Guardado")
            .body(&format!("Clip guardado en {}", output_path.display()))
            .icon("video-x-generic")
            .show();

        play_feedback_sound("screen-capture");

        let current_cfg = Config::load().unwrap_or_else(|_| self.config());
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

    /// Escanea los segmentos válidos presentes en la RAM (seg_*.mp4 o segment_*.ts con tamaño > 0).
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
                            && (ext == "mp4" || ext == "ts")
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

        // Si no se encontraron y `/dev/shm/sane-rec` es una ruta distinta y existe, comprobamos allí
        let fallback = Path::new("/dev/shm/sane-rec");
        if segments.is_empty() && fallback.exists() && fallback != self.buffer_dir {
            if let Ok(entries) = std::fs::read_dir(fallback) {
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
                            && (ext == "mp4" || ext == "ts")
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
        let fallback = Path::new("/dev/shm/sane-rec");
        if fallback.exists() && fallback != self.buffer_dir {
            Self::clean_directory(fallback);
        }
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
                    if name.starts_with("seg_")
                        || name.starts_with("segment_")
                        || name.starts_with("concat_")
                        || name == "concat_list.txt"
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
        self.pruner_running.store(false, Ordering::SeqCst);
        self.win_buffer_running.store(false, Ordering::SeqCst);

        #[cfg(target_os = "linux")]
        {
            if let Ok(mut guard) = self.replay_process.lock() {
                if let Some(mut child) = guard.take() {
                    unsafe {
                        libc::kill(child.id() as i32, libc::SIGINT);
                    }
                    let _ = child.wait();
                }
            }
            if let Ok(mut guard) = self.continuous_process.lock() {
                if let Some(mut child) = guard.take() {
                    unsafe {
                        libc::kill(child.id() as i32, libc::SIGINT);
                    }
                    let _ = child.wait();
                }
            }
            self.cleanup_audio_modules();
            let _ = self.clean_buffer();
        }

        #[cfg(target_os = "windows")]
        {
            if let Some(stop) = self.win_replay_audio_stop.lock().unwrap().take() {
                stop.store(true, Ordering::SeqCst);
            }
            if let Some(stop) = self.win_continuous_audio_stop.lock().unwrap().take() {
                stop.store(true, Ordering::SeqCst);
            }

            if let Ok(mut guard) = self.replay_process.lock() {
                if let Some(mut child) = guard.take() {
                    if let Some(mut stdin) = child.stdin.take() {
                        use std::io::Write;
                        let _ = stdin.write_all(b"q\n");
                    }
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
            if let Ok(mut guard) = self.continuous_process.lock() {
                if let Some(mut child) = guard.take() {
                    if let Some(mut stdin) = child.stdin.take() {
                        use std::io::Write;
                        let _ = stdin.write_all(b"q\n");
                    }
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
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
        }
    }

    pub fn push_sample(&mut self, s: f32, out: &Arc<Mutex<std::collections::VecDeque<f32>>>) {
        if self.curr_channel == 0 {
            self.curr_frame_left = s;
            if self.channels == 1 {
                self.curr_frame_right = s;
                self.feed_frame(out);
            }
        } else if self.curr_channel == 1 {
            self.curr_frame_right = s;
            if self.channels == 2 {
                self.feed_frame(out);
            }
        } else if self.curr_channel == self.channels - 1 {
            self.feed_frame(out);
        }
        self.curr_channel = (self.curr_channel + 1) % self.channels;
    }

    fn feed_frame(&mut self, out: &Arc<Mutex<std::collections::VecDeque<f32>>>) {
        let next_left = self.curr_frame_left;
        let next_right = self.curr_frame_right;

        if (self.from_rate - self.to_rate).abs() < 1.0 {
            let mut q = out.lock().unwrap();
            q.push_back(next_left * self.volume);
            q.push_back(next_right * self.volume);
            if q.len() > 192000 {
                q.drain(..96000);
            }
            return;
        }

        let mut q = out.lock().unwrap();
        while self.pos < 1.0 {
            let t = self.pos as f32;
            let l = (1.0 - t) * self.prev_left + t * next_left;
            let r = (1.0 - t) * self.prev_right + t * next_right;
            q.push_back(l * self.volume);
            q.push_back(r * self.volume);
            self.pos += self.ratio;
        }
        self.pos -= 1.0;
        self.prev_left = next_left;
        self.prev_right = next_right;

        if q.len() > 192000 {
            q.drain(..96000);
        }
    }

    pub fn process_f32(&mut self, data: &[f32], out: &Arc<Mutex<std::collections::VecDeque<f32>>>) {
        for &s in data {
            self.push_sample(s, out);
        }
    }

    pub fn process_i16(&mut self, data: &[i16], out: &Arc<Mutex<std::collections::VecDeque<f32>>>) {
        for &s in data {
            let f = s as f32 / 32768.0;
            self.push_sample(f, out);
        }
    }

    pub fn process_u16(&mut self, data: &[u16], out: &Arc<Mutex<std::collections::VecDeque<f32>>>) {
        for &s in data {
            let f = (s as f32 - 32768.0) / 32768.0;
            self.push_sample(f, out);
        }
    }
}

/// Inicia el feeder de audio WASAPI Loopback y Micrófono con cpal transmitiendo PCM crudo estéreo a 48 kHz vía TCP local.
#[cfg(target_os = "windows")]
fn start_windows_audio_feeder(config: &Config) -> std::io::Result<(u16, Arc<AtomicBool>)> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_flag_clone = Arc::clone(&stop_flag);

    let system_buffer = Arc::new(Mutex::new(std::collections::VecDeque::<f32>::new()));
    let mic_buffer = Arc::new(Mutex::new(std::collections::VecDeque::<f32>::new()));
    let config_clone = config.clone();

    // Feeder Thread continuo hacia FFmpeg a 48000 Hz estéreo cada 20ms
    std::thread::Builder::new()
        .name("sane-rec-win-audio-feeder".into())
        .spawn(move || {
            use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

            let host = cpal::default_host();
            let mut streams: Vec<cpal::Stream> = Vec::new();

            // 1. WASAPI Loopback (Audio del Sistema)
            if config_clone.audio.capture_desktop {
        if let Some(device) = host.default_output_device() {
            if let Ok(supported_config) = device.default_output_config() {
                let sample_rate = supported_config.sample_rate().0;
                let channels = supported_config.channels() as usize;
                let sample_format = supported_config.sample_format();
                let stream_config: cpal::StreamConfig = supported_config.into();

                let sys_buf = Arc::clone(&system_buffer);
                let volume = config_clone.audio.volume_system;
                let resampler = Arc::new(Mutex::new(WindowsAudioResampler::new(
                    sample_rate,
                    channels,
                    volume,
                )));

                let r_clone = Arc::clone(&resampler);
                let buf_clone = Arc::clone(&sys_buf);
                let err_fn = |err| eprintln!("Aviso en stream WASAPI Loopback: {}", err);

                let stream_res = match sample_format {
                    cpal::SampleFormat::F32 => device.build_input_stream(
                        &stream_config,
                        move |data: &[f32], _| {
                            if let Ok(mut r) = r_clone.lock() {
                                r.process_f32(data, &buf_clone);
                            }
                        },
                        err_fn,
                        None,
                    ),
                    cpal::SampleFormat::I16 => device.build_input_stream(
                        &stream_config,
                        move |data: &[i16], _| {
                            if let Ok(mut r) = r_clone.lock() {
                                r.process_i16(data, &buf_clone);
                            }
                        },
                        err_fn,
                        None,
                    ),
                    cpal::SampleFormat::U16 => device.build_input_stream(
                        &stream_config,
                        move |data: &[u16], _| {
                            if let Ok(mut r) = r_clone.lock() {
                                r.process_u16(data, &buf_clone);
                            }
                        },
                        err_fn,
                        None,
                    ),
                    _ => Err(cpal::BuildStreamError::StreamConfigNotSupported),
                };

                if let Ok(stream) = stream_res {
                    if stream.play().is_ok() {
                        streams.push(stream);
                    }
                }
            }
        }
    }

    // 2. WASAPI Input (Micrófono)
    if config_clone.audio.capture_mic {
        if let Some(device) = host.default_input_device() {
            if let Ok(supported_config) = device.default_input_config() {
                let sample_rate = supported_config.sample_rate().0;
                let channels = supported_config.channels() as usize;
                let sample_format = supported_config.sample_format();
                let stream_config: cpal::StreamConfig = supported_config.into();

                let mic_buf = Arc::clone(&mic_buffer);
                let volume = config_clone.audio.volume_mic;
                let resampler = Arc::new(Mutex::new(WindowsAudioResampler::new(
                    sample_rate,
                    channels,
                    volume,
                )));

                let r_clone = Arc::clone(&resampler);
                let buf_clone = Arc::clone(&mic_buf);
                let err_fn = |err| eprintln!("Aviso en stream WASAPI Mic: {}", err);

                let stream_res = match sample_format {
                    cpal::SampleFormat::F32 => device.build_input_stream(
                        &stream_config,
                        move |data: &[f32], _| {
                            if let Ok(mut r) = r_clone.lock() {
                                r.process_f32(data, &buf_clone);
                            }
                        },
                        err_fn,
                        None,
                    ),
                    cpal::SampleFormat::I16 => device.build_input_stream(
                        &stream_config,
                        move |data: &[i16], _| {
                            if let Ok(mut r) = r_clone.lock() {
                                r.process_i16(data, &buf_clone);
                            }
                        },
                        err_fn,
                        None,
                    ),
                    cpal::SampleFormat::U16 => device.build_input_stream(
                        &stream_config,
                        move |data: &[u16], _| {
                            if let Ok(mut r) = r_clone.lock() {
                                r.process_u16(data, &buf_clone);
                            }
                        },
                        err_fn,
                        None,
                    ),
                    _ => Err(cpal::BuildStreamError::StreamConfigNotSupported),
                };

                if let Ok(stream) = stream_res {
                    if stream.play().is_ok() {
                        streams.push(stream);
                    }
                }
            }
        }
    }

            let _ = listener.set_nonblocking(false);
            let (mut stream, _) = match listener.accept() {
                Ok(conn) => conn,
                Err(_) => return,
            };
            let _ = stream.set_nodelay(true);

            let mut next_tick = Instant::now() + Duration::from_millis(20);
            let mut send_buf = vec![0u8; 3840]; // 960 frames * 2 canales * 2 bytes = 3840 bytes

            while !stop_flag_clone.load(Ordering::Relaxed) {
                {
                    let mut sys_q = system_buffer.lock().unwrap();
                    let mut mic_q = mic_buffer.lock().unwrap();

                    let mut offset = 0;
                    for _ in 0..960 {
                        let sys_l = sys_q.pop_front().unwrap_or(0.0);
                        let sys_r = sys_q.pop_front().unwrap_or(0.0);
                        let mic_l = mic_q.pop_front().unwrap_or(0.0);
                        let mic_r = mic_q.pop_front().unwrap_or(0.0);

                        let mixed_l = (sys_l + mic_l).clamp(-1.0, 1.0);
                        let mixed_r = (sys_r + mic_r).clamp(-1.0, 1.0);

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

                let now = Instant::now();
                if next_tick > now {
                    std::thread::sleep(next_tick - now);
                }
                next_tick += Duration::from_millis(20);
                if next_tick < Instant::now() {
                    next_tick = Instant::now() + Duration::from_millis(20);
                }
            }
        })
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

    Ok((port, stop_flag))
}

/// Aplica los argumentos de entrada y captura de pantalla y audio en Windows (gdigrab + audio TCP).
#[cfg(target_os = "windows")]
fn apply_windows_input_args(cmd: &mut Command, config: &Config, audio_port: Option<u16>) {
    let fps_str = config.video.fps.to_string();
    let target = config.video.target.trim();

    let apply_desktop_capture = |cmd: &mut Command, target_name: &str| {
        let outputs = crate::discovery::get_connected_outputs();
        let matched_output = outputs
            .iter()
            .find(|o| o.name.eq_ignore_ascii_case(target_name))
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
                return;
            }
        }

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
    };

    // Input 0: Video
    cmd.args(["-thread_queue_size", "512"]);
    if target.starts_with("title=") || target.starts_with("window:") {
        let win_title = target
            .strip_prefix("title=")
            .or_else(|| target.strip_prefix("window:"))
            .unwrap_or(target)
            .trim();
        let lower_title = win_title.to_lowercase();
        // Capturar mediante el compositor DWM (`desktop`) si la ventana es `windows-app` (legacy)
        // o un juego OpenGL/LWJGL como Minecraft Java, evitando frame congelado en el menú ESC.
        if win_title.is_empty()
            || win_title == "windows-app"
            || lower_title.contains("minecraft")
            || lower_title.contains("javaw")
        {
            apply_desktop_capture(cmd, target);
        } else {
            cmd.args([
                "-f",
                "gdigrab",
                "-framerate",
                &fps_str,
                "-draw_mouse",
                "1",
                "-i",
                &format!("title={}", win_title),
            ]);
        }
    } else {
        apply_desktop_capture(cmd, target);
    }

    // Input 1: Audio PCM crudo continuo s16le a 48000 Hz estéreo vía TCP local
    if let Some(port) = audio_port {
        let tcp_url = format!("tcp://127.0.0.1:{}", port);
        cmd.args([
            "-thread_queue_size",
            "1024",
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
}

/// Configura los parámetros de codificación de hardware y CPU en Windows según el perfil activo.
#[cfg(target_os = "windows")]
fn apply_windows_profile_args(cmd: &mut Command, config: &Config, has_audio: bool) {
    let profile = config
        .profile
        .active_profile
        .parse::<EncoderProfile>()
        .unwrap_or(EncoderProfile::SaneSafe);

    match profile {
        EncoderProfile::SaneSafe => {
            cmd.args([
                "-c:v",
                "libx264",
                "-preset",
                "superfast",
                "-crf",
                "23",
                "-pix_fmt",
                "yuv420p",
            ]);
        }
        EncoderProfile::AmdHardware => {
            cmd.args([
                "-c:v",
                "h264_amf",
                "-quality",
                "speed",
                "-rc",
                "cqp",
                "-qp_i",
                "22",
                "-qp_p",
                "22",
                "-pix_fmt",
                "yuv420p",
            ]);
        }
        EncoderProfile::NvidiaHardware => {
            cmd.args([
                "-c:v",
                "h264_nvenc",
                "-preset",
                "p4",
                "-rc",
                "constqp",
                "-qp",
                "23",
                "-pix_fmt",
                "yuv420p",
            ]);
        }
        EncoderProfile::IntelHardware => {
            cmd.args([
                "-c:v",
                "h264_qsv",
                "-preset",
                "veryfast",
                "-global_quality",
                "23",
                "-pix_fmt",
                "yuv420p",
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
                    _ => "p3",
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
                    "-rc",
                    "constqp",
                    "-qp",
                    &crf_str,
                    "-bf",
                    "0",
                    "-pix_fmt",
                    "yuv420p",
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
                cmd.args([
                    "-c:v",
                    win_codec,
                    "-quality",
                    amf_quality,
                    "-rc",
                    "cqp",
                    "-qp_i",
                    &crf_str,
                    "-qp_p",
                    &crf_str,
                    "-pix_fmt",
                    "yuv420p",
                ]);
            } else if codec.contains("qsv") {
                let qsv_preset = match config.video.preset.as_str() {
                    "ultrafast" | "veryfast" => "veryfast",
                    "superfast" | "faster" => "faster",
                    "veryfast" | "fast" => "fast",
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
                    "yuv420p",
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
                    "-bf",
                    "0",
                    "-pix_fmt",
                    "yuv420p",
                    "-threads",
                    "2",
                ]);
            }
        }
    }

    if has_audio {
        cmd.args([
            "-c:a", "aac",
            "-b:a", "192k",
            "-ar", "48000",
            "-ac", "2",
            "-map", "0:v:0",
            "-map", "1:a:0",
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
    } else if t.is_empty() || t == "portal" {
        cmd.args(["-o", "DVI-D-1"]);
    } else {
        cmd.args(["-o", t]);
    }
}

/// Aplica los argumentos de perfil de encoder al comando wf-recorder.
fn apply_profile_args(cmd: &mut Command, config: &Config) {
    let profile = config
        .profile
        .active_profile
        .parse::<EncoderProfile>()
        .unwrap_or(EncoderProfile::SaneSafe);

    match profile {
        EncoderProfile::SaneSafe => {
            cmd.args([
                "-c",
                "libx264",
                "-p",
                "preset=ultrafast",
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
            let qp_arg = format!("qp={}", config.video.crf);
            cmd.args(["-c", "h264_vaapi", "-b", "0", "-p", &qp_arg]);
        }
        EncoderProfile::NvidiaHardware => {
            let cq_arg = format!("cq={}", config.video.crf);
            cmd.args(["-c", "h264_nvenc", "-b", "0", "-p", "preset=p3", "-p", &cq_arg]);
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
            output_dir: PathBuf::from("/mnt/Files/Videos/Replays"),
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

    #[test]
    fn test_is_source_available_nonexistent() {
        assert!(!super::is_source_available("nonexistent_device_source_xyz_12345"));
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
}
