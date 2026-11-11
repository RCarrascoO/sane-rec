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

/// Helper puro para mapear atajos de sane-rec a sintaxis de atajos KDL de Niri.
/// Convierte "Mouse4" -> "MouseBack", "Mouse5" -> "MouseForward", "MouseMiddle" -> "MouseMiddle".
pub fn format_niri_bind(bind: &str) -> String {
    let parts: Vec<String> = bind
        .split('+')
        .map(|s| {
            let trimmed = s.trim();
            if trimmed.eq_ignore_ascii_case("Mouse4") {
                "MouseBack".to_string()
            } else if trimmed.eq_ignore_ascii_case("Mouse5") {
                "MouseForward".to_string()
            } else if trimmed.eq_ignore_ascii_case("MouseMiddle") {
                "MouseMiddle".to_string()
            } else {
                trimmed.to_string()
            }
        })
        .collect();
    parts.join("+")
}

/// Helper puro para aplicar y/o inyectar atajos de sane-rec en el contenido KDL de Niri.
/// Si encuentra líneas existentes las reemplaza in-place sin duplicar.
/// Si faltan atajos, los inserta antes de la llave de cierre `}` del bloque `binds { ... }`.
/// Si no existe bloque `binds { ... }`, añade uno al final.
pub fn apply_niri_binds_kdl(content: &str, save_bind: &str, record_bind: &str) -> Option<String> {
    let niri_save_bind = format_niri_bind(save_bind);
    let niri_record_bind = format_niri_bind(record_bind);
    let target_save_line = format!("    {} {{ spawn \"sane-rec\" \"save\"; }}", niri_save_bind);
    let target_record_line = format!("    {} {{ spawn \"sane-rec\" \"record\" \"toggle\"; }}", niri_record_bind);

    let lines: Vec<String> = content.lines().map(String::from).collect();
    let mut found_save = false;
    let mut found_record = false;

    let mut filtered_lines = Vec::new();
    for line in lines {
        let trimmed = line.trim();
        if trimmed.contains("spawn \"sane-rec\" \"save\"") {
            if !found_save {
                filtered_lines.push(target_save_line.clone());
                found_save = true;
            }
        } else if trimmed.contains("spawn \"sane-rec\" \"record\"") {
            if !found_record {
                filtered_lines.push(target_record_line.clone());
                found_record = true;
            }
        } else {
            filtered_lines.push(line);
        }
    }

    if !found_save || !found_record {
        let mut missing_lines = Vec::new();
        if !found_save {
            missing_lines.push(target_save_line);
        }
        if !found_record {
            missing_lines.push(target_record_line);
        }

        let mut binds_start_idx = None;
        let mut binds_closing_idx = None;
        let mut depth: i32 = 0;

        for (idx, line) in filtered_lines.iter().enumerate() {
            let non_comment = if let Some(pos) = line.find("//") {
                &line[..pos]
            } else {
                line.as_str()
            };

            if binds_start_idx.is_none() {
                let trimmed = non_comment.trim_start();
                if trimmed.starts_with("binds") {
                    let after = trimmed["binds".len()..].trim_start();
                    if after.starts_with('{') {
                        binds_start_idx = Some(idx);
                        let mut in_str = false;
                        let mut escaped = false;
                        for ch in non_comment.chars() {
                            if escaped {
                                escaped = false;
                                continue;
                            }
                            if ch == '\\' && in_str {
                                escaped = true;
                                continue;
                            }
                            if ch == '"' {
                                in_str = !in_str;
                                continue;
                            }
                            if !in_str {
                                if ch == '{' {
                                    depth += 1;
                                } else if ch == '}' {
                                    depth -= 1;
                                    if depth == 0 {
                                        binds_closing_idx = Some(idx);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                let mut in_str = false;
                let mut escaped = false;
                for ch in non_comment.chars() {
                    if escaped {
                        escaped = false;
                        continue;
                    }
                    if ch == '\\' && in_str {
                        escaped = true;
                        continue;
                    }
                    if ch == '"' {
                        in_str = !in_str;
                        continue;
                    }
                    if !in_str {
                        if ch == '{' {
                            depth += 1;
                        } else if ch == '}' {
                            depth -= 1;
                            if depth == 0 {
                                binds_closing_idx = Some(idx);
                                break;
                            }
                        }
                    }
                }
            }

            if binds_start_idx.is_some() && depth == 0 && binds_closing_idx.is_some() {
                break;
            }
        }

        if let Some(close_idx) = binds_closing_idx {
            if binds_start_idx == Some(close_idx) {
                let single_line = filtered_lines[close_idx].clone();
                if let Some(last_brace_pos) = single_line.rfind('}') {
                    let before = &single_line[..last_brace_pos];
                    let after = &single_line[last_brace_pos + 1..];
                    filtered_lines[close_idx] = before.trim_end().to_string();
                    let mut insert_idx = close_idx + 1;
                    for m_line in missing_lines {
                        filtered_lines.insert(insert_idx, m_line);
                        insert_idx += 1;
                    }
                    filtered_lines.insert(insert_idx, format!("}}{}", after));
                }
            } else {
                for (offset, m_line) in missing_lines.into_iter().enumerate() {
                    filtered_lines.insert(close_idx + offset, m_line);
                }
            }
        } else {
            if !filtered_lines.is_empty() && !filtered_lines.last().map(|s| s.trim().is_empty()).unwrap_or(false) {
                filtered_lines.push(String::new());
            }
            filtered_lines.push("binds {".to_string());
            for m_line in missing_lines {
                filtered_lines.push(m_line);
            }
            filtered_lines.push("}".to_string());
        }
    }

    let mut result = filtered_lines.join("\n");
    if !result.is_empty() && !result.ends_with('\n') {
        result.push('\n');
    }
    Some(result)
}

/// Helper puro que formatea un atajo para Hyprland devolviendo `(mods, key)`.
/// Modificadores: Mod/Super/Win -> SUPER, Ctrl/Control -> CTRL, Shift -> SHIFT, Alt -> ALT.
/// Ratón: Mouse4 -> mouse:275, Mouse5 -> mouse:276, MouseMiddle -> mouse:274.
pub fn format_hyprland_bind(bind: &str) -> (String, String) {
    let mut mods = Vec::new();
    let mut key = String::new();

    for part in bind.split('+').map(|s| s.trim()).filter(|s| !s.is_empty()) {
        match part.to_ascii_uppercase().as_str() {
            "MOD" | "SUPER" | "WIN" => mods.push("SUPER"),
            "CTRL" | "CONTROL" => mods.push("CTRL"),
            "SHIFT" => mods.push("SHIFT"),
            "ALT" => mods.push("ALT"),
            _ => {
                let mapped_key = if part.eq_ignore_ascii_case("Mouse4") {
                    "mouse:275"
                } else if part.eq_ignore_ascii_case("Mouse5") {
                    "mouse:276"
                } else if part.eq_ignore_ascii_case("MouseMiddle") {
                    "mouse:274"
                } else {
                    part
                };
                key = mapped_key.to_string();
            }
        }
    }

    (mods.join(" "), key)
}

#[cfg(target_os = "linux")]
static PREV_HYPRLAND_BINDS: std::sync::OnceLock<std::sync::Mutex<Option<(String, String)>>> =
    std::sync::OnceLock::new();

#[cfg(target_os = "linux")]
fn update_hyprland_shortcuts(save_bind: &str, record_bind: &str) -> bool {
    if std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_err() {
        return false;
    }

    let (save_mods, save_key) = format_hyprland_bind(save_bind);
    let (rec_mods, rec_key) = format_hyprland_bind(record_bind);

    let cur_save_combo = format!("{},{}", save_mods, save_key);
    let cur_rec_combo = format!("{},{}", rec_mods, rec_key);

    let binds_lock = PREV_HYPRLAND_BINDS.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(mut guard) = binds_lock.lock() {
        if let Some((prev_save, prev_rec)) = guard.as_ref() {
            if !prev_save.is_empty() {
                let _ = std::process::Command::new("hyprctl")
                    .args(["keyword", "unbind", prev_save])
                    .output();
            }
            if !prev_rec.is_empty() {
                let _ = std::process::Command::new("hyprctl")
                    .args(["keyword", "unbind", prev_rec])
                    .output();
            }
        }
        *guard = Some((cur_save_combo.clone(), cur_rec_combo.clone()));
    }

    let save_arg = format!("{},exec,sane-rec save", cur_save_combo);
    let rec_arg = format!("{},exec,sane-rec record toggle", cur_rec_combo);

    let mut success = false;
    if let Ok(status) = std::process::Command::new("hyprctl")
        .args(["keyword", "bind", &save_arg])
        .status()
    {
        if status.success() {
            success = true;
        }
    }

    if let Ok(status) = std::process::Command::new("hyprctl")
        .args(["keyword", "bind", &rec_arg])
        .status()
    {
        if status.success() {
            success = true;
        }
    }

    success
}

/// Actualiza los atajos de teclado de sane-rec en el archivo de configuración de niri y/o Hyprland.
/// En sistemas no-Linux, retorna `Ok(false)` limpiamente sin interactuar con configuraciones de Wayland.
pub fn update_niri_shortcuts(save_bind: &str, record_bind: &str) -> std::io::Result<bool> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (save_bind, record_bind);
        Ok(false)
    }
    #[cfg(target_os = "linux")]
    {
        let mut updated = false;

        if let Some(base_dirs) = directories::BaseDirs::new() {
            let binds_path = base_dirs.config_dir().join("niri/config.d/70-binds.kdl");
            if binds_path.exists() {
                let content = std::fs::read_to_string(&binds_path)?;
                if let Some(new_content) = apply_niri_binds_kdl(&content, save_bind, record_bind) {
                    if new_content != content {
                        std::fs::write(&binds_path, new_content)?;
                        updated = true;
                    }
                }
            }
        }

        if std::env::var("HYPRLAND_INSTANCE_SIGNATURE").is_ok() {
            if update_hyprland_shortcuts(save_bind, record_bind) {
                updated = true;
            }
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
    fn test_apply_niri_binds_kdl_inserts_when_missing() {
        let initial_kdl = r#"// Atajos niri
binds {
    Mod+Q { close-window; }
    Mod+Return { spawn "alacritty"; }
}
"#;
        // Inserción inicial dentro de binds con otros atajos, mapeando Mouse4 y Mouse5
        let res1 = apply_niri_binds_kdl(initial_kdl, "Mouse4", "Mouse5")
            .expect("Debe insertar los atajos faltantes");
        assert!(res1.contains("MouseBack { spawn \"sane-rec\" \"save\"; }"));
        assert!(res1.contains("MouseForward { spawn \"sane-rec\" \"record\" \"toggle\"; }"));
        assert!(res1.contains("Mod+Q { close-window; }"));
        assert_eq!(res1.matches("spawn \"sane-rec\" \"save\"").count(), 1);
        assert_eq!(res1.matches("spawn \"sane-rec\" \"record\"").count(), 1);

        // Reemplazo posterior sin duplicar
        let res2 = apply_niri_binds_kdl(&res1, "Mod+Shift+S", "Mod+Shift+R")
            .expect("Debe actualizar atajos existentes");
        assert!(res2.contains("Mod+Shift+S { spawn \"sane-rec\" \"save\"; }"));
        assert!(res2.contains("Mod+Shift+R { spawn \"sane-rec\" \"record\" \"toggle\"; }"));
        assert!(!res2.contains("MouseBack"));
        assert!(!res2.contains("MouseForward"));
        assert_eq!(res2.matches("spawn \"sane-rec\" \"save\"").count(), 1);
        assert_eq!(res2.matches("spawn \"sane-rec\" \"record\"").count(), 1);

        // Inserción en binds vacío
        let empty_binds = "binds {\n}";
        let res_empty = apply_niri_binds_kdl(empty_binds, "Mouse4", "Mouse5")
            .expect("Debe insertar en bloque binds vacío");
        assert!(res_empty.contains("MouseBack { spawn \"sane-rec\" \"save\"; }"));
        assert!(res_empty.contains("MouseForward { spawn \"sane-rec\" \"record\" \"toggle\"; }"));

        // Inserción cuando no existe ningún bloque binds
        let no_binds = "workspace \"1\"\n";
        let res_no_binds = apply_niri_binds_kdl(no_binds, "Mouse4", "Mouse5")
            .expect("Debe crear bloque binds nuevo");
        assert!(res_no_binds.contains("binds {"));
        assert!(res_no_binds.contains("MouseBack { spawn \"sane-rec\" \"save\"; }"));
        assert!(res_no_binds.contains("MouseForward { spawn \"sane-rec\" \"record\" \"toggle\"; }"));
    }

    #[test]
    fn test_format_hyprland_bind() {
        let (mods, key) = format_hyprland_bind("Mouse4");
        assert_eq!(format!("{mods},{key}"), ",mouse:275");
        assert_eq!(mods, "");
        assert_eq!(key, "mouse:275");

        let (mods, key) = format_hyprland_bind("Mouse5");
        assert_eq!(format!("{mods},{key}"), ",mouse:276");
        assert_eq!(mods, "");
        assert_eq!(key, "mouse:276");

        let (mods, key) = format_hyprland_bind("Mod+Shift+S");
        assert_eq!(format!("{mods},{key}"), "SUPER SHIFT,S");
        assert_eq!(mods, "SUPER SHIFT");
        assert_eq!(key, "S");

        let (mods, key) = format_hyprland_bind("F10");
        assert_eq!(format!("{mods},{key}"), ",F10");
        assert_eq!(mods, "");
        assert_eq!(key, "F10");

        let (mods, key) = format_hyprland_bind("Mod+Mouse4");
        assert_eq!(format!("{mods},{key}"), "SUPER,mouse:275");

        let (mods, key) = format_hyprland_bind("Ctrl+Alt+MouseMiddle");
        assert_eq!(format!("{mods},{key}"), "CTRL ALT,mouse:274");
    }

    #[test]
    fn test_format_niri_bind() {
        assert_eq!(format_niri_bind("Mouse4"), "MouseBack");
        assert_eq!(format_niri_bind("Mouse5"), "MouseForward");
        assert_eq!(format_niri_bind("MouseMiddle"), "MouseMiddle");
        assert_eq!(format_niri_bind("Mod+Mouse4"), "Mod+MouseBack");
        assert_eq!(format_niri_bind("Alt+F10"), "Alt+F10");
        assert_eq!(format_niri_bind("Mod+Shift+S"), "Mod+Shift+S");
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
