use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EncoderProfile {
    SaneSafe,
    AmdHardware,
    NvidiaHardware,
    IntelHardware,
    Custom(String),
}

impl EncoderProfile {
    /// Devuelve los elementos del encoder H.264 para este perfil.
    pub fn encoder_elements(&self) -> &str {
        match self {
            // CPU pura con x264enc ajustado para latencia cero y consumo predecible
            // Diseñado especialmente para evitar colgar el anillo vce0 de GPUs Polaris / AMD problemáticas
            EncoderProfile::SaneSafe => {
                "x264enc speed-preset=ultrafast tune=zerolatency key-int-max=60 bframes=0"
            }
            // Hardware VA-API para AMD
            EncoderProfile::AmdHardware => "vaapih264enc",
            // Hardware NVENC para Nvidia
            EncoderProfile::NvidiaHardware => "nvh264enc",
            // Hardware QuickSync para Intel
            EncoderProfile::IntelHardware => "qsvh264enc",
            // Pipeline manual o encoder personalizado
            EncoderProfile::Custom(ref custom) => custom.as_str(),
        }
    }

    /// Devuelve los elementos del encoder H.264 permitiendo limitar el número de hilos (útil para perfiles térmicos/laptops).
    pub fn encoder_elements_with_threads(&self, threads: Option<u32>) -> String {
        match self {
            EncoderProfile::SaneSafe => {
                if let Some(t) = threads {
                    format!(
                        "x264enc speed-preset=ultrafast tune=zerolatency key-int-max=60 bframes=0 threads={}",
                        t
                    )
                } else {
                    self.encoder_elements().to_string()
                }
            }
            EncoderProfile::Custom(ref custom) => {
                if let Some(t) = threads {
                    if custom.contains("threads=") {
                        custom.clone()
                    } else if custom.contains("x264enc") {
                        format!("{} threads={}", custom, t)
                    } else {
                        custom.clone()
                    }
                } else {
                    custom.clone()
                }
            }
            _ => self.encoder_elements().to_string(),
        }
    }
}

impl std::str::FromStr for EncoderProfile {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normalized = s.trim().to_lowercase();
        Ok(match normalized.as_str() {
            "sane-safe" | "sanesafe" | "cpu" | "x264" => EncoderProfile::SaneSafe,
            "amd-hardware" | "amdhardware" | "amd" | "vaapi" => EncoderProfile::AmdHardware,
            "nvidia-hardware" | "nvidiahardware" | "nvidia" | "nvenc" => {
                EncoderProfile::NvidiaHardware
            }
            "intel-hardware" | "intelhardware" | "intel" | "qsv" => EncoderProfile::IntelHardware,
            _ => EncoderProfile::Custom(s.to_string()),
        })
    }
}

impl std::fmt::Display for EncoderProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EncoderProfile::SaneSafe => write!(f, "sane-safe"),
            EncoderProfile::AmdHardware => write!(f, "amd-hardware"),
            EncoderProfile::NvidiaHardware => write!(f, "nvidia-hardware"),
            EncoderProfile::IntelHardware => write!(f, "intel-hardware"),
            EncoderProfile::Custom(custom) => write!(f, "custom:{}", custom),
        }
    }
}

/// Construye el pipeline GStreamer para captura de video usando PipeWire y el perfil de codificación seleccionado.
pub fn build_video_pipeline(
    profile: &EncoderProfile,
    target_node: Option<&str>,
    fps: u32,
) -> String {
    build_video_pipeline_with_threads(profile, target_node, fps, None)
}

/// Construye el pipeline GStreamer para captura de video permitiendo limitar el número de hilos de codificación.
pub fn build_video_pipeline_with_threads(
    profile: &EncoderProfile,
    target_node: Option<&str>,
    fps: u32,
    threads: Option<u32>,
) -> String {
    let src = match target_node {
        Some(node) if !node.trim().is_empty() && node != "portal" => {
            format!("pipewiresrc target-object={} do-timestamp=true", node.trim())
        }
        _ => "pipewiresrc do-timestamp=true".to_string(),
    };

    let encoder_str = profile.encoder_elements_with_threads(threads);
    let parse_str = if encoder_str.contains("h264parse") {
        ""
    } else {
        " ! h264parse"
    };

    format!(
        "{} ! video/x-raw,framerate={}/1 ! videoconvert ! {}{}",
        src, fps, encoder_str, parse_str
    )
}

