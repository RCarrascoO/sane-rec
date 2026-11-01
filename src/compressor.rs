//! Módulo de compresión inteligente para redes y Discord (Modo Doble Clip).
//!
//! Calcula la tasa de bits (bitrate) óptima según la duración del clip para garantizar
//! que el archivo resultante no exceda el límite deseado (ej. 10 MB para Discord Gratuito)
//! manteniendo la mayor fidelidad de imagen y sonido posible mediante escalado adaptativo.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Obtiene la duración exacta de un archivo de vídeo en segundos usando ffprobe.
pub fn get_video_duration_secs(path: &Path) -> Option<f64> {
    let binary = crate::discovery::resolve_binary("ffprobe");
    let mut cmd = Command::new(binary);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let output = cmd
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .ok()?;

    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        text.parse::<f64>().ok()
    } else {
        None
    }
}

/// Genera la ruta de destino para el clip ligero de Discord.
/// Ej: `/videos/Replay_2026-10-31_20-00-00.mp4` -> `/videos/Replay_2026-10-31_20-00-00_discord.mp4`
pub fn get_discord_clip_path(original_path: &Path) -> PathBuf {
    let stem = original_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("clip");
    let ext = original_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mp4");
    let parent = original_path.parent().unwrap_or_else(|| Path::new("."));

    parent.join(format!("{}_discord.{}", stem, ext))
}

/// Calcula los parámetros de bitrate y escalado recomendados para comprimir un clip a un tamaño objetivo.
/// Retorna `(video_kbps, audio_kbps, scale_filter)`
pub fn calculate_compression_parameters(duration_secs: f64, target_mb: f64) -> (u32, u32, &'static str) {
    let duration = duration_secs.max(1.0);

    // Margen de seguridad del 8% para el contenedor MP4/MKV, moov atom y headers de paquete
    let total_bits_budget = target_mb * 8.0 * 1024.0 * 1024.0 * 0.92;
    let total_bps = total_bits_budget / duration;

    // Asignación de tasa de audio según presupuesto total
    let audio_kbps: u32 = if total_bps < 500_000.0 {
        64
    } else if total_bps < 1_000_000.0 {
        96
    } else {
        128
    };

    let audio_bps = (audio_kbps as f64) * 1024.0;
    let video_bps = (total_bps - audio_bps).max(150_000.0);
    let video_kbps = (video_bps / 1024.0).round() as u32;

    // Escala adaptativa inteligente:
    // Si el bitrate disponible es modesto, reducir resolución hace que la imagen se vea nítida
    // en vez de sufrir artefactos en bloques (macroblocking).
    let scale_filter = if video_kbps < 750 {
        "scale=-2:min(480\\,ih)"
    } else if video_kbps < 1800 {
        "scale=-2:min(720\\,ih)"
    } else {
        "scale=-2:min(1080\\,ih)"
    };

    (video_kbps, audio_kbps, scale_filter)
}

/// Comprime un clip de vídeo para ajustarlo a un límite de tamaño específico en megabytes (ej. 10 MB).
pub fn compress_to_target_size(
    input_path: &Path,
    output_path: &Path,
    target_mb: f64,
) -> Result<PathBuf, String> {
    if !input_path.exists() {
        return Err(format!("El archivo de entrada no existe: {}", input_path.display()));
    }

    let duration = get_video_duration_secs(input_path).unwrap_or(30.0);
    let (video_kbps, audio_kbps, scale_filter) = calculate_compression_parameters(duration, target_mb);

    let maxrate = format!("{}k", ((video_kbps as f64) * 1.25).round() as u32);
    let bufsize = format!("{}k", video_kbps * 2);
    let b_v = format!("{}k", video_kbps);
    let b_a = format!("{}k", audio_kbps);

    let binary = crate::discovery::resolve_binary("ffmpeg");
    let mut cmd = Command::new(binary);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let status = cmd
        .args(["-y", "-i"])
        .arg(input_path)
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "fast",
            "-b:v",
            &b_v,
            "-maxrate",
            &maxrate,
            "-bufsize",
            &bufsize,
            "-vf",
            scale_filter,
            "-c:a",
            "aac",
            "-b:a",
            &b_a,
            "-movflags",
            "+faststart",
        ])
        .arg(output_path)
        .output()
        .map_err(|e| format!("Fallo al ejecutar ffmpeg para compresión: {}", e))?;

    if !status.status.success() {
        let err = String::from_utf8_lossy(&status.stderr);
        return Err(format!("Error en ffmpeg al comprimir: {}", err));
    }

    Ok(output_path.to_path_buf())
}

