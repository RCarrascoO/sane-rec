//! Módulo de Benchmark y Telemetría de Grabación para sane-rec.
//!
//! Genera un informe exhaustivo (.benchmark.txt) junto al archivo de vídeo producido
//! (grabación continua o repetición instantánea), auditando:
//! - Tasa de cuadros real vs. objetivo (FPS promedio y estabilidad).
//! - Cuadros descartados (drop) y duplicados (dup).
//! - Velocidad relativa de codificación (speed multiplier).
//! - Verificación de pistas de audio y vídeo (códec, resolución, canales, sample rate).
//! - Diagnóstico de rendimiento del pipeline (DXGI/ddagrab vs gdigrab vs Wayland).

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::config::Config;

/// Contexto de ejecución para generar el informe de telemetría de benchmark.
pub struct BenchmarkContext<'a> {
    pub output_path: &'a Path,
    pub config: &'a Config,
    pub encoder_stderr: Option<&'a str>,
    pub capture_backend: &'a str,
    pub is_replay_clip: bool,
    pub elapsed_duration: Option<Duration>,
}

#[derive(serde::Deserialize, Default, Debug)]
pub struct FfprobeOutput {
    #[serde(default)]
    pub streams: Vec<FfprobeStream>,
    #[serde(default)]
    pub format: Option<FfprobeFormat>,
}

#[derive(serde::Deserialize, Default, Debug)]
pub struct FfprobeStream {
    #[serde(default)]
    pub codec_type: String,
    #[serde(default)]
    pub codec_name: String,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub r_frame_rate: Option<String>,
    #[serde(default)]
    pub avg_frame_rate: Option<String>,
    #[serde(default)]
    pub nb_frames: Option<String>,
    #[serde(default)]
    pub sample_rate: Option<String>,
    #[serde(default)]
    pub channels: Option<u32>,
}

