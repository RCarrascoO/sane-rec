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
    #[serde(default)]
    pub ui: UiConfig,
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
    #[serde(default)]
    pub custom_threads: Option<u32>,
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
            custom_threads: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioConfig {
    pub capture_desktop: bool,
    pub capture_mic: bool,
    pub codec: String,
    #[serde(default = "default_volume_system")]
    pub volume_system: f32,
    #[serde(default = "default_volume_mic")]
    pub volume_mic: f32,
}

fn default_volume_system() -> f32 {
    1.0
}

fn default_volume_mic() -> f32 {
    0.85
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            capture_desktop: true,
            capture_mic: false,
            codec: "aac".to_string(),
            volume_system: 1.0,
            volume_mic: 0.85,
        }
    }
}

fn default_discord_target_mb() -> u32 {
    10
}

pub fn default_output_dir() -> PathBuf {
    let dir = if let Some(user_dirs) = directories::UserDirs::new() {
        user_dirs
            .video_dir()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| user_dirs.home_dir().join("Videos"))
            .join("SaneRec")
    } else {
        std::env::temp_dir().join("SaneRec")
    };

    let _ = std::fs::create_dir_all(&dir);
    dir
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StorageConfig {
    pub output_dir: PathBuf,
    pub filename_format: String,
    pub format: String,
    #[serde(default)]
    pub dual_clip: bool,
    #[serde(default = "default_discord_target_mb")]
    pub discord_target_mb: u32,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            output_dir: default_output_dir(),
            filename_format: "Replay_%Y-%m-%d_%H-%M-%S".to_string(),
            format: "mp4".to_string(),
            dual_clip: false,
            discord_target_mb: default_discord_target_mb(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileConfig {
    pub active_profile: String,
    #[serde(default)]
    pub selected_hardware_profile: Option<String>,
}

impl Default for ProfileConfig {
    fn default() -> Self {
        Self {
            active_profile: "sane-safe".to_string(),
            selected_hardware_profile: None,
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
        #[cfg(target_os = "windows")]
        {
            Self {
                save_replay: "F10".to_string(),
                toggle_record: "F9".to_string(),
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            Self {
                save_replay: "Mod+Shift+S".to_string(),
                toggle_record: "Mod+Shift+R".to_string(),
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UiConfig {
    #[serde(default = "default_ui_theme")]
    pub theme: String,
    #[serde(default)]
    pub custom_theme_name: Option<String>,
}

fn default_ui_theme() -> String {
    "dark".to_string()
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: default_ui_theme(),
            custom_theme_name: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommunityTheme {
    pub name: String,
    #[serde(default = "default_theme_surface")]
    pub surface: String,
    #[serde(default = "default_theme_surface_container")]
    pub surface_container: String,
    #[serde(default = "default_theme_primary")]
    pub primary: String,
    #[serde(default = "default_theme_on_primary")]
    pub on_primary: String,
    #[serde(default = "default_theme_on_surface")]
    pub on_surface: String,
    #[serde(default = "default_theme_outline")]
    pub outline: String,
    #[serde(default = "default_theme_radius_scale")]
    pub corner_radius_scale: f32,
}

fn default_theme_surface() -> String {
    "#141218".to_string()
}

fn default_theme_surface_container() -> String {
    "#211F26".to_string()
}

fn default_theme_primary() -> String {
    "#D0BCFF".to_string()
}

fn default_theme_on_primary() -> String {
    "#381E72".to_string()
}

fn default_theme_on_surface() -> String {
    "#E6E0E9".to_string()
}

fn default_theme_outline() -> String {
    "#938F99".to_string()
}

fn default_theme_radius_scale() -> f32 {
    1.0
}

impl Default for CommunityTheme {
    fn default() -> Self {
        Self {
            name: "Default-Dark".to_string(),
            surface: default_theme_surface(),
            surface_container: default_theme_surface_container(),
            primary: default_theme_primary(),
            on_primary: default_theme_on_primary(),
            on_surface: default_theme_on_surface(),
            outline: default_theme_outline(),
            corner_radius_scale: default_theme_radius_scale(),
        }
    }
}

pub const DEFAULT_THEME_CYBERPUNK: &str = r##"name = "Community-Cyberpunk"
surface = "#0B0914"
surface_container = "#18142C"
primary = "#00F0FF"
on_primary = "#001F24"
on_surface = "#FCEE09"
outline = "#FF0055"
corner_radius_scale = 0.5
"##;

pub const DEFAULT_THEME_NORD: &str = r##"name = "Community-Nord"
surface = "#2E3440"
surface_container = "#3B4252"
primary = "#88C0D0"
on_primary = "#2E3440"
on_surface = "#ECEFF4"
outline = "#4C566A"
corner_radius_scale = 1.0
"##;

pub const DEFAULT_COMMUNITY_THEMES: [(&str, &str); 2] = [
    ("Community-Cyberpunk.toml", DEFAULT_THEME_CYBERPUNK),
    ("Community-Nord.toml", DEFAULT_THEME_NORD),
];

/// Devuelve la ruta al directorio de temas (`~/.config/sane-rec/themes` en Linux o `%APPDATA%\sane-rec\themes` en Windows).
pub fn themes_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Some(user_dirs) = directories::BaseDirs::new() {
            return user_dirs.config_dir().join("sane-rec").join("themes");
        }
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata).join("sane-rec").join("themes");
        }
    }

    if let Some(user_dirs) = directories::BaseDirs::new() {
        user_dirs.config_dir().join("sane-rec").join("themes")
    } else if let Some(proj_dirs) = directories::ProjectDirs::from("com", "sane", "sane-rec") {
        proj_dirs.config_dir().join("themes")
    } else {
        std::env::temp_dir().join("sane-rec").join("themes")
    }
}

/// Inicializa el directorio de temas comunitarios y siembra los temas por defecto si no existen.
pub fn init_default_themes() {
    let dir = themes_dir();
    let _ = std::fs::create_dir_all(&dir);

    for (file_name, content) in DEFAULT_COMMUNITY_THEMES {
        let dest = dir.join(file_name);
        if !dest.exists() {
            let _ = std::fs::write(&dest, content);
        }
    }
}

/// Lista los nombres de los temas comunitarios disponibles (sin la extensión .toml).
pub fn list_community_themes() -> Vec<String> {
    let dir = themes_dir();
    let mut names = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    names.push(stem.to_string());
                }
            }
        }
    }

    names.sort();
    names
}

/// Carga y parsea un tema comunitario por su nombre desde `themes_dir()`, con protección contra path traversal.
pub fn load_community_theme(name: &str) -> std::result::Result<CommunityTheme, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err("Nombre de tema no válido o contiene caracteres no permitidos.".to_string());
    }

    let file_name = if trimmed.ends_with(".toml") {
        trimmed.to_string()
    } else {
        format!("{}.toml", trimmed)
    };
    let path = themes_dir().join(&file_name);

    let content = if path.exists() {
        std::fs::read_to_string(&path)
            .map_err(|e| format!("Error al leer tema '{}': {}", path.display(), e))?
    } else {
        DEFAULT_COMMUNITY_THEMES
            .iter()
            .find(|(fname, _)| fname.eq_ignore_ascii_case(&file_name))
            .map(|(_, content)| (*content).to_string())
            .ok_or_else(|| format!("Error al leer tema '{}': Archivo no encontrado", path.display()))?
    };

    let theme: CommunityTheme = toml::from_str(&content)
        .map_err(|e| format!("Error al parsear TOML del tema: {}", e))?;
    Ok(theme)
}

