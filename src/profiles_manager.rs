use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Perfil de hardware exportable/importable para optimizaciones específicas por equipo.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HardwareProfile {
    pub name: String,
    pub description: String,
    pub target_device: String,
    pub codec: String,
    pub preset: String,
    pub crf: u32,
    pub fps: u32,
    pub resolution: String,
    pub max_memory_mb: u64,
    pub audio_track_mode: u32,
}

impl Default for HardwareProfile {
    fn default() -> Self {
        Self {
            name: "Sane-Desktop-Polaris".to_string(),
            description: "Perfil optimizado para GPUs Polaris (RX 570) y CPU multinúcleo".to_string(),
            target_device: "AMD Polaris RX 570 / Desktop".to_string(),
            codec: "libx264".to_string(),
            preset: "ultrafast".to_string(),
            crf: 21,
            fps: 60,
            resolution: "1360x768".to_string(),
            max_memory_mb: 150,
            audio_track_mode: 0,
        }
    }
}

/// Devuelve la ruta al directorio de perfiles (`~/.config/sane-rec/profiles/` o `%APPDATA%\sane-rec\profiles\`).
pub fn profiles_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Some(user_dirs) = directories::BaseDirs::new() {
            user_dirs.config_dir().join("sane-rec").join("profiles")
        } else if let Ok(appdata) = std::env::var("APPDATA") {
            PathBuf::from(appdata).join("sane-rec").join("profiles")
        } else {
            PathBuf::from("profiles")
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Some(user_dirs) = directories::BaseDirs::new() {
            user_dirs.config_dir().join("sane-rec").join("profiles")
        } else {
            PathBuf::from("/home/sane/.config/sane-rec/profiles")
        }
    }
}

pub const DEFAULT_PROFILE_POLARIS: &str = include_str!("../profiles/Sane-Desktop-Polaris.toml");
pub const DEFAULT_PROFILE_NOTEBOOK: &str = include_str!("../profiles/Notebook-Universal-Safe.toml");
pub const DEFAULT_PROFILE_HIME: &str = include_str!("../profiles/hime-config.toml");
pub const DEFAULT_PROFILE_PAVILION: &str = include_str!("../profiles/Pavilion-Gaming-Celsius.toml");
pub const DEFAULT_PROFILE_POLARIS_VAAPI: &str = include_str!("../profiles/Polaris-VAAPI-ZeroBFrames.toml");

pub const DEFAULT_PROFILES: [(&str, &str); 5] = [
    ("Sane-Desktop-Polaris.toml", DEFAULT_PROFILE_POLARIS),
    ("Notebook-Universal-Safe.toml", DEFAULT_PROFILE_NOTEBOOK),
    ("hime-config.toml", DEFAULT_PROFILE_HIME),
    ("Pavilion-Gaming-Celsius.toml", DEFAULT_PROFILE_PAVILION),
    ("Polaris-VAAPI-ZeroBFrames.toml", DEFAULT_PROFILE_POLARIS_VAAPI),
];

/// Siembra los 5 perfiles por defecto embebidos en el binario si aún no existen en el directorio.
pub fn seed_default_profiles() {
    let dir = profiles_dir();
    let _ = fs::create_dir_all(&dir);

    for (file_name, toml_str) in DEFAULT_PROFILES {
        let dest = dir.join(file_name);
        if !dest.exists() {
            let _ = fs::write(&dest, toml_str);
        }
    }
}

/// Inicializa los perfiles por defecto si aún no existen en el directorio.
pub fn init_default_profiles() {
    seed_default_profiles();
}

/// Lista los nombres de los perfiles disponibles (sin extensión .toml).
pub fn list_profiles() -> Vec<String> {
    let dir = profiles_dir();
    let mut names = Vec::new();

    if let Ok(entries) = fs::read_dir(&dir) {
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

/// Carga un perfil por nombre desde el directorio de perfiles.
pub fn load_profile(name: &str) -> Result<HardwareProfile, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err("Nombre de perfil no válido o contiene caracteres no permitidos.".to_string());
    }

    let file_name = if trimmed.ends_with(".toml") {
        trimmed.to_string()
    } else {
        format!("{}.toml", trimmed)
    };
    let path = profiles_dir().join(&file_name);

    let content = if path.exists() {
        fs::read_to_string(&path)
            .map_err(|e| format!("Error al leer perfil '{}': {}", path.display(), e))?
    } else {
        // Fallback a los perfiles embebidos por defecto si aún no existen físicamente en disco
        DEFAULT_PROFILES
            .iter()
            .find(|(fname, _)| fname.eq_ignore_ascii_case(&file_name))
            .map(|(_, content)| (*content).to_string())
            .ok_or_else(|| format!("Error al leer perfil '{}': Archivo no encontrado", path.display()))?
    };

    toml::from_str::<HardwareProfile>(&content)
        .map_err(|e| format!("Error al parsear TOML del perfil: {}", e))
}

