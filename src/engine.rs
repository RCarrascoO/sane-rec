use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use gstreamer as gst;
use gstreamer::prelude::*;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::profiles::{self, EncoderProfile};

pub const DEFAULT_SEGMENT_DURATION_SECS: u32 = 2;
pub const DEFAULT_MAX_SIZE_TIME_NS: u64 = 2_000_000_000; // 2 segundos

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
    Gstreamer(gst::glib::Error),
    GstreamerBool(gst::glib::BoolError),
    StateChange(gst::StateChangeError),
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
            EngineError::Gstreamer(e) => write!(f, "Error de GStreamer: {}", e),
            EngineError::GstreamerBool(e) => write!(f, "Error booleano de GStreamer: {}", e),
            EngineError::StateChange(e) => write!(f, "Error de cambio de estado en GStreamer: {:?}", e),
            EngineError::Io(e) => write!(f, "Error de E/S: {}", e),
            EngineError::NoSegments => write!(f, "No hay segmentos disponibles en el búfer para guardar el replay"),
            EngineError::FfmpegError(msg) => write!(f, "Error de FFmpeg: {}", msg),
            EngineError::AlreadyRunning => write!(f, "El búfer de repetición o la grabación ya se encuentra en ejecución"),
            EngineError::NotRunning => write!(f, "El búfer de repetición o la grabación no está en ejecución"),
            EngineError::ConfigError(msg) => write!(f, "Error de configuración: {}", msg),
            EngineError::Custom(msg) => write!(f, "Error del motor: {}", msg),
        }
    }
}

impl std::error::Error for EngineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            EngineError::Gstreamer(e) => Some(e),
            EngineError::GstreamerBool(e) => Some(e),
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

impl From<gst::glib::Error> for EngineError {
    fn from(e: gst::glib::Error) -> Self {
        EngineError::Gstreamer(e)
    }
}

impl From<gst::glib::BoolError> for EngineError {
    fn from(e: gst::glib::BoolError) -> Self {
        EngineError::GstreamerBool(e)
    }
}

impl From<gst::StateChangeError> for EngineError {
    fn from(e: gst::StateChangeError) -> Self {
        EngineError::StateChange(e)
    }
}

pub type Result<T> = std::result::Result<T, EngineError>;

/// Motor principal de sane-rec para repetición instantánea y grabación continua.
pub struct ReplayEngine {
    config: Mutex<Config>,
    buffer_dir: PathBuf,
    pipeline: Mutex<Option<gst::Pipeline>>,
    continuous_pipeline: Mutex<Option<gst::Pipeline>>,
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
        // Inicializa GStreamer de forma segura e idempotente
        let _ = gst::init();

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
            pipeline: Mutex::new(None),
            continuous_pipeline: Mutex::new(None),
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

    /// Inicia el pipeline GStreamer de repetición instantánea con splitmuxsink en RAM.
    pub fn start_replay(&self) -> Result<()> {
        if self.is_running() {
            return Err(EngineError::AlreadyRunning);
        }

        // Limpieza preventiva del buffer
        self.clean_buffer()?;
        let _ = std::fs::create_dir_all(&self.buffer_dir);

        let config = self.config();
        let profile = config
            .profile
            .active_profile
            .parse::<EncoderProfile>()
            .unwrap_or(EncoderProfile::SaneSafe);

        let target_node = if config.video.target.is_empty() || config.video.target == "portal" {
            None
        } else {
            Some(config.video.target.as_str())
        };

        let video_pipe = profiles::build_video_pipeline(&profile, target_node, config.video.fps);
        let audio_pipe = profiles::build_audio_pipeline(
            config.audio.capture_desktop,
            config.audio.capture_mic,
            &config.audio.codec,
        );

        // Número de archivos rodantes en RAM: (duration_secs / 2) + 1
        let max_files = (config.replay.duration_secs / DEFAULT_SEGMENT_DURATION_SECS) + 1;
        let location = format!("{}/segment_%05d.ts", self.buffer_dir.display());

        let pipeline_str = if !audio_pipe.is_empty() {
            format!(
                "{} ! queue ! mux.video {} ! queue ! mux.audio_0 splitmuxsink muxer-factory=mpegtsmux name=mux location=\"{}\" max-size-time={} max-files={} send-keyframe-requests=true",
                video_pipe, audio_pipe, location, DEFAULT_MAX_SIZE_TIME_NS, max_files
            )
        } else {
            format!(
                "{} ! queue ! mux.video splitmuxsink muxer-factory=mpegtsmux name=mux location=\"{}\" max-size-time={} max-files={} send-keyframe-requests=true",
                video_pipe, location, DEFAULT_MAX_SIZE_TIME_NS, max_files
            )
        };

        let element = gst::parse::launch(&pipeline_str)?;
        let pipeline = element
            .dynamic_cast::<gst::Pipeline>()
            .map_err(|_| EngineError::Custom("Pipeline creado no es de tipo gst::Pipeline".into()))?;

        pipeline.set_state(gst::State::Playing)?;

        *self.pipeline.lock().unwrap() = Some(pipeline);
        *self.start_time.lock().unwrap() = Some(Instant::now());

        Ok(())
    }

