use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use crate::discovery::new_hidden_command;

const GITHUB_REPO: &str = "RCarrascoO/sane-rec";

#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    pub version: String,
    pub html_url: String,
    pub tarball_url: Option<String>,
    pub body: String,
}

/// Retorna el contenido del archivo `CHANGELOG.md` embebido en tiempo de compilación.
pub fn get_embedded_changelog() -> &'static str {
    include_str!("../CHANGELOG.md")
}

/// Limpia los encabezados y formato Markdown del changelog para mostrarlo de forma legible en la interfaz Slint.
pub fn clean_changelog_markdown(raw: &str) -> String {
    let mut out = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(h3) = trimmed.strip_prefix("### ") {
            out.push(format!("• {}", h3.trim()));
        } else if let Some(h2) = trimmed.strip_prefix("## ") {
            if !out.is_empty() {
                out.push(String::new());
            }
            out.push(format!("▸ {}", h2.trim()));
        } else if let Some(h1) = trimmed.strip_prefix("# ") {
            out.push(h1.trim().to_string());
        } else if let Some(bullet) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
            let clean_bullet = bullet.replace("**", "");
            out.push(format!("  • {}", clean_bullet.trim()));
        } else {
            out.push(trimmed.replace("**", ""));
        }
    }
    out.join("\n").trim().to_string()
}

