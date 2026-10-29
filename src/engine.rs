use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::Mutex;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::profiles::EncoderProfile;

pub const DEFAULT_SEGMENT_DURATION_SECS: u32 = 2;

/// Retorna la ruta por defecto al buffer circular en memoria RAM: `/dev/shm/sane-rec-<UID>/`.
pub fn default_buffer_dir() -> PathBuf {
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/dev/shm/sane-rec-{}", uid))
}

/// Estado actual del motor de grabación y repetición instantánea.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EngineStatus {
    pub is_running: bool,
    pub is_recording: bool,
    pub duration_secs: u32,
    pub current_buffer_secs: u32,
    pub ram_usage_mb: f64,
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
    config: Mutex<Config>,
    buffer_dir: PathBuf,
    replay_process: Mutex<Option<Child>>,
    continuous_process: Mutex<Option<Child>>,
    continuous_output: Mutex<Option<PathBuf>>,
    start_time: Mutex<Option<Instant>>,
    continuous_start_time: Mutex<Option<Instant>>,
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
            config: Mutex::new(config),
            buffer_dir,
            replay_process: Mutex::new(None),
            continuous_process: Mutex::new(None),
            continuous_output: Mutex::new(None),
            start_time: Mutex::new(None),
            continuous_start_time: Mutex::new(None),
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

    /// Inicia el proceso de grabación en búfer rodante en RAM con `wf-recorder`.
    pub fn start_replay(&self) -> Result<()> {
        if self.is_running() {
            return Err(EngineError::AlreadyRunning);
        }

        // Limpieza preventiva del buffer
        self.clean_buffer()?;
        let _ = std::fs::create_dir_all(&self.buffer_dir);

        let config = self.config();
        let mut cmd = Command::new("wf-recorder");
        cmd.arg("-y");

        // Configuración de destino (monitor o geometría)
        apply_target_args(&mut cmd, &config.video.target);

        // Audio del sistema si está activo
        apply_audio_args(&mut cmd, &config);

        // Perfil de codificación
        apply_profile_args(&mut cmd, &config);

        // Segmenter en RAM:
        // -m segment -p segment_time=2 -p segment_wrap=<N> -p reset_timestamps=1 -f /dev/shm/sane-rec-<UID>/seg_%02d.mp4
        // donde <N> = (duration_secs / 2) + 2
        let wrap_n = (config.replay.duration_secs / DEFAULT_SEGMENT_DURATION_SECS) + 2;
        let seg_pattern = format!("{}/seg_%02d.mp4", self.buffer_dir.display());

        cmd.args([
            "-m",
            "segment",
            "-p",
            "segment_time=2",
            "-p",
            &format!("segment_wrap={}", wrap_n),
            "-p",
            "reset_timestamps=1",
            "-f",
            &seg_pattern,
        ]);

        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        let child = cmd.spawn()?;

        *self.replay_process.lock().unwrap() = Some(child);
        *self.start_time.lock().unwrap() = Some(Instant::now());

        Ok(())
    }

    /// Detiene el proceso de repetición instantánea enviando SIGINT y limpia la RAM.
    pub fn stop_replay(&self) -> Result<()> {
        let mut guard = self.replay_process.lock().unwrap();
        if let Some(mut child) = guard.take() {
            unsafe {
                libc::kill(child.id() as i32, libc::SIGINT);
            }
            let _ = child.wait();
        }
        *self.start_time.lock().unwrap() = None;

        // Limpieza de segmentos en RAM
        let _ = self.clean_buffer();
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
                _ => {
                    *guard = None;
                    false
                }
            }
        } else {
            false
        }
    }

    /// Inicia la grabación continua directa al directorio de almacenamiento con `wf-recorder`.
    pub fn start_continuous_recording(&self) -> Result<PathBuf> {
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

        let mut cmd = Command::new("wf-recorder");
        cmd.arg("-y");

        // Destino de captura
        apply_target_args(&mut cmd, &config.video.target);

        // Audio del sistema
        apply_audio_args(&mut cmd, &config);

        // Perfil de codificación
        apply_profile_args(&mut cmd, &config);

        // Archivo de salida directo
        cmd.arg("-f").arg(&output_path);

        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        let child = cmd.spawn()?;

        *self.continuous_process.lock().unwrap() = Some(child);
        *self.continuous_output.lock().unwrap() = Some(output_path.clone());
        *self.continuous_start_time.lock().unwrap() = Some(Instant::now());

        let _ = notify_rust::Notification::new()
            .appname("sane-rec")
            .summary("🔴 Grabación Iniciada")
            .body(&format!("Grabando pantalla en {}", output_path.display()))
            .icon("media-record")
            .show();

        Ok(output_path)
    }

    /// Detiene la grabación continua enviando SIGINT, espera el cierre ordenado del archivo y notifica.
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

        let _ = notify_rust::Notification::new()
            .appname("sane-rec")
            .summary("⏹️ Grabación Finalizada")
            .body(&format!("Vídeo guardado en {}", output_path.display()))
            .icon("video-x-generic")
            .show();

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
                _ => {
                    *guard = None;
                    false
                }
            }
        } else {
            false
        }
    }

    /// Guarda un replay del búfer actual concatenando segmentos en RAM a alta velocidad con ffmpeg.
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

        // 2. Crea el archivo de lista concat_list.txt con líneas "file '<abs_path>'"
        let list_file = self.buffer_dir.join("concat_list.txt");
        let mut list_content = String::new();
        for seg in &segments {
            list_content.push_str(&format!("file '{}'\n", seg.display()));
        }
        std::fs::write(&list_file, list_content)?;

        // 3. Determina el destino final
        let config = self.config();
        config.ensure_output_dir().map_err(|e| {
            EngineError::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
        })?;

        let timestamp = chrono::Local::now()
            .format(&config.storage.filename_format)
            .to_string();
        let filename = format!("{}.{}", timestamp, config.storage.format);
        let output_path = config.storage.output_dir.join(filename);

        // 4. Concatena los segmentos usando ffmpeg con copia directa de streams
        let ffmpeg_res = Command::new("ffmpeg")
            .args(["-y", "-f", "concat", "-safe", "0", "-i"])
            .arg(&list_file)
            .args(["-c", "copy", "-movflags", "+faststart"])
            .arg(&output_path)
            .output();

        let _ = std::fs::remove_file(&list_file);

        let output = ffmpeg_res?;
        if !output.status.success() {
            let err_msg = String::from_utf8_lossy(&output.stderr);
            return Err(EngineError::FfmpegError(format!(
                "ffmpeg concat error: {}",
                err_msg
            )));
        }

        // 5. Lanza la notificación de escritorio
        let _ = notify_rust::Notification::new()
            .appname("sane-rec")
            .summary("🎬 Replay Guardado")
            .body(&format!("Clip guardado en {}", output_path.display()))
            .icon("video-x-generic")
            .show();

        Ok(output_path)
    }

    /// Retorna el estado actual del motor de grabación y memoria.
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

        EngineStatus {
            is_running,
            is_recording,
            duration_secs: config.replay.duration_secs,
            current_buffer_secs,
            ram_usage_mb,
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
        let _ = self.clean_buffer();
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

/// Aplica los argumentos de audio al comando wf-recorder.
fn apply_audio_args(cmd: &mut Command, config: &Config) {
    if config.audio.capture_desktop {
        if let Some(sink) = get_default_audio_sink() {
            cmd.arg(format!("-a{}", sink));
        } else {
            cmd.arg("-a");
        }
        cmd.args(["-C", "aac"]);
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
                "-b",
                "0",
                "--no-dmabuf",
            ]);
        }
        EncoderProfile::AmdHardware => {
            cmd.args(["-c", "h264_vaapi", "-b", "0"]);
        }
        EncoderProfile::NvidiaHardware => {
            cmd.args(["-c", "h264_nvenc", "-b", "0"]);
        }
        EncoderProfile::IntelHardware => {
            cmd.args(["-c", "h264_vaapi", "-b", "0"]);
        }
        EncoderProfile::Custom(ref c) => {
            cmd.args(["-c", c.as_str(), "-b", "0"]);
        }
    }
}

/// Obtiene el monitor del sink de audio por defecto usando pactl.
fn get_default_audio_sink() -> Option<String> {
    let output = Command::new("pactl")
        .arg("get-default-sink")
        .output()
        .ok()?;

    if output.status.success() {
        let sink = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !sink.is_empty() {
            if sink.ends_with(".monitor") {
                return Some(sink);
            } else {
                return Some(format!("{}.monitor", sink));
            }
        }
    }
    None
}

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
            ram_usage_mb: 28.5,
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

        let cmd0 = Command::new("ffmpeg")
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

        let cmd1 = Command::new("ffmpeg")
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
}