/// Construye el pipeline GStreamer para la captura de audio (escritorio, micrófono o ambos mezclados).
pub fn build_audio_pipeline(capture_desktop: bool, capture_mic: bool, codec: &str) -> String {
    if !capture_desktop && !capture_mic {
        return String::new();
    }

    let (enc, parse) = if codec.eq_ignore_ascii_case("opus") {
        ("opusenc", "opusparse")
    } else {
        ("avenc_aac", "aacparse")
    };

    if capture_desktop && capture_mic {
        format!(
            "audiomixer name=mix ! audioconvert ! audioresample ! {enc} ! {parse} \
            pulsesrc device=\"@DEFAULT_MONITOR@\" ! audioconvert ! audioresample ! mix. \
            pulsesrc device=\"@DEFAULT_SOURCE@\" ! audioconvert ! audioresample ! mix."
        )
    } else if capture_desktop {
        format!(
            "pulsesrc device=\"@DEFAULT_MONITOR@\" ! audioconvert ! audioresample ! {enc} ! {parse}"
        )
    } else {
        format!(
            "pulsesrc device=\"@DEFAULT_SOURCE@\" ! audioconvert ! audioresample ! {enc} ! {parse}"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encoder_profile_from_str() {
        assert_eq!("sane-safe".parse::<EncoderProfile>().unwrap(), EncoderProfile::SaneSafe);
        assert_eq!("amd-hardware".parse::<EncoderProfile>().unwrap(), EncoderProfile::AmdHardware);
        assert_eq!("nvidia-hardware".parse::<EncoderProfile>().unwrap(), EncoderProfile::NvidiaHardware);
        assert_eq!("intel-hardware".parse::<EncoderProfile>().unwrap(), EncoderProfile::IntelHardware);
        assert_eq!(
            "x264enc bitrate=3000".parse::<EncoderProfile>().unwrap(),
            EncoderProfile::Custom("x264enc bitrate=3000".to_string())
        );
    }

    #[test]
    fn test_build_video_pipeline_sane_safe() {
        let pipeline = build_video_pipeline(&EncoderProfile::SaneSafe, Some("DVI-D-1"), 60);

        assert!(pipeline.contains("pipewiresrc target-object=DVI-D-1 do-timestamp=true"));
        assert!(pipeline.contains("video/x-raw,framerate=60/1"));
        assert!(pipeline.contains("videoconvert"));
        assert!(pipeline.contains("x264enc speed-preset=ultrafast tune=zerolatency key-int-max=60 bframes=0"));
        assert!(pipeline.contains("h264parse"));
    }

    #[test]
    fn test_build_video_pipeline_with_threads() {
        let pipeline_2t = build_video_pipeline_with_threads(&EncoderProfile::SaneSafe, None, 30, Some(2));
        assert!(pipeline_2t.contains("threads=2"));
        assert!(pipeline_2t.contains("framerate=30/1"));

        let pipeline_custom = build_video_pipeline_with_threads(
            &EncoderProfile::Custom("x264enc preset=ultrafast".to_string()),
            None,
            30,
            Some(2),
        );
        assert!(pipeline_custom.contains("threads=2"));
    }

    #[test]
    fn test_build_video_pipeline_hardware_profiles() {
        let p_amd = build_video_pipeline(&EncoderProfile::AmdHardware, None, 30);
        assert!(p_amd.contains("pipewiresrc do-timestamp=true"));
        assert!(p_amd.contains("vaapih264enc"));
        assert!(p_amd.contains("framerate=30/1"));

        let p_nv = build_video_pipeline(&EncoderProfile::NvidiaHardware, None, 60);
        assert!(p_nv.contains("nvh264enc"));

        let p_intel = build_video_pipeline(&EncoderProfile::IntelHardware, None, 60);
        assert!(p_intel.contains("qsvh264enc"));
    }

    #[test]
    fn test_build_audio_pipeline() {
        // Ninguno
        assert_eq!(build_audio_pipeline(false, false, "aac"), "");

        // Solo escritorio AAC
        let audio_desktop_aac = build_audio_pipeline(true, false, "aac");
        assert!(audio_desktop_aac.contains("pulsesrc device=\"@DEFAULT_MONITOR@\""));
        assert!(audio_desktop_aac.contains("avenc_aac"));
        assert!(audio_desktop_aac.contains("aacparse"));
        assert!(!audio_desktop_aac.contains("DEFAULT_SOURCE"));

        // Solo micro Opus
        let audio_mic_opus = build_audio_pipeline(false, true, "opus");
        assert!(audio_mic_opus.contains("pulsesrc device=\"@DEFAULT_SOURCE@\""));
        assert!(audio_mic_opus.contains("opusenc"));
        assert!(audio_mic_opus.contains("opusparse"));
        assert!(!audio_mic_opus.contains("DEFAULT_MONITOR"));

        // Ambos mezclados
        let audio_both = build_audio_pipeline(true, true, "opus");
        assert!(audio_both.contains("audiomixer name=mix"));
        assert!(audio_both.contains("DEFAULT_MONITOR"));
        assert!(audio_both.contains("DEFAULT_SOURCE"));
        assert!(audio_both.contains("opusenc"));
    }
}