/// Guarda un perfil en el directorio de perfiles.
pub fn save_profile(profile: &HardwareProfile) -> Result<(), String> {
    let dir = profiles_dir();
    fs::create_dir_all(&dir)
        .map_err(|e| format!("No se pudo crear directorio de perfiles: {}", e))?;

    let safe_name = profile.name.trim().replace(['/', '\\', ' '], "_");
    let path = dir.join(format!("{}.toml", safe_name));

    let content = toml::to_string_pretty(profile)
        .map_err(|e| format!("Error al serializar perfil: {}", e))?;

    fs::write(&path, content)
        .map_err(|e| format!("Error al guardar archivo '{}': {}", path.display(), e))?;

    Ok(())
}

/// Exporta un perfil a una ruta arbitraria (para compartir con otros usuarios).
pub fn export_profile(profile: &HardwareProfile, dest: &Path) -> Result<(), String> {
    let content = toml::to_string_pretty(profile)
        .map_err(|e| format!("Error al serializar perfil: {}", e))?;

    fs::write(dest, content)
        .map_err(|e| format!("Error al exportar archivo '{}': {}", dest.display(), e))?;

    Ok(())
}

/// Importa un archivo de perfil externo `.toml` y lo copia a la carpeta de perfiles locales.
pub fn import_profile(source: &Path) -> Result<HardwareProfile, String> {
    let content = fs::read_to_string(source)
        .map_err(|e| format!("Error al leer archivo externo '{}': {}", source.display(), e))?;

    let profile: HardwareProfile = toml::from_str(&content)
        .map_err(|e| format!("El archivo no contiene un formato de perfil válido: {}", e))?;

    save_profile(&profile)?;
    Ok(profile)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_serialization() {
        let p = HardwareProfile::default();
        let s = toml::to_string(&p).expect("serialize");
        let p2: HardwareProfile = toml::from_str(&s).expect("deserialize");
        assert_eq!(p.name, p2.name);
        assert_eq!(p.codec, p2.codec);
        assert_eq!(p.fps, p2.fps);
    }

    #[test]
    fn test_path_traversal_protection() {
        assert!(load_profile("../../../etc/passwd").is_err());
        assert!(load_profile("..").is_err());
        assert!(load_profile("").is_err());
        assert!(load_profile("   ").is_err());
        assert!(load_profile("subdir/test").is_err());
    }

    #[test]
    fn test_init_default_profiles_creates_templates() {
        init_default_profiles();
        let profiles = list_profiles();
        assert!(profiles.contains(&"Sane-Desktop-Polaris".to_string()));
        assert!(profiles.contains(&"Notebook-Universal-Safe".to_string()));
        assert!(profiles.contains(&"hime-config".to_string()));
        assert!(profiles.contains(&"Pavilion-Gaming-Celsius".to_string()));
        assert!(profiles.contains(&"Polaris-VAAPI-ZeroBFrames".to_string()));
    }

    #[test]
    fn test_seed_and_parse_all_default_profiles() {
        for (filename, raw_toml) in DEFAULT_PROFILES {
            let profile: HardwareProfile = toml::from_str(raw_toml)
                .unwrap_or_else(|e| panic!("Error al parsear perfil por defecto {}: {}", filename, e));
            assert!(!profile.name.is_empty());
            assert!(!profile.description.is_empty());
            assert!(!profile.codec.is_empty());
            assert!(profile.fps > 0);
            assert!(profile.max_memory_mb > 0);
        }

        // Validación específica del perfil Polaris VA-API Zero B-Frames
        let polaris_vaapi: HardwareProfile = toml::from_str(DEFAULT_PROFILE_POLARIS_VAAPI)
            .expect("Polaris-VAAPI-ZeroBFrames debe parsearse limpiamente");
        assert_eq!(polaris_vaapi.name, "Polaris-VAAPI-ZeroBFrames");
        assert_eq!(polaris_vaapi.codec, "h264_vaapi");
        assert_eq!(polaris_vaapi.fps, 60);

        // Validación específica del perfil térmico Pavilion Gaming - Celsius
        let pavilion: HardwareProfile = toml::from_str(DEFAULT_PROFILE_PAVILION)
            .expect("Pavilion-Gaming-Celsius debe parsearse limpiamente");
        assert_eq!(pavilion.name, "Pavilion-Gaming-Celsius");
        assert_eq!(pavilion.fps, 30);
        assert_eq!(pavilion.resolution, "1280x720");
        assert_eq!(pavilion.max_memory_mb, 350);
        assert_eq!(pavilion.codec, "libx264");
        assert_eq!(pavilion.preset, "ultrafast");
        assert_eq!(pavilion.crf, 25);

        seed_default_profiles();
        let loaded = load_profile("Pavilion-Gaming-Celsius").expect("Debe cargar Pavilion-Gaming-Celsius");
        assert_eq!(loaded.name, "Pavilion-Gaming-Celsius");
        assert_eq!(loaded.fps, 30);
    }
}