#[derive(serde::Deserialize, Default, Debug)]
pub struct FfprobeFormat {
    #[serde(default)]
    pub duration: Option<String>,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub bit_rate: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct EncoderStderrTelemetry {
    pub last_fps: Option<f64>,
    pub total_dropped: Option<u64>,
    pub total_duplicated: Option<u64>,
    pub speed_ratio: Option<String>,
    pub bitrate_kbps: Option<String>,
    pub total_frames: Option<u64>,
}

/// Parsea una fracción de framerate de FFmpeg (ej: "60/1", "30000/1001", "25/1") a f64.
pub fn parse_fraction_fps(frac_str: &str) -> Option<f64> {
    let parts: Vec<&str> = frac_str.trim().split('/').collect();
    if parts.len() == 2 {
        let num = parts[0].parse::<f64>().ok()?;
        let den = parts[1].parse::<f64>().ok()?;
        if den > 0.0 {
            Some(num / den)
        } else {
            None
        }
    } else {
        frac_str.trim().parse::<f64>().ok()
    }
}

/// Extrae métricas de telemetría desde el texto acumulado del stderr del codificador (FFmpeg / wf-recorder).
pub fn parse_stderr_telemetry(stderr_text: &str) -> EncoderStderrTelemetry {
    let mut telem = EncoderStderrTelemetry::default();

    for line in stderr_text.lines().rev() {
        if line.contains("fps=") || line.contains("speed=") || line.contains("frame=") {
            // Analizar tokens en la línea de progreso
            let tokens: Vec<&str> = line.split_whitespace().collect();
            for i in 0..tokens.len() {
                let token = tokens[i];
                if token.starts_with("fps=") && telem.last_fps.is_none() {
                    let val = token.strip_prefix("fps=").unwrap_or("");
                    if let Ok(v) = val.parse::<f64>() {
                        telem.last_fps = Some(v);
                    } else if i + 1 < tokens.len() {
                        if let Ok(v) = tokens[i + 1].parse::<f64>() {
                            telem.last_fps = Some(v);
                        }
                    }
                } else if token.starts_with("drop=") && telem.total_dropped.is_none() {
                    let val = token.strip_prefix("drop=").unwrap_or("");
                    if let Ok(v) = val.parse::<u64>() {
                        telem.total_dropped = Some(v);
                    } else if i + 1 < tokens.len() {
                        if let Ok(v) = tokens[i + 1].parse::<u64>() {
                            telem.total_dropped = Some(v);
                        }
                    }
                } else if token.starts_with("dup=") && telem.total_duplicated.is_none() {
                    let val = token.strip_prefix("dup=").unwrap_or("");
                    if let Ok(v) = val.parse::<u64>() {
                        telem.total_duplicated = Some(v);
                    } else if i + 1 < tokens.len() {
                        if let Ok(v) = tokens[i + 1].parse::<u64>() {
                            telem.total_duplicated = Some(v);
                        }
                    }
                } else if token.starts_with("speed=") && telem.speed_ratio.is_none() {
                    let val = token.strip_prefix("speed=").unwrap_or("");
                    if !val.is_empty() {
                        telem.speed_ratio = Some(val.to_string());
                    } else if i + 1 < tokens.len() {
                        telem.speed_ratio = Some(tokens[i + 1].to_string());
                    }
                } else if token.starts_with("bitrate=") && telem.bitrate_kbps.is_none() {
                    let val = token.strip_prefix("bitrate=").unwrap_or("");
                    if !val.is_empty() {
                        telem.bitrate_kbps = Some(val.to_string());
                    } else if i + 1 < tokens.len() {
                        telem.bitrate_kbps = Some(tokens[i + 1].to_string());
                    }
                } else if token.starts_with("frame=") && telem.total_frames.is_none() {
                    let val = token.strip_prefix("frame=").unwrap_or("");
                    if let Ok(v) = val.parse::<u64>() {
                        telem.total_frames = Some(v);
                    } else if i + 1 < tokens.len() {
                        if let Ok(v) = tokens[i + 1].parse::<u64>() {
                            telem.total_frames = Some(v);
                        }
                    }
                }
            }

            if telem.last_fps.is_some() && telem.speed_ratio.is_some() {
                break;
            }
        }
    }

    telem
}

/// Ejecuta ffprobe para inspeccionar exhaustivamente las propiedades y flujos del archivo.
pub fn probe_video_file(path: &Path) -> Result<FfprobeOutput, String> {
    let ffprobe_bin = crate::discovery::resolve_binary("ffprobe");
    let mut cmd = Command::new(ffprobe_bin);
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
            "format=duration,size,bit_rate:stream=codec_type,codec_name,width,height,r_frame_rate,avg_frame_rate,nb_frames,sample_rate,channels",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .map_err(|e| format!("Fallo al ejecutar ffprobe: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "ffprobe devolvió error (código {:?}): {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    serde_json::from_slice::<FfprobeOutput>(&output.stdout)
        .map_err(|e| format!("Fallo al deserializar JSON de ffprobe: {}", e))
}

/// Genera el informe de telemetría y benchmark (.benchmark.txt) junto al video de salida.
pub fn generate_benchmark_report(ctx: &BenchmarkContext) -> Result<PathBuf, String> {
    if !ctx.output_path.exists() {
        return Err(format!(
            "El archivo de salida '{}' no existe para generar el benchmark",
            ctx.output_path.display()
        ));
    }

    let file_size_bytes = std::fs::metadata(ctx.output_path)
        .map(|m| m.len())
        .unwrap_or(0);
    let file_size_mb = (file_size_bytes as f64) / (1024.0 * 1024.0);

    let probe_res = probe_video_file(ctx.output_path);
    let probe_err = probe_res.as_ref().err().cloned();
    let probe_data = probe_res.as_ref().ok();

    let video_stream = probe_data
        .and_then(|probe| probe.streams.iter().find(|stream| stream.codec_type == "video"));
    let audio_stream = probe_data
        .and_then(|probe| probe.streams.iter().find(|stream| stream.codec_type == "audio"));

    let duration_secs = probe_data
        .and_then(|probe| probe.format.as_ref())
        .and_then(|fmt| fmt.duration.as_ref())
        .and_then(|dur_str| dur_str.parse::<f64>().ok())
        .or_else(|| ctx.elapsed_duration.map(|elapsed| elapsed.as_secs_f64()))
        .unwrap_or(0.0);

    let stderr_telem = ctx
        .encoder_stderr
        .map(parse_stderr_telemetry)
        .unwrap_or_default();

    let target_fps = ctx.config.video.fps.max(15) as f64;
    let real_fps = video_stream
        .and_then(|video| video.avg_frame_rate.as_ref().and_then(|fps_str| parse_fraction_fps(fps_str)))
        .or_else(|| video_stream.and_then(|video| video.r_frame_rate.as_ref().and_then(|fps_str| parse_fraction_fps(fps_str))))
        .or(stderr_telem.last_fps)
        .unwrap_or(0.0);

    let fps_ratio = if target_fps > 0.0 {
        (real_fps / target_fps) * 100.0
    } else {
        100.0
    };

    let total_frames = video_stream
        .and_then(|video| video.nb_frames.as_ref().and_then(|frames_str| frames_str.parse::<u64>().ok()))
        .or(stderr_telem.total_frames)
        .unwrap_or_else(|| (real_fps * duration_secs).round() as u64);

    let dropped = stderr_telem.total_dropped.unwrap_or(0);
    let duplicated = stderr_telem.total_duplicated.unwrap_or(0);

    let audio_expected = ctx.config.audio.capture_desktop || ctx.config.audio.capture_mic;
    let audio_ok = !audio_expected || audio_stream.is_some() || probe_err.is_some();

    // Determinación del veredicto del benchmark
    let (verdict_title, verdict_desc) = if let Some(ref _err_msg) = probe_err {
        (
            "AVISO / INSPECCIÓN LIMITADA",
            "ffprobe no estuvo disponible para introspección binaria detallada; se conservan métricas extraídas del codificador.",
        )
    } else if real_fps >= (target_fps * 0.95) && dropped <= 2 && audio_ok {
        (
            "EXCELENTE / PASS (Rendimiento Óptimo)",
            "La tasa de cuadros se mantuvo estable en el objetivo sin pérdidas apreciables de sincronía ni cuellos de botella.",
        )
    } else if real_fps >= (target_fps * 0.80) && dropped <= 30 && audio_ok {
        (
            "BUENO / ACEPTABLE (Rendimiento Estable)",
            "Se registraron mínimas caídas de cuadros esporádicas pero el pipeline se mantuvo en tiempo real.",
        )
    } else {
        (
            "ADVERTENCIA / REVIEW (Cuello de Botella Detectado)",
            "Se detectaron caídas notables de FPS, cuadros descartados o ausencia de pista de audio. Considere usar perfil SaneSafe CPU o revisar la resolución.",
        )
    };

    let now_str = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let os_name = if cfg!(target_os = "windows") {
        "Windows (Win32 / DWM)"
    } else {
        "Linux (Wayland / PipeWire)"
    };

    let mut report = String::new();
    report.push_str("================================================================================\n");
    report.push_str("                    SANE-REC BENCHMARK & TELEMETRY REPORT\n");
    report.push_str("================================================================================\n");
    report.push_str(&format!("Fecha y Hora         : {}\n", now_str));
    report.push_str(&format!("Versión sane-rec     : v{}\n", env!("CARGO_PKG_VERSION")));
    report.push_str(&format!("Sistema Operativo    : {}\n", os_name));
    report.push_str(&format!("Tipo de Grabación    : {}\n", if ctx.is_replay_clip { "Clip de Repetición Instantánea (Replay)" } else { "Grabación Continua Directa" }));
    report.push_str(&format!("Archivo de Video     : {}\n", ctx.output_path.display()));
    report.push_str(&format!("Tamaño de Archivo    : {:.2} MB ({} bytes)\n", file_size_mb, file_size_bytes));
    report.push_str(&format!("Duración del Clip    : {:.2} segundos\n\n", duration_secs));

    report.push_str("--- CONFIGURACIÓN Y PIPELINE ---\n");
    report.push_str(&format!("Backend de Captura   : {}\n", ctx.capture_backend));
    report.push_str(&format!("Perfil de Encoder    : {}\n", ctx.config.profile.active_profile));
    report.push_str(&format!("Códec Solicitado     : {}\n", if ctx.config.video.codec.is_empty() { "auto" } else { &ctx.config.video.codec }));
    report.push_str(&format!("FPS Objetivo         : {:.2} FPS\n", target_fps));
    report.push_str(&format!("Resolución/Escala    : {}\n", if ctx.config.video.scale.is_empty() { "Nativa (100%)" } else { &ctx.config.video.scale }));
    report.push_str(&format!("Audio Sistema        : {}\n", if ctx.config.audio.capture_desktop { "Activado" } else { "Desactivado" }));
    report.push_str(&format!("Audio Micrófono      : {}\n\n", if ctx.config.audio.capture_mic { "Activado" } else { "Desactivado" }));

    report.push_str("--- ANÁLISIS DE PISTAS (FFPROBE) ---\n");
    if let Some(video) = video_stream {
        report.push_str(&format!("Pista de Video       : DETECTADA [OK]\n"));
        report.push_str(&format!("  - Códec Real       : {}\n", video.codec_name));
        report.push_str(&format!("  - Resolución       : {}x{}\n", video.width.unwrap_or(0), video.height.unwrap_or(0)));
        report.push_str(&format!("  - FPS Real (Probe) : {:.2} FPS ({:.1}% del objetivo)\n", real_fps, fps_ratio));
        report.push_str(&format!("  - Cuadros Totales  : {} frames\n", total_frames));
    } else if let Some(ref err) = probe_err {
        report.push_str(&format!("Pista de Video       : NO ANALIZADA (ffprobe no disponible: {})\n", err));
    } else {
        report.push_str("Pista de Video       : NO DETECTADA [ERROR]\n");
    }

    if let Some(audio) = audio_stream {
        report.push_str(&format!("Pista de Audio       : DETECTADA [OK]\n"));
        report.push_str(&format!("  - Códec Audio      : {}\n", audio.codec_name));
        report.push_str(&format!("  - Frecuencia (Hz)  : {} Hz\n", audio.sample_rate.as_deref().unwrap_or("48000")));
        report.push_str(&format!("  - Canales          : {}\n", audio.channels.unwrap_or(2)));
    } else if let Some(ref err) = probe_err {
        report.push_str(&format!("Pista de Audio       : NO ANALIZADA ({})\n", err));
    } else if audio_expected {
        report.push_str("Pista de Audio       : AUSENTE [ADVERTENCIA: Se solicitó audio pero la pista no se encontró]\n");
    } else {
        report.push_str("Pista de Audio       : Omitida por configuración (Silencioso)\n");
    }

    report.push_str("\n--- TELEMETRÍA DE RENDIMIENTO DEL ENCODER ---\n");
    if let Some(fps) = stderr_telem.last_fps {
        report.push_str(&format!("FPS de Codificación  : {:.2} FPS\n", fps));
    } else {
        report.push_str(&format!("FPS de Codificación  : {:.2} FPS (derivado de pista)\n", real_fps));
    }
    report.push_str(&format!("Cuadros Descartados  : {} (drop)\n", dropped));
    report.push_str(&format!("Cuadros Duplicados   : {} (dup)\n", duplicated));
    if let Some(ref speed) = stderr_telem.speed_ratio {
        report.push_str(&format!("Multiplicador Speed  : {}\n", speed));
    }
    if let Some(ref br) = stderr_telem.bitrate_kbps {
        report.push_str(&format!("Tasa de Bits Final   : {}\n", br));
    }

    report.push_str("\n--- DICTAMEN DE SALUD Y RENDIMIENTO ---\n");
    report.push_str(&format!("ESTADO GENERAL       : {}\n", verdict_title));
    report.push_str(&format!("Diagnóstico          : {}\n", verdict_desc));
    report.push_str("================================================================================\n");

    let benchmark_path = ctx.output_path.with_extension("benchmark.txt");
    let mut file = File::create(&benchmark_path)
        .map_err(|e| format!("No se pudo crear el archivo de benchmark '{}': {}", benchmark_path.display(), e))?;

    file.write_all(report.as_bytes())
        .map_err(|e| format!("Error al escribir el reporte de benchmark: {}", e))?;

    // Notificación de escritorio al usuario
    #[cfg(not(target_os = "windows"))]
    let _ = notify_rust::Notification::new()
        .appname("sane-rec")
        .summary("📊 Benchmark y Telemetría Generada")
        .body(&format!(
            "Estado: {}\nGuardado en: {}",
            verdict_title,
            benchmark_path.file_name().unwrap_or_default().to_string_lossy()
        ))
        .icon("dialog-information")
        .show();

    Ok(benchmark_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_fraction_fps() {
        assert_eq!(parse_fraction_fps("60/1"), Some(60.0));
        assert_eq!(parse_fraction_fps("30/1"), Some(30.0));
        let p = parse_fraction_fps("30000/1001").unwrap();
        assert!((p - 29.97).abs() < 0.01);
        assert_eq!(parse_fraction_fps("60"), Some(60.0));
        assert_eq!(parse_fraction_fps("invalid"), None);
    }

    #[test]
    fn test_parse_stderr_telemetry() {
        let stderr = r#"
frame=   10 fps=0.0 q=0.0 size=       0kB time=00:00:00.16 bitrate=   0.0kbits/s speed=0.33x    
frame=  120 fps= 59.8 q=23.0 size=    1024kB time=00:00:02.00 bitrate=4194.3kbits/s dup=0 drop=0 speed=1.00x    
frame=  360 fps= 60.0 q=23.0 size=    3200kB time=00:00:06.00 bitrate=4369.0kbits/s dup=2 drop=1 speed=1.00x    
"#;
        let telem = parse_stderr_telemetry(stderr);
        assert_eq!(telem.last_fps, Some(60.0));
        assert_eq!(telem.total_dropped, Some(1));
        assert_eq!(telem.total_duplicated, Some(2));
        assert_eq!(telem.speed_ratio.as_deref(), Some("1.00x"));
        assert_eq!(telem.total_frames, Some(360));
    }
}
