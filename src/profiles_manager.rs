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

/// Devuelve la ruta al directorio de perfiles (`~/.config/sane-rec/profiles/`).
pub fn profiles_dir() -> PathBuf {
    if let Some(user_dirs) = directories::BaseDirs::new() {
        user_dirs.config_dir().join("sane-rec").join("profiles")
    } else {
        PathBuf::from("/home/sane/.config/sane-rec/profiles")
    }
}

/// Inicializa los perfiles por defecto si aún no existen en el directorio.
pub fn init_default_profiles() {
    let dir = profiles_dir();
    let _ = fs::create_dir_all(&dir);

    let polaris = HardwareProfile {
        name: "Sane-Desktop-Polaris".to_string(),
        description: "Optimizado para PC de escritorio con AMD Polaris (RX 570) y CPU multinúcleo (60 FPS)".to_string(),
        target_device: "AMD Polaris RX 570 / Desktop".to_string(),
        codec: "libx264".to_string(),
        preset: "ultrafast".to_string(),
        crf: 21,
        fps: 60,
        resolution: "1360x768".to_string(),
        max_memory_mb: 150,
        audio_track_mode: 0,
    };

    let notebook = HardwareProfile {
        name: "Notebook-Universal-Safe".to_string(),
        description: "Ahorro de batería y bajo consumo térmico para portátiles con gráficos integrados (30 FPS)".to_string(),
        target_device: "Notebook / Laptop universal".to_string(),
        codec: "libx264".to_string(),
        preset: "superfast".to_string(),
        crf: 23,
        fps: 30,
        resolution: "1280x720".to_string(),
        max_memory_mb: 100,
        audio_track_mode: 0,
    };

    let p_path = dir.join("Sane-Desktop-Polaris.toml");
    if !p_path.exists() {
        let _ = save_profile(&polaris);
    }

    let n_path = dir.join("Notebook-Universal-Safe.toml");
    if !n_path.exists() {
        let _ = save_profile(&notebook);
    }
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
    let path = profiles_dir().join(file_name);

    let content = fs::read_to_string(&path)
        .map_err(|e| format!("Error al leer perfil '{}': {}", path.display(), e))?;

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
    }
}
