use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Representa la información de una salida de pantalla / monitor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutputInfo {
    pub name: String,
    pub description: String,
    pub width: u32,
    pub height: u32,
    pub refresh_rate: f32,
    pub is_current: bool,
}

/// Representa la información de una ventana abierta en el compositor Wayland.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: u64,
    pub title: String,
    pub app_id: String,
    pub pid: u32,
    pub is_focused: bool,
}

/// Representa un dispositivo de audio detectado por PipeWire / PulseAudio.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioDeviceInfo {
    pub name: String,
    pub description: String,
    pub is_default: bool,
}

/// Representa un clip o replay grabado previamente.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipInfo {
    pub name: String,
    pub path: PathBuf,
    pub size_mb: f64,
    pub modified: Option<SystemTime>,
}

/// Obtiene las salidas de pantalla conectadas.
/// Ejecuta `niri msg -j outputs` y parsea el JSON.
/// Si falla o no se está ejecutando Niri, hace fallback a `wf-recorder -L`.
pub fn get_connected_outputs() -> Vec<OutputInfo> {
    // 1. Intentar con niri msg -j outputs
    let niri_res = Command::new("niri")
        .args(["msg", "-j", "outputs"])
        .output();

    if let Ok(out) = niri_res {
        if out.status.success() {
            let json_str = String::from_utf8_lossy(&out.stdout);
            // Intentar detectar la pantalla enfocada actualmente
            let focused_name = Command::new("niri")
                .args(["msg", "-j", "focused-output"])
                .output()
                .ok()
                .and_then(|f_out| {
                    if f_out.status.success() {
                        let f_json: Value = serde_json::from_slice(&f_out.stdout).ok()?;
                        f_json.get("name").and_then(|n| n.as_str()).map(|s| s.to_string())
                    } else {
                        None
                    }
                });

            let parsed = parse_niri_outputs_with_focus(&json_str, focused_name.as_deref());
            if !parsed.is_empty() {
                return parsed;
            }
        }
    }

    // 2. Fallback a wf-recorder -L
    let wf_res = Command::new("wf-recorder")
        .arg("-L")
        .output();

    if let Ok(out) = wf_res {
        if out.status.success() {
            let output_str = String::from_utf8_lossy(&out.stdout);
            let parsed = parse_wf_recorder_outputs(&output_str);
            if !parsed.is_empty() {
                return parsed;
            }
        }
    }

    Vec::new()
}

/// Parsea el JSON devuelto por `niri msg -j outputs`.
pub fn parse_niri_outputs(json_str: &str) -> Vec<OutputInfo> {
    parse_niri_outputs_with_focus(json_str, None)
}

/// Parsea el JSON de salidas de Niri indicando opcionalmente el nombre del monitor enfocado.
pub fn parse_niri_outputs_with_focus(json_str: &str, focused_name: Option<&str>) -> Vec<OutputInfo> {
    let Ok(root) = serde_json::from_str::<Value>(json_str) else {
        return Vec::new();
    };

    let mut outputs = Vec::new();

    let items: Vec<(String, &Value)> = match &root {
        Value::Object(map) => map.iter().map(|(k, v)| (k.clone(), v)).collect(),
        Value::Array(list) => list
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let name = v.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                if name.is_empty() {
                    (format!("output-{}", i), v)
                } else {
                    (name, v)
                }
            })
            .collect(),
        _ => return Vec::new(),
    };

    for (key, val) in items {
        let name = val
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or(&key)
            .to_string();

        let make = val.get("make").and_then(|m| m.as_str()).unwrap_or("").trim();
        let model = val.get("model").and_then(|m| m.as_str()).unwrap_or("").trim();

        let mut width = 0u32;
        let mut height = 0u32;
        let mut refresh_rate = 0.0f32;

        let modes = val.get("modes").and_then(|m| m.as_array());
        let current_mode_idx = val.get("current_mode").and_then(|c| c.as_u64());

        if let (Some(idx), Some(modes_list)) = (current_mode_idx, modes) {
            if let Some(mode) = modes_list.get(idx as usize) {
                width = mode.get("width").and_then(|w| w.as_u64()).unwrap_or(0) as u32;
                height = mode.get("height").and_then(|h| h.as_u64()).unwrap_or(0) as u32;
                let raw_rr = mode.get("refresh_rate").and_then(|r| r.as_f64()).unwrap_or(0.0) as f32;
                refresh_rate = if raw_rr > 1000.0 { raw_rr / 1000.0 } else { raw_rr };
            }
        }

        // Fallback a resolución lógica si modes no estaba disponible
        if width == 0 || height == 0 {
            if let Some(logical) = val.get("logical") {
                width = logical.get("width").and_then(|w| w.as_u64()).unwrap_or(0) as u32;
                height = logical.get("height").and_then(|h| h.as_u64()).unwrap_or(0) as u32;
                if refresh_rate == 0.0 {
                    refresh_rate = 60.0;
                }
            }
        }

        // Fallback si los modos tienen propiedades directas
        if width == 0 {
            width = val.get("width").and_then(|w| w.as_u64()).unwrap_or(0) as u32;
        }
        if height == 0 {
            height = val.get("height").and_then(|h| h.as_u64()).unwrap_or(0) as u32;
        }
        if refresh_rate == 0.0 {
            let raw_rr = val.get("refresh_rate").and_then(|r| r.as_f64()).unwrap_or(0.0) as f32;
            refresh_rate = if raw_rr > 1000.0 { raw_rr / 1000.0 } else { raw_rr };
        }

        let display_name = if !make.is_empty() {
            make
        } else if !model.is_empty() {
            model
        } else {
            &name
        };

        let description = if let Some(custom_desc) = val.get("description").and_then(|d| d.as_str()) {
            custom_desc.to_string()
        } else if width > 0 && height > 0 {
            format!("{} ({}x{} @ {:.0}Hz)", display_name, width, height, refresh_rate)
        } else {
            display_name.to_string()
        };

        let is_current = if let Some(explicit) = val
            .get("is_current")
            .or_else(|| val.get("is_focused"))
            .and_then(|v| v.as_bool())
        {
            explicit
        } else if let Some(fn_name) = focused_name {
            fn_name == name
        } else {
            false
        };

        outputs.push(OutputInfo {
            name,
            description,
            width,
            height,
            refresh_rate,
            is_current,
        });
    }

    outputs.sort_by(|a, b| a.name.cmp(&b.name));
    outputs
}

