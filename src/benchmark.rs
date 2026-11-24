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

/// Estadísticas del búfer de repetición en memoria RAM (Windows) o en disco (Linux).
#[derive(Debug, Clone, Default)]
pub struct RamBufferStats {
    pub segments: usize,
    pub bytes: u64,
    pub oldest: Option<std::time::SystemTime>,
    pub newest: Option<std::time::SystemTime>,
    pub truncated: bool,
}

/// Contexto de ejecución para generar el informe de telemetría de benchmark.
pub struct BenchmarkContext<'a> {
    pub output_path: &'a Path,
    pub config: &'a Config,
    pub encoder_stderr: Option<&'a str>,
    pub executed_command: Option<&'a str>,
    pub capture_backend: &'a str,
    pub is_replay_clip: bool,
    pub elapsed_duration: Option<Duration>,
    pub ram_buffer_stats: Option<RamBufferStats>,
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

#[derive(Debug, Default)]
struct ReplayTelemetry {
    segments: usize,
    bytes: u64,
    oldest: Option<std::time::SystemTime>,
    newest: Option<std::time::SystemTime>,
    truncated: bool,
}

fn replay_telemetry(ctx: &BenchmarkContext) -> ReplayTelemetry {
    if !ctx.is_replay_clip {
        return ReplayTelemetry::default();
    }

    if let Some(ref stats) = ctx.ram_buffer_stats {
        return ReplayTelemetry {
            segments: stats.segments,
            bytes: stats.bytes,
            oldest: stats.oldest,
            newest: stats.newest,
            truncated: stats.truncated,
        };
    }

    let dir = crate::engine::default_buffer_dir();
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default();
            if path.is_file()
                && (name.starts_with("seg_") || name.starts_with("segment_"))
                && matches!(ext, "mp4" | "ts")
            {
                if let Ok(meta) = entry.metadata() {
                    files.push((meta.len(), meta.modified().ok()));
                }
            }
        }
    }

    let bytes = files.iter().map(|(size, _)| *size).sum();
    let mut times: Vec<_> = files.iter().filter_map(|(_, time)| *time).collect();
    times.sort_unstable();
    ReplayTelemetry {
        segments: files.len(),
        bytes,
        oldest: times.first().copied(),
        newest: times.last().copied(),
        truncated: bytes >= u64::from(ctx.config.replay.max_memory_mb) * 1024 * 1024,
    }
}

fn target_details(config: &Config) -> (String, String, String, String, Option<String>) {
    let target = config.video.target.trim();
    let (window_title, crop_opt, rect_opt, hwnd_desc, dpi_desc) = target
        .strip_prefix("window:")
        .or_else(|| target.strip_prefix("title="))
        .map(|title| {
            let title = title.trim().to_string();
            #[cfg(target_os = "windows")]
            {
                if let Some(hwnd) = crate::discovery::find_window_hwnd(&title) {
                    let crop = crate::discovery::get_window_monitor_crop(hwnd);
                    let rect = crate::discovery::get_window_capture_rect_by_hwnd(hwnd);
                    let (h_str, d_str) = window_platform_details_by_hwnd(hwnd);
                    (title, crop, rect, h_str, d_str)
                } else {
                    (title, None, None, "no encontrado".to_string(), "no disponible".to_string())
                }
            }
            #[cfg(not(target_os = "windows"))]
            {
                let crop = crate::discovery::get_window_monitor_crop_by_title(&title);
                let rect = crate::discovery::get_window_capture_rect(&title);
                let (h_str, d_str) = window_platform_details(&title);
                (title, crop, rect, h_str, d_str)
            }
        })
        .unwrap_or_else(|| (String::new(), None, None, "no especificado".to_string(), "no especificado".to_string()));

    let effective_crop = crop_opt.as_ref().map(|c| c.crop_filter.clone());
    let crop_desc = if let Some(crop) = crop_opt {
        format!("-vf {} (DXGI DWM Hardware Crop @ {})", crop.crop_filter, crop.device_name)
    } else if let Some((x, y, w, h)) = rect_opt {
        format!(
            "offset_x={}, offset_y={}, video_size={}x{} (GDI Fallback)",
            x, y, w, h
        )
    } else if let Some(area) = target
        .strip_prefix("area:")
        .or_else(|| target.strip_prefix("geom:"))
    {
        format!("-vf crop={} (geometría solicitada)", area.trim())
    } else {
        "sin recorte (captura de escritorio completa)".to_string()
    };

    (
        if window_title.is_empty() {
            "no especificada".to_string()
        } else {
            window_title
        },
        rect_opt
            .map(|(x, y, w, h)| format!("x={}, y={}, w={}, h={}", x, y, w, h))
            .unwrap_or_else(|| "no disponible".to_string()),
        hwnd_desc,
        format!("{}; {}", dpi_desc, crop_desc),
        effective_crop,
    )
}