    /// Detiene el pipeline de repetición instantánea y limpia la RAM.
    pub fn stop_replay(&self) -> Result<()> {
        let mut guard = self.pipeline.lock().unwrap();
        if let Some(pipe) = guard.take() {
            let _ = pipe.set_state(gst::State::Null);
        }
        *self.start_time.lock().unwrap() = None;

        // Limpieza automática al cerrar
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

    /// Indica si el búfer de repetición instantánea se encuentra en ejecución.
    pub fn is_running(&self) -> bool {
        self.pipeline.lock().unwrap().is_some()
    }

    /// Inicia la grabación continua directa al directorio de almacenamiento.
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

        let profile = config
            .profile
            .active_profile
            .parse::<EncoderProfile>()
            .unwrap_or(EncoderProfile::SaneSafe);

        let target_node = if config.video.target.is_empty() || config.video.target == "portal" {
            None
        } else {
            Some(config.video.target.as_str())
        };

        let video_pipe = profiles::build_video_pipeline(&profile, target_node, config.video.fps);
        let audio_pipe = profiles::build_audio_pipeline(
            config.audio.capture_desktop,
            config.audio.capture_mic,
            &config.audio.codec,
        );

        let muxer = if config.storage.format.eq_ignore_ascii_case("mkv") {
            "matroskamux"
        } else {
            "mp4mux"
        };

        let pipeline_str = if !audio_pipe.is_empty() {
            format!(
                "{} ! queue ! mux.video_0 {} ! queue ! mux.audio_0 {} name=mux ! filesink location=\"{}\"",
                video_pipe, audio_pipe, muxer, output_path.display()
            )
        } else {
            format!(
                "{} ! queue ! mux.video_0 {} name=mux ! filesink location=\"{}\"",
                video_pipe, muxer, output_path.display()
            )
        };

        let element = gst::parse::launch(&pipeline_str)?;
        let pipeline = element
            .dynamic_cast::<gst::Pipeline>()
            .map_err(|_| EngineError::Custom("Pipeline creado no es de tipo gst::Pipeline".into()))?;

        pipeline.set_state(gst::State::Playing)?;

        *self.continuous_pipeline.lock().unwrap() = Some(pipeline);
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

    /// Detiene la grabación continua, finaliza el contenedor y notifica al usuario.
    pub fn stop_continuous_recording(&self) -> Result<PathBuf> {
        let mut guard = self.continuous_pipeline.lock().unwrap();
        let pipeline = guard.take().ok_or(EngineError::NotRunning)?;
        let output_path = self
            .continuous_output
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| PathBuf::from("recording.mp4"));
        *self.continuous_start_time.lock().unwrap() = None;

        // Envía EOS para permitir que el muxer escriba metadatos finales (ej. moov atom)
        let _ = pipeline.send_event(gst::event::Eos::new());
        if let Some(bus) = pipeline.bus() {
            for msg in bus.iter_timed(gst::ClockTime::from_mseconds(1000)) {
                use gst::MessageView;
                match msg.view() {
                    MessageView::Eos(..) | MessageView::Error(..) => break,
                    _ => (),
                }
            }
        }
        let _ = pipeline.set_state(gst::State::Null);

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

    /// Indica si la grabación continua se encuentra activa.
    pub fn is_recording(&self) -> bool {
        self.continuous_pipeline.lock().unwrap().is_some()
    }

    /// Guarda un replay del búfer actual concatenando segmentos en RAM a alta velocidad.
    pub fn save_clip(&self) -> Result<PathBuf> {
        // 1. Escanea los segmentos .mp4 o .ts en el directorio de búfer ordenados cronológicamente
        let mut segments = self.scan_segments()?;
        if segments.is_empty() {
            return Err(EngineError::NoSegments);
        }

        // Orden cronológico por fecha de modificación o nombre
        segments.sort_by(|a, b| {
            let mod_a = a.metadata().and_then(|m| m.modified()).ok();
            let mod_b = b.metadata().and_then(|m| m.modified()).ok();
            match (mod_a, mod_b) {
                (Some(ta), Some(tb)) => ta.cmp(&tb),
                _ => a.cmp(b),
            }
        });

        // Para archivos .mp4, descartar el último si todavía está incompleto (sin atom moov)
        if let Some(last) = segments.last() {
            if last.extension().and_then(|e| e.to_str()) == Some("mp4") && segments.len() > 1 {
                if let Ok(bytes) = std::fs::read(last) {
                    if !bytes.windows(4).any(|w| w == b"moov") {
                        segments.pop();
                    }
                }
            }
        }

        if segments.is_empty() {
            return Err(EngineError::NoSegments);
        }

        // 2. Genera la lista para ffmpeg concat
        let list_file = self
            .buffer_dir
            .join(format!("concat_{}.txt", std::process::id()));
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

        // 4. Concatena los segmentos usando ffmpeg con copia directa de streams (<150 ms)
        let ffmpeg_res = std::process::Command::new("ffmpeg")
            .arg("-y")
            .arg("-f")
            .arg("concat")
            .arg("-safe")
            .arg("0")
            .arg("-i")
            .arg(&list_file)
            .arg("-c")
            .arg("copy")
            .arg("-movflags")
            .arg("+faststart")
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

        // 5. Calcula la duración del clip guardado
        let duration_secs = (segments.len() as u32 * DEFAULT_SEGMENT_DURATION_SECS)
            .min(config.replay.duration_secs);

        // 6. Lanza la notificación de escritorio
        let _ = notify_rust::Notification::new()
            .appname("sane-rec")
            .summary("🎬 Replay Guardado")
            .body(&format!(
                "Clip de {}s guardado en {}",
                duration_secs,
                output_path.display()
            ))
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

    /// Escanea los segmentos válidos presentes en la RAM.
    pub fn scan_segments(&self) -> Result<Vec<PathBuf>> {
        let mut segments = Vec::new();

        if self.buffer_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&self.buffer_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                        if (ext == "ts" || ext == "mp4")
                            && path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .map_or(false, |name| name.starts_with("segment_"))
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
                        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                        if (ext == "ts" || ext == "mp4")
                            && path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .map_or(false, |name| name.starts_with("segment_"))
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
                    if name.starts_with("segment_") || name.starts_with("concat_") {
                        let _ = std::fs::remove_file(path);
                    }
                }
            }
        }
    }
}