/// Parsea la salida de `wf-recorder -L`.
pub fn parse_wf_recorder_outputs(output_str: &str) -> Vec<OutputInfo> {
    let mut outputs = Vec::new();
    for (idx, line) in output_str.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(name_idx) = line.find("Name: ") {
            let after_name = &line[name_idx + 6..];
            let (name, desc) = if let Some(desc_idx) = after_name.find("Description: ") {
                (
                    after_name[..desc_idx].trim().to_string(),
                    after_name[desc_idx + 13..].trim().to_string(),
                )
            } else {
                (after_name.trim().to_string(), after_name.trim().to_string())
            };

            if !name.is_empty() {
                outputs.push(OutputInfo {
                    name,
                    description: desc,
                    width: 0,
                    height: 0,
                    refresh_rate: 0.0,
                    is_current: idx == 0,
                });
            }
        }
    }
    outputs
}

/// Obtiene las ventanas abiertas en el compositor Niri.
/// Ejecuta `niri msg -j windows` y parsea el array resultante.
pub fn get_open_windows() -> Vec<WindowInfo> {
    let output = Command::new("niri")
        .args(["msg", "-j", "windows"])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let json_str = String::from_utf8_lossy(&out.stdout);
            parse_niri_windows(&json_str)
        }
        _ => Vec::new(),
    }
}

/// Parsea el JSON devuelto por `niri msg -j windows`.
pub fn parse_niri_windows(json_str: &str) -> Vec<WindowInfo> {
    let Ok(val) = serde_json::from_str::<Value>(json_str) else {
        return Vec::new();
    };

    let Some(arr) = val.as_array() else {
        return Vec::new();
    };

    arr.iter()
        .filter_map(|w| {
            if !w.is_object() {
                return None;
            }
            let id = w.get("id").and_then(|i| i.as_u64()).unwrap_or(0);
            let title = w
                .get("title")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            let app_id = w
                .get("app_id")
                .and_then(|a| a.as_str())
                .unwrap_or("")
                .to_string();
            let pid = w
                .get("pid")
                .and_then(|p| p.as_u64())
                .unwrap_or(0) as u32;
            let is_focused = w
                .get("is_focused")
                .and_then(|f| f.as_bool())
                .unwrap_or(false);

            Some(WindowInfo {
                id,
                title,
                app_id,
                pid,
                is_focused,
            })
        })
        .collect()
}

