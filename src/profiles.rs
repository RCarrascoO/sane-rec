use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EncoderProfile {
    SaneSafe,
    #[serde(alias = "amd-vaapi", alias = "amdhardware", alias = "amd", alias = "vaapi", alias = "h264_vaapi")]
    AmdHardware,
    #[serde(alias = "nvidia-nvenc", alias = "nvidiahardware", alias = "nvidia", alias = "nvenc", alias = "h264_nvenc")]
    NvidiaHardware,
    #[serde(alias = "intel-qsv", alias = "intelhardware", alias = "qsv", alias = "h264_qsv")]
    IntelHardware,
    Custom(String),
}

impl std::str::FromStr for EncoderProfile {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normalized = s.trim().to_lowercase();
        Ok(match normalized.as_str() {
            "sane-safe" | "sanesafe" | "cpu" | "x264" => EncoderProfile::SaneSafe,
            "amd-hardware" | "amdhardware" | "amd" | "vaapi" | "amd-vaapi" | "h264_vaapi" => {
                EncoderProfile::AmdHardware
            }
            "nvidia-hardware" | "nvidiahardware" | "nvidia" | "nvenc" | "nvidia-nvenc" | "h264_nvenc" => {
                EncoderProfile::NvidiaHardware
            }
            "intel-qsv" | "intel-hardware" | "intelhardware" | "intel" | "qsv" | "h264_qsv" => {
                EncoderProfile::IntelHardware
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encoder_profile_from_str() {
        assert_eq!("sane-safe".parse::<EncoderProfile>().unwrap(), EncoderProfile::SaneSafe);
        assert_eq!("amd-hardware".parse::<EncoderProfile>().unwrap(), EncoderProfile::AmdHardware);
        assert_eq!("nvidia-hardware".parse::<EncoderProfile>().unwrap(), EncoderProfile::NvidiaHardware);
        assert_eq!("intel-hardware".parse::<EncoderProfile>().unwrap(), EncoderProfile::IntelHardware);
        assert_eq!("intel-qsv".parse::<EncoderProfile>().unwrap(), EncoderProfile::IntelHardware);
        assert_eq!("h264_qsv".parse::<EncoderProfile>().unwrap(), EncoderProfile::IntelHardware);
        assert_eq!("qsv".parse::<EncoderProfile>().unwrap(), EncoderProfile::IntelHardware);
        assert_eq!(
            "x264enc bitrate=3000".parse::<EncoderProfile>().unwrap(),
            EncoderProfile::Custom("x264enc bitrate=3000".to_string())
        );

        // Test serde deserialization with aliases
        let de_qsv: EncoderProfile = serde_json::from_str(r#""intel-qsv""#).unwrap();
        assert_eq!(de_qsv, EncoderProfile::IntelHardware);
        let de_hw: EncoderProfile = serde_json::from_str(r#""intel-hardware""#).unwrap();
        assert_eq!(de_hw, EncoderProfile::IntelHardware);
    }
}