#[cfg(target_os = "windows")]
fn window_platform_details_by_hwnd(hwnd: windows_sys::Win32::Foundation::HWND) -> (String, String) {
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    (format!("{:?}", hwnd), format!("{} DPI", dpi))
}

#[cfg(target_os = "windows")]
fn window_platform_details(title: &str) -> (String, String) {
    if title.is_empty() {
        return ("no especificado".to_string(), "no especificado".to_string());
    }
    match crate::discovery::find_window_hwnd(title) {
        Some(hwnd) => window_platform_details_by_hwnd(hwnd),
        None => ("no encontrado".to_string(), "no disponible".to_string()),
    }
}

#[cfg(not(target_os = "windows"))]
fn window_platform_details(_title: &str) -> (String, String) {
    (
        "no aplica (Linux/Wayland)".to_string(),
        "no aplica".to_string(),
    )
}

fn format_system_time(time: Option<std::time::SystemTime>) -> String {
    time.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| format!("{} s desde epoch", d.as_secs()))
        .unwrap_or_else(|| "no disponible".to_string())
}

fn retention_seconds(telemetry: &ReplayTelemetry) -> Option<u64> {
    Some(
        telemetry
            .newest?
            .duration_since(telemetry.oldest?)
            .ok()?
            .as_secs(),
    )
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
    let output_exists = ctx.output_path.exists();
    let file_size_bytes = if output_exists {
        std::fs::metadata(ctx.output_path)
            .map(|m| m.len())
            .unwrap_or(0)
    } else {
        0
    };
    let file_size_mb = (file_size_bytes as f64) / (1024.0 * 1024.0);

    let (probe_res, is_aborted): (Result<FfprobeOutput, String>, bool) = if !output_exists {
        (
            Err("Archivo de video no presente en disco (posible eliminación previa por fallo de inicio)".to_string()),
            true,
        )
    } else if file_size_bytes < 1024 {
        (
            Err(format!("Inspección ffprobe omitida: archivo truncado o vacío ({} bytes < 1 KB)", file_size_bytes)),
            true,
        )
    } else {
        let res = probe_video_file(ctx.output_path);
        let failed = res.is_err();
        (res, failed)
    };
    let probe_err = probe_res.as_ref().err().cloned();
    let probe_data = probe_res.as_ref().ok();

    let video_stream = probe_data.and_then(|probe| {
        probe
            .streams
            .iter()
            .find(|stream| stream.codec_type == "video")
    });
    let audio_stream = probe_data.and_then(|probe| {
        probe
            .streams
            .iter()
            .find(|stream| stream.codec_type == "audio")
    });

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
        .and_then(|video| {
            video
                .avg_frame_rate
                .as_ref()
                .and_then(|fps_str| parse_fraction_fps(fps_str))
        })
        .or_else(|| {
            video_stream.and_then(|video| {
                video
                    .r_frame_rate
                    .as_ref()
                    .and_then(|fps_str| parse_fraction_fps(fps_str))
            })
        })
        .or(stderr_telem.last_fps)
        .unwrap_or(0.0);

    let fps_ratio = if target_fps > 0.0 {
        (real_fps / target_fps) * 100.0
    } else {
        100.0
    };

    let total_frames = video_stream
        .and_then(|video| {
            video
                .nb_frames
                .as_ref()
                .and_then(|frames_str| frames_str.parse::<u64>().ok())
        })
        .or(stderr_telem.total_frames)
        .unwrap_or_else(|| (real_fps * duration_secs).round() as u64);

    let dropped = stderr_telem.total_dropped.unwrap_or(0);
    let duplicated = stderr_telem.total_duplicated.unwrap_or(0);
    let replay = replay_telemetry(ctx);
    let (window_title, window_rect, window_hwnd, window_details, crop_filter_opt) = target_details(ctx.config);
    let scale_filter = if ctx.config.video.scale.is_empty() || ctx.config.video.scale == "none" {
        None
    } else {
        Some(format!("scale={}", ctx.config.video.scale))
    };
    let filter = match (crop_filter_opt, scale_filter) {
        (Some(c), Some(s)) => format!("{},{}", c, s),
        (Some(c), None) => c,
        (None, Some(s)) => s,
        (None, None) => "sin filtro".to_string(),
    };
    let pipeline = format!(
        "ffmpeg -f {} -r {} -c:v {} -preset {} -crf {} -vf {} -threads {}",
        ctx.capture_backend,
        target_fps,
        if ctx.config.video.codec.is_empty() {
            "auto"
        } else {
            &ctx.config.video.codec
        },
        if ctx.config.video.preset.is_empty() {
            "auto"
        } else {
            &ctx.config.video.preset
        },
        ctx.config.video.crf,
        filter,
        ctx.config
            .video
            .custom_threads
            .unwrap_or(ctx.config.video.threads),
    );

    let audio_expected = ctx.config.audio.capture_desktop || ctx.config.audio.capture_mic;
    let audio_ok = !audio_expected || audio_stream.is_some() || probe_err.is_some();

    // Determinación del veredicto del benchmark
    let (verdict_title, verdict_desc) = if is_aborted {
        (
            "🚨 CRÍTICO / GRABACIÓN ABORTADA (Fallo Prematuro)",
            "El codificador no consolidó fotogramas suficientes (<1KB o fallo en contenedor).",
        )
    } else if let Some(ref _err_msg) = probe_err {
        (
            "AVISO / INSPECCIÓN LIMITADA",
            "ffprobe no estuvo disponible para introspección binaria detallada; se conservan métricas extraídas del codificador.",
        )
    } else if duration_secs < 1.0 && video_stream.is_some() && audio_ok {
        (
            "BUENO / PRUEBA RÁPIDA (Clip Válido)",
            "El clip se grabó y finalizó en menos de 1 segundo de forma limpia con pistas de video y audio válidas.",
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
    let duplicate_ratio = if total_frames > 0 {
        duplicated as f64 / total_frames as f64
    } else {
        0.0
    };
    let requested_duration = if ctx.is_replay_clip {
        Some(ctx.config.replay.duration_secs as f64)
    } else {
        None
    };
    let short_clip = requested_duration
        .map(|requested| requested > 0.0 && duration_secs < requested * 0.95)
        .unwrap_or(false);

    let now_str = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let os_name = if cfg!(target_os = "windows") {
        "Windows (Win32 / DWM)"
    } else {
        "Linux (Wayland / PipeWire)"
    };

    let mut report = String::new();
    report.push_str(
        "================================================================================\n",
    );
    report.push_str("                    SANE-REC BENCHMARK & TELEMETRY REPORT\n");
    report.push_str(
        "================================================================================\n",
    );
    report.push_str(&format!("Fecha y Hora         : {}\n", now_str));
    report.push_str(&format!(
        "Versión sane-rec     : v{}\n",
        env!("CARGO_PKG_VERSION")
    ));
    report.push_str(&format!("Sistema Operativo    : {}\n", os_name));
    report.push_str(&format!(
        "GPU / Adaptador Gráfico: {}\n",
        crate::discovery::detect_gpu_info()
    ));
    #[cfg(target_os = "linux")]
    {
        if let Ok(vram) = std::fs::read_to_string("/sys/class/drm/card0/device/mem_info_vram_used") {
            if let Ok(bytes) = vram.trim().parse::<u64>() {
                report.push_str(&format!(
                    "VRAM en Uso          : {:.2} MB\n",
                    bytes as f64 / (1024.0 * 1024.0)
                ));
            }
        }
    }
    report.push_str(&format!(
        "Tipo de Grabación    : {}\n",
        if ctx.is_replay_clip {
            "Clip de Repetición Instantánea (Replay)"
        } else {
            "Grabación Continua Directa"
        }
    ));
    report.push_str(&format!(
        "Archivo de Video     : {}\n",
        ctx.output_path.display()
    ));
    report.push_str(&format!(
        "Tamaño de Archivo    : {:.2} MB ({} bytes)\n",
        file_size_mb, file_size_bytes
    ));
    report.push_str(&format!(
        "Duración del Clip    : {:.2} segundos\n\n",
        duration_secs
    ));

    report.push_str("--- CONFIGURACIÓN Y PIPELINE ---\n");
    report.push_str(&format!("Backend de Captura   : {}\n", ctx.capture_backend));
    report.push_str(&format!(
        "Perfil de Encoder    : {}\n",
        ctx.config.profile.active_profile
    ));
    report.push_str(&format!(
        "Códec Solicitado     : {}\n",
        if ctx.config.video.codec.is_empty() {
            "auto"
        } else {
            &ctx.config.video.codec
        }
    ));
    report.push_str(&format!("FPS Objetivo         : {:.2} FPS\n", target_fps));
    report.push_str(&format!(
        "Resolución/Escala    : {}\n",
        if ctx.config.video.scale.is_empty() {
            "Nativa (100%)"
        } else {
            &ctx.config.video.scale
        }
    ));
    report.push_str(&format!(
        "Audio Sistema        : {}\n",
        if ctx.config.audio.capture_desktop {
            "Activado"
        } else {
            "Desactivado"
        }
    ));
    report.push_str(&format!(
        "Audio Micrófono      : {}\n\n",
        if ctx.config.audio.capture_mic {
            "Activado"
        } else {
            "Desactivado"
        }
    ));

    report.push_str("--- LÍNEA DE COMANDOS Y PIPELINE EJECUTADO ---\n");
    report.push_str(&format!(
        "Parámetros efectivos : {}\n\n",
        ctx.executed_command.unwrap_or(&pipeline)
    ));

    if is_aborted {
        report.push_str("--- ESTADO DE EJECUCIÓN: GRABACIÓN FALLIDA / ABORTADA ---\n");
        report.push_str("Diagnóstico Inmediato: El proceso finalizó prematuramente antes de consolidar el stream de video.\n");
        if let Some(stderr) = ctx.encoder_stderr {
            let lines: Vec<&str> = stderr
                .lines()
                .filter(|line| !line.trim().is_empty())
                .collect();
            if !lines.is_empty() {
                report.push_str("Últimos registros del motor:\n");
                for line in lines.iter().rev().take(5).rev() {
                    report.push_str(&format!("  {}\n", line));
                }
            }
        }
        report.push('\n');
    }

    report.push_str("--- TELEMETRÍA DEL BÚFER EN MEMORIA RAM (REPLAY) ---\n");
    report.push_str(&format!("Microsegmentos en cola: {}\n", replay.segments));
    report.push_str(&format!(
        "RAM física contabilizada: {:.2} MB / {} MB máximos\n",
        replay.bytes as f64 / (1024.0 * 1024.0),
        ctx.config.replay.max_memory_mb
    ));
    report.push_str(&format!(
        "Segmento más antiguo : {}\n",
        format_system_time(replay.oldest)
    ));
    report.push_str(&format!(
        "Segmento más reciente: {}\n",
        format_system_time(replay.newest)
    ));
    report.push_str(&format!(
        "Retención temporal   : {} s observados / {} s solicitados\n",
        retention_seconds(&replay).unwrap_or(0),
        ctx.config.replay.duration_secs
    ));
    report.push_str(if replay.truncated {
        "ADVERTENCIA          : El búfer alcanzó max_memory_mb; pudo truncarse por límite de memoria.\n\n"
    } else {
        "Estado del límite    : No se observó saturación de max_memory_mb.\n\n"
    });

    report.push_str("--- TOPOLOGÍA Y RECORTE DE VENTANA (FOCUS) ---\n");
    report.push_str(&format!("Título de ventana    : {}\n", window_title));
    report.push_str(&format!("HWND                 : {}\n", window_hwnd));
    report.push_str(&format!("Coordenadas DWM      : {}\n", window_rect));
    report.push_str(&format!("DPI / filtro aplicado: {}\n\n", window_details));

    report.push_str("--- ANÁLISIS DE PISTAS (FFPROBE) ---\n");
    if let Some(video) = video_stream {
        report.push_str(&format!("Pista de Video       : DETECTADA [OK]\n"));
        report.push_str(&format!("  - Códec Real       : {}\n", video.codec_name));
        report.push_str(&format!(
            "  - Resolución       : {}x{}\n",
            video.width.unwrap_or(0),
            video.height.unwrap_or(0)
        ));
        report.push_str(&format!(
            "  - FPS Real (Probe) : {:.2} FPS ({:.1}% del objetivo)\n",
            real_fps, fps_ratio
        ));
        report.push_str(&format!("  - Cuadros Totales  : {} frames\n", total_frames));
    } else if let Some(ref err) = probe_err {
        report.push_str(&format!(
            "Pista de Video       : NO ANALIZADA (ffprobe no disponible: {})\n",
            err
        ));
    } else {
        report.push_str("Pista de Video       : NO DETECTADA [ERROR]\n");
    }

    if let Some(audio) = audio_stream {
        report.push_str(&format!("Pista de Audio       : DETECTADA [OK]\n"));
        report.push_str(&format!("  - Códec Audio      : {}\n", audio.codec_name));
        report.push_str(&format!(
            "  - Frecuencia (Hz)  : {} Hz\n",
            audio.sample_rate.as_deref().unwrap_or("48000")
        ));
        report.push_str(&format!(
            "  - Canales          : {}\n",
            audio.channels.unwrap_or(2)
        ));
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
        report.push_str(&format!(
            "FPS de Codificación  : {:.2} FPS (derivado de pista)\n",
            real_fps
        ));
    }
    report.push_str(&format!("Cuadros Descartados  : {} (drop)\n", dropped));
    report.push_str(&format!("Cuadros Duplicados   : {} (dup)\n", duplicated));
    if let Some(ref speed) = stderr_telem.speed_ratio {
        report.push_str(&format!("Multiplicador Speed  : {}\n", speed));
    }
    if ctx.is_replay_clip && duration_secs > 0.0 && file_size_bytes > 0 {
        let kbits = (file_size_bytes as f64 * 8.0) / (duration_secs * 1000.0);
        report.push_str(&format!("Tasa de Bits Final   : {:.2} kbits/s\n", kbits));
    } else if let Some(ref br) = stderr_telem.bitrate_kbps {
        report.push_str(&format!("Tasa de Bits Final   : {}\n", br));
    }

    report.push_str("\n--- DICTAMEN DE SALUD Y RENDIMIENTO ---\n");
    report.push_str(&format!("ESTADO GENERAL       : {}\n", verdict_title));
    report.push_str(&format!("Diagnóstico          : {}\n", verdict_desc));
    if duplicate_ratio > 0.10 && ctx.capture_backend.to_lowercase().contains("gdigrab") {
        report.push_str("ADVERTENCIA CAUSA RAÍZ: Más del 10% de cuadros duplicados; el backend GDI no puede capturar la swapchain de la GPU.\n");
    }
    if short_clip {
        report.push_str(&format!(
            "ADVERTENCIA CAUSA RAÍZ: El metraje ({:.2} s) es menor al solicitado ({:.2} s); el búfer no retuvo suficientes segmentos.\n",
            duration_secs,
            requested_duration.unwrap_or_default()
        ));
    }
    report.push_str(
        "================================================================================\n",
    );

    let benchmark_path = ctx.output_path.with_extension("benchmark.txt");
    let mut file = File::create(&benchmark_path).map_err(|e| {
        format!(
            "No se pudo crear el archivo de benchmark '{}': {}",
            benchmark_path.display(),
            e
        )
    })?;

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
            benchmark_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
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

    #[test]
    fn test_gdi_duplicate_warning_only_for_gdigrab() {
        let temp_dir = std::env::temp_dir().join(format!("sane_rec_bench_gdi_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let dummy_video = temp_dir.join("clip.mp4");
        let _ = std::fs::write(&dummy_video, vec![0u8; 2048]);

        let config = Config::default();
        let stderr = "frame=100 fps=60.0 dup=25 drop=0 speed=1.00x";

        let gdi_ctx = BenchmarkContext {
            output_path: &dummy_video,
            config: &config,
            encoder_stderr: Some(stderr),
            executed_command: None,
            capture_backend: "GDI Screen Grabber (gdigrab)",
            is_replay_clip: false,
            elapsed_duration: Some(Duration::from_secs(2)),
            ram_buffer_stats: None,
        };
        let gdi_report = generate_benchmark_report(&gdi_ctx).unwrap();
        let gdi_content = std::fs::read_to_string(&gdi_report).unwrap();
        assert!(gdi_content.contains("el backend GDI no puede capturar"));

        let dda_ctx = BenchmarkContext {
            output_path: &dummy_video,
            config: &config,
            encoder_stderr: Some(stderr),
            executed_command: None,
            capture_backend: "Desktop Duplication API (ddagrab / DXGI)",
            is_replay_clip: false,
            elapsed_duration: Some(Duration::from_secs(2)),
            ram_buffer_stats: None,
        };
        let dda_report = generate_benchmark_report(&dda_ctx).unwrap();
        let dda_content = std::fs::read_to_string(&dda_report).unwrap();
        assert!(!dda_content.contains("el backend GDI no puede capturar"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