/// Obtiene los dispositivos de audio disponibles del sistema a través de PulseAudio / PipeWire.
/// Retorna `(monitors, microphones)`:
/// - `monitors`: dispositivos de salida / escritorio (terminan en `.monitor` o con clase `Audio/Sink`).
/// - `microphones`: entradas físicas o virtuales (con clase `Audio/Source`).
pub fn get_audio_devices() -> (Vec<AudioDeviceInfo>, Vec<AudioDeviceInfo>) {
    let default_sink = Command::new("pactl")
        .arg("get-default-sink")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok().map(|s| s.trim().to_string())
            } else {
                None
            }
        });

    let default_source = Command::new("pactl")
        .arg("get-default-source")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stdout).ok().map(|s| s.trim().to_string())
            } else {
                None
            }
        });

    let output = Command::new("pactl")
        .args(["-f", "json", "list", "sources"])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let json_str = String::from_utf8_lossy(&out.stdout);
            parse_pactl_sources(&json_str, default_sink.as_deref(), default_source.as_deref())
        }
        _ => (Vec::new(), Vec::new()),
    }
}

/// Parsea el JSON devuelto por `pactl -f json list sources`.
pub fn parse_pactl_sources(
    json_str: &str,
    default_sink: Option<&str>,
    default_source: Option<&str>,
) -> (Vec<AudioDeviceInfo>, Vec<AudioDeviceInfo>) {
    let Ok(val) = serde_json::from_str::<Value>(json_str) else {
        return (Vec::new(), Vec::new());
    };

    let Some(arr) = val.as_array() else {
        return (Vec::new(), Vec::new());
    };

    let mut monitors = Vec::new();
    let mut microphones = Vec::new();

    for item in arr {
        let name = item
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("")
            .to_string();
        if name.is_empty() {
            continue;
        }

        let description = item
            .get("description")
            .and_then(|d| d.as_str())
            .or_else(|| {
                item.get("properties")
                    .and_then(|p| p.get("device.description"))
                    .and_then(|d| d.as_str())
            })
            .unwrap_or(&name)
            .to_string();

        let props = item.get("properties");
        let media_class = props
            .and_then(|p| p.get("media.class"))
            .and_then(|m| m.as_str());

        let is_monitor = name.ends_with(".monitor")
            || media_class == Some("Audio/Sink")
            || media_class.map_or(false, |mc| mc.contains("Sink"));

        let explicit_default = item.get("is_default").and_then(|v| v.as_bool());

        if is_monitor {
            let is_default = explicit_default.unwrap_or_else(|| {
                default_sink.map_or(false, |ds| {
                    !ds.is_empty()
                        && (name == ds
                            || name == format!("{}.monitor", ds)
                            || name.starts_with(ds)
                            || ds.starts_with(&name.trim_end_matches(".monitor")))
                })
            });

            monitors.push(AudioDeviceInfo {
                name,
                description,
                is_default,
            });
        } else {
            let is_default = explicit_default.unwrap_or_else(|| {
                default_source.map_or(false, |ds| {
                    !ds.is_empty() && (name == ds || name.starts_with(ds) || ds.starts_with(&name))
                })
            });

            microphones.push(AudioDeviceInfo {
                name,
                description,
                is_default,
            });
        }
    }

    (monitors, microphones)
}

/// Lanza `slurp` interactivamente para seleccionar una región de pantalla.
/// Si el usuario selecciona una región, retorna la cadena con formato "x,y WxH".
/// Si cancela (por ejemplo pulsando Esc) o hay error, retorna `None`.
pub fn run_interactive_slurp() -> Option<String> {
    let output = Command::new("slurp")
        .output()
        .ok()?;

    if output.status.success() {
        let result = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !result.is_empty() {
            Some(result)
        } else {
            None
        }
    } else {
        None
    }
}