impl Drop for ReplayEngine {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.pipeline.lock() {
            if let Some(pipe) = guard.take() {
                let _ = pipe.set_state(gst::State::Null);
            }
        }
        if let Ok(mut guard) = self.continuous_pipeline.lock() {
            if let Some(pipe) = guard.take() {
                let _ = pipe.set_state(gst::State::Null);
            }
        }
        let _ = self.clean_buffer();
    }
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
        let temp_dir = std::env::temp_dir().join(format!("sane_rec_engine_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);

        let mut config = Config::default();
        config.replay.duration_secs = 20;

        let engine = ReplayEngine::with_buffer_dir(config, temp_dir.clone()).unwrap();

        // Crear 3 segmentos ficticios
        let seg0 = temp_dir.join("segment_00000.ts");
        let seg1 = temp_dir.join("segment_00001.ts");
        let seg2 = temp_dir.join("segment_00002.ts");
        let other = temp_dir.join("other.txt");

        std::fs::write(&seg0, b"data0").unwrap();
        std::fs::write(&seg1, b"data1").unwrap();
        std::fs::write(&seg2, b"data2").unwrap();
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
        let temp_dir = std::env::temp_dir().join(format!("sane_rec_clip_test_{}", std::process::id()));
        let buffer_dir = temp_dir.join("buffer");
        let output_dir = temp_dir.join("output");
        let _ = std::fs::create_dir_all(&buffer_dir);
        let _ = std::fs::create_dir_all(&output_dir);

        let mut config = Config::default();
        config.storage.output_dir = output_dir.clone();
        config.storage.format = "mp4".to_string();

        let engine = ReplayEngine::with_buffer_dir(config, buffer_dir.clone()).unwrap();

        // Generar 2 segmentos reales de 1 segundo con GStreamer
        let seg0 = buffer_dir.join("segment_00000.ts");
        let seg1 = buffer_dir.join("segment_00001.ts");

        let cmd0 = format!(
            "gst-launch-1.0 -e videotestsrc num-buffers=30 ! video/x-raw,framerate=30/1 ! x264enc speed-preset=ultrafast tune=zerolatency ! h264parse ! mpegtsmux ! filesink location=\"{}\"",
            seg0.display()
        );
        let cmd1 = format!(
            "gst-launch-1.0 -e videotestsrc num-buffers=30 ! video/x-raw,framerate=30/1 ! x264enc speed-preset=ultrafast tune=zerolatency ! h264parse ! mpegtsmux ! filesink location=\"{}\"",
            seg1.display()
        );

        let res0 = std::process::Command::new("sh").arg("-c").arg(&cmd0).output();
        let res1 = std::process::Command::new("sh").arg("-c").arg(&cmd1).output();

        if let (Ok(r0), Ok(r1)) = (res0, res1) {
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