/// Lanza la compresión en un hilo de fondo desacoplado para no bloquear el motor de grabación ni la UI.
pub fn spawn_discord_compression(input_path: PathBuf, target_mb: u32) {
    std::thread::spawn(move || {
        let output_path = get_discord_clip_path(&input_path);
        match compress_to_target_size(&input_path, &output_path, target_mb as f64) {
            Ok(out) => {
                let size_mb = out
                    .metadata()
                    .map(|m| (m.len() as f64) / (1024.0 * 1024.0))
                    .unwrap_or(0.0);

                let filename = out
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("clip_discord.mp4");

                let _ = notify_rust::Notification::new()
                    .appname("sane-rec")
                    .summary("💬 Doble Clip Listo (Discord)")
                    .body(&format!(
                        "Generado: {}\nTamaño: {:.1} MB (límite: {} MB)",
                        filename, size_mb, target_mb
                    ))
                    .icon("video-x-generic")
                    .show();

                crate::engine::play_feedback_sound("message");
            }
            Err(e) => {
                crate::crash_handler::write_crash_report(
                    "Fallo en compresión de clip para Discord",
                    &e,
                );
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_discord_clip_path() {
        let orig = PathBuf::from("/home/user/Videos/Replay_2026-10-31.mp4");
        let discord_path = get_discord_clip_path(&orig);
        assert_eq!(
            discord_path,
            PathBuf::from("/home/user/Videos/Replay_2026-10-31_discord.mp4")
        );
    }

    #[test]
    fn test_calculate_compression_parameters() {
        // Para 30s a 10MB
        let (video_kbps, audio_kbps, scale) = calculate_compression_parameters(30.0, 10.0);
        assert!(video_kbps > 2000);
        assert_eq!(audio_kbps, 128);
        assert_eq!(scale, "scale=-2:min(1080\\,ih)");

        // Para 120s a 10MB (debe reducir escala a 480p o 720p para no pixelarse)
        let (v_long, a_long, s_long) = calculate_compression_parameters(120.0, 10.0);
        assert!(v_long < 1000);
        assert_eq!(a_long, 96);
        assert_eq!(s_long, "scale=-2:min(480\\,ih)");
    }

    #[test]
    fn test_compress_synthetic_video() {
        let temp_dir = std::env::temp_dir().join(format!("sane_rec_compress_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);

        let input_file = temp_dir.join("input.mp4");
        let output_file = temp_dir.join("output_discord.mp4");

        // Crear clip sintético de 2 segundos con audio silencioso
        let _ = Command::new("ffmpeg")
            .args([
                "-y",
                "-f", "lavfi", "-i", "testsrc=duration=2:size=640x360:rate=30",
                "-f", "lavfi", "-i", "anullsrc=channel_layout=stereo:sample_rate=44100",
                "-t", "2",
                "-c:v", "libx264", "-preset", "ultrafast",
                "-c:a", "aac",
            ])
            .arg(&input_file)
            .output();

        if input_file.exists() {
            let res = compress_to_target_size(&input_file, &output_file, 5.0);
            assert!(res.is_ok());
            assert!(output_file.exists());
            let meta = output_file.metadata().unwrap();
            let size_mb = (meta.len() as f64) / (1024.0 * 1024.0);
            assert!(size_mb < 5.0);
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