/// Escanea el directorio especificado (ej. `/mnt/Files/Videos/Replays/`) y lista
/// los archivos `.mp4` y `.mkv`, extrayendo tamaño en MB, fecha de modificación y nombre.
/// Los clips se retornan ordenados por fecha de modificación descendente (los más recientes primero).
pub fn get_recent_clips(dir: &Path) -> Vec<ClipInfo> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut clips = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let Some(ext) = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
        else {
            continue;
        };

        if ext == "mp4" || ext == "mkv" {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            let metadata = entry.metadata().ok();
            let size_bytes = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
            let size_mb = ((size_bytes as f64) / (1024.0 * 1024.0) * 100.0).round() / 100.0;
            let modified = metadata.and_then(|m| m.modified().ok());

            clips.push(ClipInfo {
                name,
                path,
                size_mb,
                modified,
            });
        }
    }

    clips.sort_by(|a, b| b.modified.cmp(&a.modified));
    clips
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn test_parse_niri_outputs_simulated() {
        let niri_json = r#"{
            "DVI-D-1": {
                "name": "DVI-D-1",
                "make": "Envision Peripherals, Inc",
                "model": "EPI2443",
                "modes": [
                    { "width": 1280, "height": 720, "refresh_rate": 60000, "is_preferred": true },
                    { "width": 1360, "height": 768, "refresh_rate": 60015, "is_preferred": false }
                ],
                "current_mode": 1,
                "logical": { "x": 0, "y": 0, "width": 1360, "height": 768 }
            },
            "HDMI-A-1": {
                "name": "HDMI-A-1",
                "make": "PNP(SAC)",
                "model": "LED MONITOR",
                "modes": [
                    { "width": 1920, "height": 1080, "refresh_rate": 59940, "is_preferred": true }
                ],
                "current_mode": 0,
                "logical": { "x": 1360, "y": 0, "width": 1920, "height": 1080 }
            }
        }"#;

        let outputs = parse_niri_outputs_with_focus(niri_json, Some("DVI-D-1"));
        assert_eq!(outputs.len(), 2);

        let dvi = outputs.iter().find(|o| o.name == "DVI-D-1").unwrap();
        assert_eq!(dvi.name, "DVI-D-1");
        assert_eq!(dvi.width, 1360);
        assert_eq!(dvi.height, 768);
        assert!((dvi.refresh_rate - 60.015).abs() < 0.01);
        assert_eq!(dvi.description, "Envision Peripherals, Inc (1360x768 @ 60Hz)");
        assert!(dvi.is_current);

        let hdmi = outputs.iter().find(|o| o.name == "HDMI-A-1").unwrap();
        assert_eq!(hdmi.name, "HDMI-A-1");
        assert_eq!(hdmi.width, 1920);
        assert_eq!(hdmi.height, 1080);
        assert!((hdmi.refresh_rate - 59.94).abs() < 0.01);
        assert_eq!(hdmi.description, "PNP(SAC) (1920x1080 @ 60Hz)");
        assert!(!hdmi.is_current);
    }

    #[test]
    fn test_parse_wf_recorder_outputs() {
        let sample_output = "1. Name: DVI-D-1 Description: Envision Peripherals, Inc - EPI2443 - DVI-D-1\n\
                             2. Name: HDMI-A-1 Description: PNP(SAC) - LED MONITOR - HDMI-A-1\n";

        let outputs = parse_wf_recorder_outputs(sample_output);
        assert_eq!(outputs.len(), 2);

        assert_eq!(outputs[0].name, "DVI-D-1");
        assert_eq!(outputs[0].description, "Envision Peripherals, Inc - EPI2443 - DVI-D-1");
        assert!(outputs[0].is_current);

        assert_eq!(outputs[1].name, "HDMI-A-1");
        assert_eq!(outputs[1].description, "PNP(SAC) - LED MONITOR - HDMI-A-1");
        assert!(!outputs[1].is_current);
    }

    #[test]
    fn test_parse_niri_windows_simulated() {
        let windows_json = r#"[
            {
                "id": 5,
                "title": "Pre Prueba - Google Gemini — Zen Browser",
                "app_id": "zen",
                "pid": 15792,
                "is_focused": false
            },
            {
                "id": 7,
                "title": "sane-rec",
                "app_id": "antigravity",
                "pid": 89518,
                "is_focused": true
            }
        ]"#;

        let windows = parse_niri_windows(windows_json);
        assert_eq!(windows.len(), 2);

        assert_eq!(windows[0].id, 5);
        assert_eq!(windows[0].title, "Pre Prueba - Google Gemini — Zen Browser");
        assert_eq!(windows[0].app_id, "zen");
        assert_eq!(windows[0].pid, 15792);
        assert!(!windows[0].is_focused);

        assert_eq!(windows[1].id, 7);
        assert_eq!(windows[1].title, "sane-rec");
        assert_eq!(windows[1].app_id, "antigravity");
        assert_eq!(windows[1].pid, 89518);
        assert!(windows[1].is_focused);
    }

    #[test]
    fn test_parse_pactl_sources_simulated() {
        let sources_json = r#"[
            {
                "name": "alsa_output.pci-0000_01_00.1.hdmi-stereo.monitor",
                "description": "Monitor of Ellesmere HDMI Audio",
                "properties": {
                    "media.class": "Audio/Sink"
                }
            },
            {
                "name": "alsa_output.pci-0000_08_00.6.analog-stereo.monitor",
                "description": "Monitor de Audio Analógico",
                "properties": {
                    "media.class": "Audio/Sink"
                }
            },
            {
                "name": "alsa_input.usb-Webcam.mono-fallback",
                "description": "Webcam Mono",
                "properties": {
                    "media.class": "Audio/Source"
                }
            },
            {
                "name": "alsa_input.usb-PnP_Audio.mono-fallback",
                "description": "Micrófono USB PnP",
                "properties": {
                    "media.class": "Audio/Source"
                }
            },
            {
                "name": "easyeffects_source",
                "description": "Easy Effects Source",
                "properties": {
                    "media.class": "Audio/Source/Virtual"
                }
            }
        ]"#;

        let default_sink = "alsa_output.pci-0000_08_00.6.analog-stereo";
        let default_source = "alsa_input.usb-PnP_Audio.mono-fallback";

        let (monitors, microphones) = parse_pactl_sources(sources_json, Some(default_sink), Some(default_source));

        assert_eq!(monitors.len(), 2);
        let m0 = &monitors[0];
        assert_eq!(m0.name, "alsa_output.pci-0000_01_00.1.hdmi-stereo.monitor");
        assert!(!m0.is_default);

        let m1 = &monitors[1];
        assert_eq!(m1.name, "alsa_output.pci-0000_08_00.6.analog-stereo.monitor");
        assert!(m1.is_default);

        assert_eq!(microphones.len(), 3);
        let mic0 = &microphones[0];
        assert_eq!(mic0.name, "alsa_input.usb-Webcam.mono-fallback");
        assert!(!mic0.is_default);

        let mic1 = &microphones[1];
        assert_eq!(mic1.name, "alsa_input.usb-PnP_Audio.mono-fallback");
        assert!(mic1.is_default);

        let mic2 = &microphones[2];
        assert_eq!(mic2.name, "easyeffects_source");
        assert!(!mic2.is_default);
    }

    #[test]
    fn test_get_recent_clips_simulated() {
        let temp_dir = std::env::temp_dir().join("sane_rec_discovery_test");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        let clip1_path = temp_dir.join("Recording_2026-10-29_18-10-26.mp4");
        let clip2_path = temp_dir.join("Recording_2026-10-29_18-59-09.mkv");
        let txt_path = temp_dir.join("notes.txt");

        {
            let mut f1 = File::create(&clip1_path).unwrap();
            f1.write_all(&vec![0u8; 1024 * 1024]).unwrap(); // 1 MB
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        {
            let mut f2 = File::create(&clip2_path).unwrap();
            f2.write_all(&vec![0u8; 2 * 1024 * 1024]).unwrap(); // 2 MB
        }
        {
            let mut f3 = File::create(&txt_path).unwrap();
            f3.write_all(b"not a clip").unwrap();
        }

        let clips = get_recent_clips(&temp_dir);
        assert_eq!(clips.len(), 2);

        // Debería estar ordenado con el más reciente primero (clip2)
        assert_eq!(clips[0].name, "Recording_2026-10-29_18-59-09.mkv");
        assert_eq!(clips[0].size_mb, 2.0);
        assert!(clips[0].modified.is_some());

        assert_eq!(clips[1].name, "Recording_2026-10-29_18-10-26.mp4");
        assert_eq!(clips[1].size_mb, 1.0);
        assert!(clips[1].modified.is_some());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_serialization_roundtrip() {
        let out = OutputInfo {
            name: "DVI-D-1".to_string(),
            description: "Monitor (1920x1080 @ 60Hz)".to_string(),
            width: 1920,
            height: 1080,
            refresh_rate: 60.0,
            is_current: true,
        };
        let s = serde_json::to_string(&out).unwrap();
        let de: OutputInfo = serde_json::from_str(&s).unwrap();
        assert_eq!(out, de);

        let win = WindowInfo {
            id: 42,
            title: "Test".to_string(),
            app_id: "test.app".to_string(),
            pid: 1234,
            is_focused: true,
        };
        let s = serde_json::to_string(&win).unwrap();
        let de: WindowInfo = serde_json::from_str(&s).unwrap();
        assert_eq!(win, de);

        let audio = AudioDeviceInfo {
            name: "test.monitor".to_string(),
            description: "Test Audio".to_string(),
            is_default: false,
        };
        let s = serde_json::to_string(&audio).unwrap();
        let de: AudioDeviceInfo = serde_json::from_str(&s).unwrap();
        assert_eq!(audio, de);

        let clip = ClipInfo {
            name: "video.mp4".to_string(),
            path: PathBuf::from("/path/to/video.mp4"),
            size_mb: 5.5,
            modified: Some(SystemTime::now()),
        };
        let s = serde_json::to_string(&clip).unwrap();
        let de: ClipInfo = serde_json::from_str(&s).unwrap();
        assert_eq!(clip.name, de.name);
        assert_eq!(clip.path, de.path);
        assert_eq!(clip.size_mb, de.size_mb);
    }
}