/// Elimina el ejecutable antiguo `<current_exe>.old` o `.exe.old` dejado tras una actualización previa en Windows.
pub fn cleanup_old_executable() {
    if let Ok(current_exe) = std::env::current_exe() {
        let old_exe = current_exe.with_extension("exe.old");
        if old_exe.exists() {
            let _ = fs::remove_file(&old_exe);
        }
        let old_generic = PathBuf::from(format!("{}.old", current_exe.display()));
        if old_generic.exists() {
            let _ = fs::remove_file(&old_generic);
        }
    }
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
    cleanup_old_executable();

    let current_version = env!("CARGO_PKG_VERSION");
    let url = format!("https://api.github.com/repos/{}/releases/latest", GITHUB_REPO);

    let output = new_hidden_command("curl")
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

    let raw_body = val
        .get("body")
        .and_then(|b| b.as_str())
        .unwrap_or("")
        .trim();
    let body = if raw_body.is_empty() {
        get_embedded_changelog().to_string()
    } else {
        raw_body.to_string()
    };

    let mut tarball_url = None;

    #[cfg(not(target_os = "windows"))]
    {
        let arch = std::env::consts::ARCH; // ej. "x86_64"
        if let Some(assets) = val.get("assets").and_then(|a| a.as_array()) {
            for asset in assets {
                if let Some(name) = asset.get("name").and_then(|n| n.as_str()) {
                    let lower = name.to_lowercase();
                    let matches_arch =
                        lower.contains(arch) || (arch == "x86_64" && (lower.contains("amd64") || lower.contains("x64")));
                    if lower.ends_with(".tar.gz") && lower.contains("linux") && matches_arch {
                        tarball_url = asset
                            .get("browser_download_url")
                            .and_then(|u| u.as_str())
                            .map(|s| s.to_string());
                        break;
                    }
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(assets) = val.get("assets").and_then(|a| a.as_array()) {
            // Prioridad 1: paquete portable .zip de Windows
            for asset in assets {
                if let Some(name) = asset.get("name").and_then(|n| n.as_str()) {
                    let lower = name.to_lowercase();
                    if lower.ends_with(".zip") && lower.contains("windows") {
                        tarball_url = asset
                            .get("browser_download_url")
                            .and_then(|u| u.as_str())
                            .map(|s| s.to_string());
                        break;
                    }
                }
            }
            // Prioridad 2: instalador .exe con "setup"
            if tarball_url.is_none() {
                for asset in assets {
                    if let Some(name) = asset.get("name").and_then(|n| n.as_str()) {
                        let lower = name.to_lowercase();
                        if lower.ends_with(".exe") && lower.contains("setup") {
                            tarball_url = asset
                                .get("browser_download_url")
                                .and_then(|u| u.as_str())
                                .map(|s| s.to_string());
                            break;
                        }
                    }
                }
            }
        }
    }

    Some(ReleaseInfo {
        version: tag_name.to_string(),
        html_url,
        tarball_url,
        body,
    })
}

/// Descarga el release y actualiza el binario de sane-rec en el sistema (reemplazo atómico POSIX en Linux o actualización nativa en Windows).
pub fn perform_update() -> Result<String, String> {
    cleanup_old_executable();

    let info = check_for_updates().ok_or_else(|| {
        "Ya estás utilizando la versión más reciente de sane-rec o no se pudo consultar GitHub."
            .to_string()
    })?;

    #[cfg(not(target_os = "windows"))]
    {
        let download_url = info.tarball_url.ok_or_else(|| {
            format!(
                "El release {} no cuenta con un archivo compilado para linux-{}.",
                info.version,
                std::env::consts::ARCH
            )
        })?;

        let tmp_dir =
            std::env::temp_dir().join(format!("sane-rec-update-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp_dir);
        fs::create_dir_all(&tmp_dir)
            .map_err(|e| format!("Error al crear carpeta temporal de actualización: {}", e))?;

        let tar_file = tmp_dir.join("release.tar.gz");

        let status = new_hidden_command("curl")
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
            return Err(
                "La descarga del paquete de actualización falló (servidor retornó error o timeout)."
                    .to_string(),
            );
        }

        let status_tar = new_hidden_command("tar")
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

        let new_binary = if tmp_dir.join("sane-rec").is_file() {
            tmp_dir.join("sane-rec")
        } else {
            find_binary_recursive(&tmp_dir)?
        };

        let current_exe = std::env::current_exe().unwrap_or_else(|_| {
            if let Some(user_dirs) = directories::BaseDirs::new() {
                user_dirs
                    .home_dir()
                    .join(".local")
                    .join("bin")
                    .join("sane-rec")
            } else {
                PathBuf::from("/home/sane/.local/bin/sane-rec")
            }
        });

        let target_dest = if current_exe.exists() {
            current_exe
        } else if let Some(user_dirs) = directories::BaseDirs::new() {
            user_dirs
                .home_dir()
                .join(".local")
                .join("bin")
                .join("sane-rec")
        } else {
            PathBuf::from("/home/sane/.local/bin/sane-rec")
        };

        let dest_dir = target_dest.parent().unwrap_or_else(|| Path::new("."));
        let _ = fs::create_dir_all(dest_dir);
        let staged_file = dest_dir.join(format!(".sane-rec-staged-{}", std::process::id()));

        fs::copy(&new_binary, &staged_file).map_err(|e| {
            let _ = fs::remove_dir_all(&tmp_dir);
            format!(
                "Error al preparar archivo en '{}': {}",
                dest_dir.display(),
                e
            )
        })?;

        let _ = new_hidden_command("chmod")
            .args(["+x", staged_file.to_str().unwrap()])
            .status();

        fs::rename(&staged_file, &target_dest).map_err(|e| {
            let _ = fs::remove_file(&staged_file);
            let _ = fs::remove_dir_all(&tmp_dir);
            format!(
                "Error al reemplazar atómicamente el ejecutable '{}': {}",
                target_dest.display(),
                e
            )
        })?;

        let _ = fs::remove_dir_all(&tmp_dir);

        Ok(info.version)
    }

    #[cfg(target_os = "windows")]
    {
        let download_url = info.tarball_url.ok_or_else(|| {
            format!(
                "El release {} no cuenta con un paquete compilado para Windows.",
                info.version
            )
        })?;

        let tmp_dir =
            std::env::temp_dir().join(format!("sane-rec-update-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp_dir);
        fs::create_dir_all(&tmp_dir)
            .map_err(|e| format!("Error al crear carpeta temporal de actualización: {}", e))?;

        let is_exe = download_url.to_lowercase().ends_with(".exe");
        let asset_file = if is_exe {
            tmp_dir.join("sane-rec-setup-x64.exe")
        } else {
            tmp_dir.join("release.zip")
        };

        let status = new_hidden_command("curl")
            .args([
                "-L",
                "--fail",
                "--connect-timeout",
                "10",
                "--max-time",
                "180",
                "-o",
                asset_file.to_str().unwrap(),
                &download_url,
            ])
            .status()
            .map_err(|e| format!("Error al descargar actualización en Windows: {}", e))?;

        if !status.success() {
            let _ = fs::remove_dir_all(&tmp_dir);
            return Err(
                "La descarga del paquete de actualización falló (servidor retornó error o timeout)."
                    .to_string(),
            );
        }

        if is_exe {
            new_hidden_command(&asset_file)
                .arg("/SILENT")
                .spawn()
                .map_err(|e| format!("Error al ejecutar el instalador de actualización: {}", e))?;
            return Ok(info.version);
        }

        // Extraer archivo .zip con tar nativo de Windows 10/11 o fallback a PowerShell Expand-Archive
        let tar_ok = new_hidden_command("tar")
            .args([
                "-xf",
                asset_file.to_str().unwrap(),
                "-C",
                tmp_dir.to_str().unwrap(),
            ])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if !tar_ok {
            let ps_cmd = format!(
                "Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
                asset_file.display(),
                tmp_dir.display()
            );
            let ps_status = new_hidden_command("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", &ps_cmd])
                .status()
                .map_err(|e| format!("Error al descomprimir archivo ZIP: {}", e))?;
            if !ps_status.success() {
                let _ = fs::remove_dir_all(&tmp_dir);
                return Err("Error al extraer el archivo ZIP del release.".to_string());
            }
        }

        let new_binary = if tmp_dir.join("sane-rec.exe").is_file() {
            tmp_dir.join("sane-rec.exe")
        } else {
            find_binary_recursive(&tmp_dir)?
        };

        let current_exe = std::env::current_exe()
            .map_err(|e| format!("No se pudo determinar la ruta del ejecutable actual: {}", e))?;

        let old_exe = current_exe.with_extension("exe.old");
        let _ = fs::remove_file(&old_exe);

        fs::rename(&current_exe, &old_exe).map_err(|e| {
            let _ = fs::remove_dir_all(&tmp_dir);
            format!(
                "No se pudo renombrar el ejecutable en uso '{}': {}",
                current_exe.display(),
                e
            )
        })?;

        if let Err(e) = fs::copy(&new_binary, &current_exe) {
            let _ = fs::rename(&old_exe, &current_exe);
            let _ = fs::remove_dir_all(&tmp_dir);
            return Err(format!(
                "Error al copiar el nuevo ejecutable '{}': {}",
                current_exe.display(),
                e
            ));
        }

        let _ = fs::remove_dir_all(&tmp_dir);
        Ok(info.version)
    }
}

fn find_binary_recursive(dir: &Path) -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    const TARGET_BIN: &str = "sane-rec.exe";
    #[cfg(not(target_os = "windows"))]
    const TARGET_BIN: &str = "sane-rec";

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.file_name().and_then(|n| n.to_str()) == Some(TARGET_BIN) {
                return Ok(path);
            } else if path.is_dir() {
                if let Ok(found) = find_binary_recursive(&path) {
                    return Ok(found);
                }
            }
        }
    }
    Err(format!(
        "No se encontró el ejecutable '{}' dentro del paquete descargado.",
        TARGET_BIN
    ))
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

    #[test]
    fn test_embedded_changelog_and_cleaning() {
        let raw = get_embedded_changelog();
        assert!(raw.contains("v0.5.1"));
        assert!(raw.contains("v0.5.0"));
        let cleaned = clean_changelog_markdown(raw);
        assert!(cleaned.contains("▸ v0.5.1"));
        assert!(cleaned.contains("▸ v0.5.0"));
        assert!(!cleaned.contains("## "));
    }
}