/// Parsea una cadena de color hexadecimal (#RRGGBB o #RRGGBBAA) y devuelve `(r, g, b, a)` con `a = 255` por defecto.
pub fn parse_hex_color(hex: &str) -> Option<(u8, u8, u8, u8)> {
    let trimmed = hex.trim().trim_start_matches('#');
    match trimmed.len() {
        3 => {
            let r = u8::from_str_radix(&trimmed[0..1], 16).ok()?;
            let g = u8::from_str_radix(&trimmed[1..2], 16).ok()?;
            let b = u8::from_str_radix(&trimmed[2..3], 16).ok()?;
            Some((r * 17, g * 17, b * 17, 255))
        }
        6 => {
            let r = u8::from_str_radix(&trimmed[0..2], 16).ok()?;
            let g = u8::from_str_radix(&trimmed[2..4], 16).ok()?;
            let b = u8::from_str_radix(&trimmed[4..6], 16).ok()?;
            Some((r, g, b, 255))
        }
        8 => {
            let r = u8::from_str_radix(&trimmed[0..2], 16).ok()?;
            let g = u8::from_str_radix(&trimmed[2..4], 16).ok()?;
            let b = u8::from_str_radix(&trimmed[4..6], 16).ok()?;
            let a = u8::from_str_radix(&trimmed[6..8], 16).ok()?;
            Some((r, g, b, a))
        }
        _ => None,
    }
}

