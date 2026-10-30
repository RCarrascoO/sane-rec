use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const GITHUB_REPO: &str = "RCarrascoO/sane-rec";

#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    pub version: String,
    pub html_url: String,
    pub tarball_url: Option<String>,
}

/// Parsea un string de versión "v0.2.1" o "0.2.1" a una tupla (major, minor, patch).
fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
    let clean = v.trim().trim_start_matches('v');
    let parts: Vec<&str> = clean.split('.').collect();
    if parts.len() >= 2 {
        let major = parts[0].parse::<u32>().ok()?;
        let minor = parts[1].parse::<u32>().ok()?;
        let patch = parts.get(2).and_then(|p| p.parse::<u32>().ok()).unwrap_or(0);
        Some((major, minor, patch))
    } else {
        None
    }
}

/// Compara si `remote` es estrictamente más nuevo que `local`.
pub fn is_newer_version(remote: &str, local: &str) -> bool {
    match (parse_version(remote), parse_version(local)) {
        (Some(r), Some(l)) => r > l,
        _ => remote != local && !remote.is_empty(),
    }
}

/// Consulta GitHub Releases de forma no bloqueante para ver si hay una versión superior disponible.
pub fn check_for_updates() -> Option<ReleaseInfo> {
    let current_version = env!("CARGO_PKG_VERSION");
    let url = format!("https://api.github.com/repos/{}/releases/latest", GITHUB_REPO);

    let output = Command::new("curl")
        .args([
            "-s",
            "--connect-timeout",
            "4",
            "--max-time",
            "8",
            "-H",
            "User-Agent: sane-rec-updater",
            &url,
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let json_str = String::from_utf8_lossy(&output.stdout);
    let val: Value = serde_json::from_str(&json_str).ok()?;

    let tag_name = val.get("tag_name")?.as_str()?;
    let html_url = val.get("html_url")?.as_str()?.to_string();

    if !is_newer_version(tag_name, current_version) {
        return None;
    }

    // Busca en los assets un archivo .tar.gz para la arquitectura activa de Linux
    let mut tarball_url = None;
    let arch = std::env::consts::ARCH; // ej. "x86_64"
    if let Some(assets) = val.get("assets").and_then(|a| a.as_array()) {
        for asset in assets {
            if let Some(name) = asset.get("name").and_then(|n| n.as_str()) {
                let matches_arch = name.contains(arch) || (arch == "x86_64" && name.contains("amd64"));
                if name.ends_with(".tar.gz") && name.contains("linux") && matches_arch {
                    tarball_url = asset.get("browser_download_url").and_then(|u| u.as_str()).map(|s| s.to_string());
                    break;
                }
            }
        }
    }

    Some(ReleaseInfo {
        version: tag_name.to_string(),
        html_url,
        tarball_url,
    })
}

/// Descarga el release y actualiza el binario de sane-rec en el sistema mediante reemplazo atómico POSIX.
pub fn perform_update() -> Result<String, String> {
    let info = check_for_updates()
        .ok_or_else(|| "Ya estás utilizando la versión más reciente de sane-rec o no se pudo consultar GitHub.".to_string())?;

    let download_url = info.tarball_url
        .ok_or_else(|| format!("El release {} no cuenta con un archivo compilado para linux-{}.", info.version, std::env::consts::ARCH))?;

    // Directorio temporal seguro aislado con PID del proceso actual
    let tmp_dir = std::env::temp_dir().join(format!("sane-rec-update-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp_dir);
    fs::create_dir_all(&tmp_dir)
        .map_err(|e| format!("Error al crear carpeta temporal de actualización: {}", e))?;

    let tar_file = tmp_dir.join("release.tar.gz");

    // Descarga con curl agregando --fail para rechazar códigos HTTP 4xx/5xx
    let status = Command::new("curl")
        .args([
            "-L",
            "--fail",
            "--connect-timeout",
            "10",
            "--max-time",
            "120",
            "-o",
            tar_file.to_str().unwrap(),
            &download_url,
        ])
        .status()
        .map_err(|e| format!("Error al descargar actualización: {}", e))?;

    if !status.success() {
        let _ = fs::remove_dir_all(&tmp_dir);
        return Err("La descarga del paquete de actualización falló (servidor retornó error o timeout).".to_string());
    }

    // Descomprime con tar
    let status_tar = Command::new("tar")
        .args([
            "-xzf",
            tar_file.to_str().unwrap(),
            "-C",
            tmp_dir.to_str().unwrap(),
        ])
        .status()
        .map_err(|e| format!("Error al descomprimir paquete: {}", e))?;

    if !status_tar.success() {
        let _ = fs::remove_dir_all(&tmp_dir);
        return Err("Error al extraer el archivo comprimido del release.".to_string());
    }

    // Localizar el binario extraído
    let new_binary = if tmp_dir.join("sane-rec").is_file() {
        tmp_dir.join("sane-rec")
    } else {
        find_binary_recursive(&tmp_dir)?
    };

    // Determina la ruta destino: resuelve el ejecutable actual o ~/.local/bin/sane-rec
    let current_exe = std::env::current_exe().unwrap_or_else(|_| {
        if let Some(user_dirs) = directories::BaseDirs::new() {
            user_dirs.home_dir().join(".local").join("bin").join("sane-rec")
        } else {
            PathBuf::from("/home/sane/.local/bin/sane-rec")
        }
    });

    let target_dest = if current_exe.exists() {
        current_exe
    } else {
        if let Some(user_dirs) = directories::BaseDirs::new() {
            user_dirs.home_dir().join(".local").join("bin").join("sane-rec")
        } else {
            PathBuf::from("/home/sane/.local/bin/sane-rec")
        }
    };

    let dest_dir = target_dest.parent().unwrap_or_else(|| Path::new("."));
    let _ = fs::create_dir_all(dest_dir);
    let staged_file = dest_dir.join(format!(".sane-rec-staged-{}", std::process::id()));

    // Copiar al staging en el mismo sistema de archivos destino para permitir rename atómico
    fs::copy(&new_binary, &staged_file)
        .map_err(|e| {
            let _ = fs::remove_dir_all(&tmp_dir);
            format!("Error al preparar archivo en '{}': {}", dest_dir.display(), e)
        })?;

    // Asignar permisos de ejecución al staging antes de reemplazar
    let _ = Command::new("chmod").args(["+x", staged_file.to_str().unwrap()]).status();

    // Reemplazo atómico POSIX
    fs::rename(&staged_file, &target_dest)
        .map_err(|e| {
            let _ = fs::remove_file(&staged_file);
            let _ = fs::remove_dir_all(&tmp_dir);
            format!("Error al reemplazar atómicamente el ejecutable '{}': {}", target_dest.display(), e)
        })?;

    let _ = fs::remove_dir_all(&tmp_dir);
    let _ = fs::remove_dir_all(&tmp_dir);

    Ok(info.version)
}

fn find_binary_recursive(dir: &Path) -> Result<PathBuf, String> {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.file_name().and_then(|n| n.to_str()) == Some("sane-rec") {
                return Ok(path);
            } else if path.is_dir() {
                if let Ok(found) = find_binary_recursive(&path) {
                    return Ok(found);
                }
            }
        }
    }
    Err("No se encontró el ejecutable 'sane-rec' dentro del paquete descargado.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_comparison() {
        assert!(is_newer_version("v0.2.0", "0.1.0"));
        assert!(is_newer_version("0.1.1", "0.1.0"));
        assert!(is_newer_version("v1.0.0", "0.9.9"));
        assert!(!is_newer_version("v0.1.0", "0.1.0"));
        assert!(!is_newer_version("v0.0.9", "0.1.0"));
    }
}
