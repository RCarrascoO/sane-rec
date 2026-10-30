use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    #[serde(default)]
    pub replay: ReplayConfig,
    #[serde(default)]
    pub video: VideoConfig,
    #[serde(default)]
    pub audio: AudioConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub profile: ProfileConfig,
    #[serde(default)]
    pub shortcuts: ShortcutsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplayConfig {
    pub duration_secs: u32,
    pub max_memory_mb: u32,
}

impl Default for ReplayConfig {
    fn default() -> Self {
        Self {
            duration_secs: 30,
            max_memory_mb: 150,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VideoConfig {
    pub target: String,
    pub fps: u32,
    pub scale: String,
    pub threads: u32,
    pub preset: String,
    pub crf: u32,
    #[serde(default = "default_video_codec")]
    pub codec: String,
}

fn default_video_codec() -> String {
    "libx264".to_string()
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            target: "portal".to_string(),
            fps: 60,
            scale: "none".to_string(),
            threads: 4,
            preset: "ultrafast".to_string(),
            crf: 23,
            codec: "libx264".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AudioConfig {
    pub capture_desktop: bool,
    pub capture_mic: bool,
    pub codec: String,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            capture_desktop: true,
            capture_mic: false,
            codec: "aac".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StorageConfig {
    pub output_dir: PathBuf,
    pub filename_format: String,
    pub format: String,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("/mnt/Files/Videos/Replays"),
            filename_format: "Replay_%Y-%m-%d_%H-%M-%S".to_string(),
            format: "mp4".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileConfig {
    pub active_profile: String,
}

impl Default for ProfileConfig {
    fn default() -> Self {
        Self {
            active_profile: "sane-safe".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShortcutsConfig {
    pub save_replay: String,
    pub toggle_record: String,
}

impl Default for ShortcutsConfig {
    fn default() -> Self {
        Self {
            save_replay: "Mod+F10".to_string(),
            toggle_record: "Mod+F9".to_string(),
        }
    }
}

/// Actualiza los atajos de teclado de sane-rec en el archivo de configuración de niri.
pub fn update_niri_shortcuts(save_bind: &str, record_bind: &str) -> std::io::Result<bool> {
    let path = if let Some(dirs) = directories::BaseDirs::new() {
        dirs.home_dir().join(".config/niri/config.d/70-binds.kdl")
    } else {
        PathBuf::from("/home/sane/.config/niri/config.d/70-binds.kdl")
    };

    if !path.exists() {
        return Ok(false);
    }

    let content = std::fs::read_to_string(&path)?;
    let mut lines: Vec<String> = content.lines().map(String::from).collect();
    let mut updated = false;

    for i in 0..lines.len() {
        let trimmed = lines[i].trim();
        if trimmed.contains("spawn \"sane-rec\" \"save\"") {
            lines[i] = format!("    {} {{ spawn \"sane-rec\" \"save\"; }}", save_bind);
            updated = true;
        } else if trimmed.contains("spawn \"sane-rec\" \"record\"") {
            lines[i] = format!("    {} {{ spawn \"sane-rec\" \"record\" \"toggle\"; }}", record_bind);
            updated = true;
        }
    }

    if updated {
        let mut new_content = lines.join("\n");
        new_content.push('\n');
        std::fs::write(&path, new_content)?;
    }

    Ok(updated)
}

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    TomlDe(toml::de::Error),
    TomlSer(toml::ser::Error),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(err) => write!(f, "IO error: {}", err),
            ConfigError::TomlDe(err) => write!(f, "Failed to deserialize config TOML: {}", err),
            ConfigError::TomlSer(err) => write!(f, "Failed to serialize config TOML: {}", err),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConfigError::Io(err) => Some(err),
            ConfigError::TomlDe(err) => Some(err),
            ConfigError::TomlSer(err) => Some(err),
        }
    }
}

impl From<std::io::Error> for ConfigError {
    fn from(err: std::io::Error) -> Self {
        ConfigError::Io(err)
    }
}

impl From<toml::de::Error> for ConfigError {
    fn from(err: toml::de::Error) -> Self {
        ConfigError::TomlDe(err)
    }
}

impl From<toml::ser::Error> for ConfigError {
    fn from(err: toml::ser::Error) -> Self {
        ConfigError::TomlSer(err)
    }
}

pub type Result<T> = std::result::Result<T, ConfigError>;

pub fn default_config_path() -> PathBuf {
    if let Some(dirs) = directories::BaseDirs::new() {
        dirs.config_dir().join("sane-rec").join("config.toml")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config").join("sane-rec").join("config.toml")
    } else {
        PathBuf::from("config.toml")
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            replay: ReplayConfig::default(),
            video: VideoConfig::default(),
            audio: AudioConfig::default(),
            storage: StorageConfig::default(),
            profile: ProfileConfig::default(),
            shortcuts: ShortcutsConfig::default(),
        }
    }
}

impl Config {
    /// Carga la configuración desde el archivo estándar `~/.config/sane-rec/config.toml`.
    /// Si no existe, crea una configuración con los valores por defecto y la guarda.
    /// Asegura además que exista la carpeta de salida `output_dir`.
    pub fn load() -> Result<Self> {
        Self::load_from_path(&default_config_path())
    }

    /// Carga la configuración desde una ruta específica.
    pub fn load_from_path(path: &Path) -> Result<Self> {
        let config = if path.exists() {
            let content = std::fs::read_to_string(path)?;
            toml::from_str(&content)?
        } else {
            let default_cfg = Self::default();
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = default_cfg.save_to_path(path);
            default_cfg
        };

        // Crea automáticamente la carpeta de salida si no existe
        let _ = config.ensure_output_dir();

        Ok(config)
    }

    /// Guarda la configuración actual en el archivo estándar `~/.config/sane-rec/config.toml`.
    /// Asegura además que exista la carpeta de salida `output_dir`.
    pub fn save(&self) -> Result<()> {
        self.save_to_path(&default_config_path())
    }

    /// Guarda la configuración en una ruta específica.
    pub fn save_to_path(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let serialized = toml::to_string_pretty(self)?;
        std::fs::write(path, serialized)?;
        let _ = self.ensure_output_dir();
        Ok(())
    }

    /// Crea la carpeta configurada en `storage.output_dir` si no existe todavía.
    pub fn ensure_output_dir(&self) -> Result<()> {
        if !self.storage.output_dir.exists() {
            std::fs::create_dir_all(&self.storage.output_dir)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_values() {
        let cfg = Config::default();

        assert_eq!(cfg.replay.duration_secs, 30);
        assert_eq!(cfg.replay.max_memory_mb, 150);

        assert_eq!(cfg.video.target, "portal");
        assert_eq!(cfg.video.fps, 60);
        assert_eq!(cfg.video.scale, "none");
        assert_eq!(cfg.video.threads, 4);
        assert_eq!(cfg.video.preset, "ultrafast");
        assert_eq!(cfg.video.crf, 23);
        assert_eq!(cfg.video.codec, "libx264");

        assert!(cfg.audio.capture_desktop);
        assert!(!cfg.audio.capture_mic);
        assert_eq!(cfg.audio.codec, "aac");

        assert_eq!(cfg.storage.output_dir, PathBuf::from("/mnt/Files/Videos/Replays"));
        assert_eq!(cfg.storage.filename_format, "Replay_%Y-%m-%d_%H-%M-%S");
        assert_eq!(cfg.storage.format, "mp4");

        assert_eq!(cfg.profile.active_profile, "sane-safe");
        assert_eq!(cfg.shortcuts.save_replay, "Mod+F10");
        assert_eq!(cfg.shortcuts.toggle_record, "Mod+F9");
    }

    #[test]
    fn test_toml_serialization_roundtrip() {
        let cfg = Config::default();
        let serialized = toml::to_string_pretty(&cfg).expect("Failed to serialize to TOML");
        let deserialized: Config = toml::from_str(&serialized).expect("Failed to deserialize from TOML");

        assert_eq!(cfg, deserialized);
    }

    #[test]
    fn test_save_and_load_file() {
        let temp_dir = std::env::temp_dir().join(format!("sane_rec_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let config_path = temp_dir.join("config.toml");
        let test_output_dir = temp_dir.join("videos_output");

        let mut cfg = Config::default();
        cfg.replay.duration_secs = 60;
        cfg.storage.output_dir = test_output_dir.clone();

        cfg.save_to_path(&config_path).expect("Failed to save config");
        assert!(config_path.exists());
        assert!(test_output_dir.exists(), "Output dir should be created automatically");

        let loaded = Config::load_from_path(&config_path).expect("Failed to load config");
        assert_eq!(loaded.replay.duration_secs, 60);
        assert_eq!(loaded.storage.output_dir, test_output_dir);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