/// Actualiza los atajos de teclado de sane-rec en el archivo de configuración de niri.
/// En sistemas no-Linux, retorna `Ok(false)` limpiamente sin interactuar con configuraciones de Wayland.
pub fn update_niri_shortcuts(save_bind: &str, record_bind: &str) -> std::io::Result<bool> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (save_bind, record_bind);
        Ok(false)
    }
    #[cfg(target_os = "linux")]
    {
        let base_dirs = match directories::BaseDirs::new() {
            Some(dirs) => dirs,
            None => return Ok(false),
        };
        let binds_path = base_dirs.config_dir().join("niri/config.d/70-binds.kdl");

        if !binds_path.exists() {
            return Ok(false);
        }

        let content = std::fs::read_to_string(&binds_path)?;
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
            std::fs::write(&binds_path, new_content)?;
        }

        Ok(updated)
    }
}

/// Inyecta o actualiza los atajos de teclado en Niri si aplica. En sistemas no-Linux retorna Ok(false) limpiamente.
pub fn inject_niri_keybinds(save_bind: &str, record_bind: &str) -> std::io::Result<bool> {
    update_niri_shortcuts(save_bind, record_bind)
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
    #[cfg(target_os = "windows")]
    {
        if let Some(dirs) = directories::BaseDirs::new() {
            dirs.config_dir().join("sane-rec").join("config.toml")
        } else if let Ok(appdata) = std::env::var("APPDATA") {
            PathBuf::from(appdata).join("sane-rec").join("config.toml")
        } else {
            PathBuf::from("config.toml")
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Some(dirs) = directories::BaseDirs::new() {
            dirs.config_dir().join("sane-rec").join("config.toml")
        } else if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join(".config").join("sane-rec").join("config.toml")
        } else {
            PathBuf::from("config.toml")
        }
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
            ui: UiConfig::default(),
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

    /// Guarda la configuración en una ruta específica utilizando reemplazo atómico.
    pub fn save_to_path(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let serialized = toml::to_string_pretty(self)?;
        let tmp_path = path.with_extension("tmp");
        std::fs::write(&tmp_path, serialized)?;
        std::fs::rename(&tmp_path, path)?;
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
        assert_eq!(cfg.video.custom_threads, None);

        assert!(cfg.audio.capture_desktop);
        assert!(!cfg.audio.capture_mic);
        assert_eq!(cfg.audio.codec, "aac");
        assert_eq!(cfg.audio.volume_system, 1.0);
        assert_eq!(cfg.audio.volume_mic, 0.85);

        assert!(cfg.storage.output_dir.to_string_lossy().contains("SaneRec"));
        assert_eq!(cfg.storage.filename_format, "Replay_%Y-%m-%d_%H-%M-%S");
        assert_eq!(cfg.storage.format, "mp4");
        assert!(!cfg.storage.dual_clip);
        assert_eq!(cfg.storage.discord_target_mb, 10);

        assert_eq!(cfg.profile.active_profile, "sane-safe");
        assert_eq!(cfg.profile.selected_hardware_profile, None);
        #[cfg(not(target_os = "windows"))]
        {
            assert_eq!(cfg.shortcuts.save_replay, "Mod+Shift+S");
            assert_eq!(cfg.shortcuts.toggle_record, "Mod+Shift+R");
        }
        #[cfg(target_os = "windows")]
        {
            assert_eq!(cfg.shortcuts.save_replay, "F10");
            assert_eq!(cfg.shortcuts.toggle_record, "F9");
        }

        assert_eq!(cfg.ui.theme, "dark");
        assert_eq!(cfg.ui.custom_theme_name, None);
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

    #[test]
    fn test_inject_niri_keybinds_does_not_panic() {
        // En Windows retorna Ok(false), en Linux busca el archivo o retorna Ok(false)
        let res = inject_niri_keybinds("Mod+F10", "Mod+F9");
        assert!(res.is_ok());
    }

    #[test]
    fn test_parse_hex_color() {
        assert_eq!(parse_hex_color("#FF0000"), Some((255, 0, 0, 255)));
        assert_eq!(parse_hex_color("00FF00"), Some((0, 255, 0, 255)));
        assert_eq!(parse_hex_color("#0000FF80"), Some((0, 0, 255, 128)));
        assert_eq!(parse_hex_color("#0F0"), Some((0, 255, 0, 255)));
        assert_eq!(parse_hex_color("invalid"), None);
        assert_eq!(parse_hex_color("#12345"), None);
    }

    #[test]
    fn test_community_theme_serialization_and_load() {
        let theme_str = r##"
            name = "Test-Theme"
            surface = "#112233"
            surface_container = "#223344"
            primary = "#334455"
            on_primary = "#445566"
            on_surface = "#556677"
            outline = "#667788"
            corner_radius_scale = 0.75
        "##;
        let theme: CommunityTheme = toml::from_str(theme_str).expect("Failed to deserialize theme");
        assert_eq!(theme.name, "Test-Theme");
        assert_eq!(theme.surface, "#112233");
        assert_eq!(theme.corner_radius_scale, 0.75);

        let default_theme = CommunityTheme::default();
        let serialized = toml::to_string_pretty(&default_theme).expect("Failed to serialize theme");
        let deserialized: CommunityTheme = toml::from_str(&serialized).expect("Failed to deserialize theme");
        assert_eq!(default_theme, deserialized);

        // Prueba de carga con fallback embebido
        let cyberpunk = load_community_theme("Community-Cyberpunk").expect("Should load cyberpunk theme");
        assert_eq!(cyberpunk.name, "Community-Cyberpunk");
        assert_eq!(cyberpunk.primary, "#00F0FF");
        assert_eq!(cyberpunk.corner_radius_scale, 0.5);

        // Path traversal rejection
        assert!(load_community_theme("../evil").is_err());
        assert!(load_community_theme("sub/theme").is_err());
        assert!(load_community_theme("").is_err());
    }

    #[test]
    fn test_config_ui_backwards_compatible() {
        let old_toml = r#"
            [replay]
            duration_secs = 45
            max_memory_mb = 200
        "#;
        let cfg: Config = toml::from_str(old_toml).expect("Should deserialize old config");
        assert_eq!(cfg.ui.theme, "dark");
        assert_eq!(cfg.ui.custom_theme_name, None);
    }

    #[test]
    fn test_no_hardcoded_author_paths_in_defaults() {
        let default_cfg = Config::default();
        let out_dir = default_cfg.storage.output_dir.to_string_lossy();
        assert!(!out_dir.contains("/mnt/Files"), "default output_dir no debe contener /mnt/Files");
        assert!(out_dir.ends_with("SaneRec"), "default output_dir debe terminar en SaneRec");

        let prof_path = crate::profiles_manager::profiles_dir();
        let prof_dir = prof_path.to_string_lossy();
        assert!(!prof_dir.contains("/mnt/Files"), "profiles_dir no debe contener /mnt/Files");
        assert!(prof_dir.ends_with("profiles"), "profiles_dir debe terminar en profiles");

        let serialized = toml::to_string(&default_cfg).expect("serialize default config");
        assert!(!serialized.contains("/mnt/Files"), "Default toml no debe contener /mnt/Files");
    }

    #[test]
    fn test_selected_hardware_profile_defaults_to_none() {
        assert_eq!(Config::default().profile.selected_hardware_profile, None);
    }
}
