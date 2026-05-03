use std::fmt;
use std::path::PathBuf;

use framepipe::capture::types::CaptureBackendKind;
use framepipe::drm_kms::types::{
    BitrateMode, ColorRange, Colorimetry, EncoderBackend, FrameRateMode, Profile, QualityPreset,
    VideoCodec,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Idle,
    Preview,
    Recording,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiPage {
    Configure,
    Record,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceChoice {
    MonitorKms,
    Portal,
}

impl SourceChoice {
    pub const ALL: [SourceChoice; 2] = [SourceChoice::MonitorKms, SourceChoice::Portal];

    pub fn to_backend(self) -> CaptureBackendKind {
        match self {
            SourceChoice::MonitorKms => CaptureBackendKind::DrmKms,
            SourceChoice::Portal => CaptureBackendKind::PipewirePortal,
        }
    }
}

impl fmt::Display for SourceChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceChoice::MonitorKms => write!(f, "Monitor (KMS)"),
            SourceChoice::Portal => write!(f, "Monitor (Portal)"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderChoice {
    Vaapi,
    Qsv,
    Vulkan,
    Cpu,
}

impl EncoderChoice {
    pub const ALL: [EncoderChoice; 4] = [
        EncoderChoice::Vaapi,
        EncoderChoice::Qsv,
        EncoderChoice::Vulkan,
        EncoderChoice::Cpu,
    ];

    pub fn to_encoder(self) -> EncoderBackend {
        match self {
            EncoderChoice::Vaapi => EncoderBackend::Vaapi,
            EncoderChoice::Qsv => EncoderBackend::Qsv,
            EncoderChoice::Vulkan => EncoderBackend::Vulkan,
            EncoderChoice::Cpu => EncoderBackend::Cpu,
        }
    }
}

impl fmt::Display for EncoderChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EncoderChoice::Vaapi => write!(f, "VAAPI"),
            EncoderChoice::Qsv => write!(f, "QSV"),
            EncoderChoice::Vulkan => write!(f, "Vulkan"),
            EncoderChoice::Cpu => write!(f, "CPU"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecChoice {
    H264,
    H265,
    Av1,
}

impl CodecChoice {
    pub const ALL: [CodecChoice; 3] = [CodecChoice::H264, CodecChoice::H265, CodecChoice::Av1];

    pub fn to_codec(self) -> VideoCodec {
        match self {
            CodecChoice::H264 => VideoCodec::H264,
            CodecChoice::H265 => VideoCodec::H265,
            CodecChoice::Av1 => VideoCodec::Av1,
        }
    }
}

impl fmt::Display for CodecChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodecChoice::H264 => write!(f, "H.264"),
            CodecChoice::H265 => write!(f, "H.265"),
            CodecChoice::Av1 => write!(f, "AV1"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityChoice {
    Low,
    Medium,
    High,
    VeryHigh,
    Ultra,
}

impl QualityChoice {
    pub const ALL: [QualityChoice; 5] = [
        QualityChoice::Low,
        QualityChoice::Medium,
        QualityChoice::High,
        QualityChoice::VeryHigh,
        QualityChoice::Ultra,
    ];

    pub fn to_quality(self) -> QualityPreset {
        match self {
            QualityChoice::Low => QualityPreset::Low,
            QualityChoice::Medium => QualityPreset::Medium,
            QualityChoice::High => QualityPreset::High,
            QualityChoice::VeryHigh => QualityPreset::VeryHigh,
            QualityChoice::Ultra => QualityPreset::Ultra,
        }
    }
}

impl fmt::Display for QualityChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QualityChoice::Low => write!(f, "Low"),
            QualityChoice::Medium => write!(f, "Medium"),
            QualityChoice::High => write!(f, "High"),
            QualityChoice::VeryHigh => write!(f, "Very High"),
            QualityChoice::Ultra => write!(f, "Ultra"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorRangeChoice {
    Full,
    Limited,
}

impl ColorRangeChoice {
    pub const ALL: [ColorRangeChoice; 2] = [ColorRangeChoice::Full, ColorRangeChoice::Limited];

    pub fn to_color_range(self) -> ColorRange {
        match self {
            ColorRangeChoice::Full => ColorRange::Full,
            ColorRangeChoice::Limited => ColorRange::Limited,
        }
    }
}

impl fmt::Display for ColorRangeChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ColorRangeChoice::Full => write!(f, "Full"),
            ColorRangeChoice::Limited => write!(f, "Limited"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorimetryChoice {
    Bt709,
    Bt601,
    Bt2020,
}

impl ColorimetryChoice {
    pub const ALL: [ColorimetryChoice; 3] = [
        ColorimetryChoice::Bt709,
        ColorimetryChoice::Bt601,
        ColorimetryChoice::Bt2020,
    ];

    pub fn to_colorimetry(self) -> Colorimetry {
        match self {
            ColorimetryChoice::Bt709 => Colorimetry::Bt709,
            ColorimetryChoice::Bt601 => Colorimetry::Bt601,
            ColorimetryChoice::Bt2020 => Colorimetry::Bt2020,
        }
    }
}

impl fmt::Display for ColorimetryChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ColorimetryChoice::Bt709 => write!(f, "BT.709"),
            ColorimetryChoice::Bt601 => write!(f, "BT.601"),
            ColorimetryChoice::Bt2020 => write!(f, "BT.2020"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameRateModeChoice {
    Cfr,
    Vfr,
}

impl FrameRateModeChoice {
    pub const ALL: [FrameRateModeChoice; 2] = [FrameRateModeChoice::Cfr, FrameRateModeChoice::Vfr];

    pub fn to_mode(self) -> FrameRateMode {
        match self {
            FrameRateModeChoice::Cfr => FrameRateMode::Cfr,
            FrameRateModeChoice::Vfr => FrameRateMode::Vfr,
        }
    }
}

impl fmt::Display for FrameRateModeChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameRateModeChoice::Cfr => write!(f, "CFR"),
            FrameRateModeChoice::Vfr => write!(f, "VFR"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitrateModeChoice {
    Default,
    Cbr,
    Vbr,
    Qvbr,
    Vcm,
    Cqp,
    Icq,
    Quant,
    Qual,
    Pass1,
    Pass2,
    Pass3,
}

impl BitrateModeChoice {
    pub const ALL: [BitrateModeChoice; 12] = [
        BitrateModeChoice::Default,
        BitrateModeChoice::Cbr,
        BitrateModeChoice::Vbr,
        BitrateModeChoice::Qvbr,
        BitrateModeChoice::Vcm,
        BitrateModeChoice::Cqp,
        BitrateModeChoice::Icq,
        BitrateModeChoice::Quant,
        BitrateModeChoice::Qual,
        BitrateModeChoice::Pass1,
        BitrateModeChoice::Pass2,
        BitrateModeChoice::Pass3,
    ];

    pub fn to_mode(self) -> BitrateMode {
        match self {
            BitrateModeChoice::Default => BitrateMode::Default,
            BitrateModeChoice::Cbr => BitrateMode::Cbr,
            BitrateModeChoice::Vbr => BitrateMode::Vbr,
            BitrateModeChoice::Qvbr => BitrateMode::Qvbr,
            BitrateModeChoice::Vcm => BitrateMode::Vcm,
            BitrateModeChoice::Cqp => BitrateMode::Cqp,
            BitrateModeChoice::Icq => BitrateMode::Icq,
            BitrateModeChoice::Quant => BitrateMode::Quant,
            BitrateModeChoice::Qual => BitrateMode::Qual,
            BitrateModeChoice::Pass1 => BitrateMode::Pass1,
            BitrateModeChoice::Pass2 => BitrateMode::Pass2,
            BitrateModeChoice::Pass3 => BitrateMode::Pass3,
        }
    }
}

impl fmt::Display for BitrateModeChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BitrateModeChoice::Default => write!(f, "Default"),
            BitrateModeChoice::Cbr => write!(f, "CBR"),
            BitrateModeChoice::Vbr => write!(f, "VBR"),
            BitrateModeChoice::Qvbr => write!(f, "QVBR"),
            BitrateModeChoice::Vcm => write!(f, "VCM"),
            BitrateModeChoice::Cqp => write!(f, "CQP"),
            BitrateModeChoice::Icq => write!(f, "ICQ"),
            BitrateModeChoice::Quant => write!(f, "Quant"),
            BitrateModeChoice::Qual => write!(f, "Qual"),
            BitrateModeChoice::Pass1 => write!(f, "Pass1"),
            BitrateModeChoice::Pass2 => write!(f, "Pass2"),
            BitrateModeChoice::Pass3 => write!(f, "Pass3"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileChoice {
    Auto,
    Hdr10,
    Hdr,
    Sdr,
}

impl ProfileChoice {
    pub const ALL: [ProfileChoice; 4] = [
        ProfileChoice::Auto,
        ProfileChoice::Hdr10,
        ProfileChoice::Hdr,
        ProfileChoice::Sdr,
    ];

    pub fn to_profile(self) -> Option<Profile> {
        match self {
            ProfileChoice::Auto => None,
            ProfileChoice::Hdr10 => Some(Profile::Hdr10),
            ProfileChoice::Hdr => Some(Profile::Hdr),
            ProfileChoice::Sdr => Some(Profile::Sdr),
        }
    }
}

impl fmt::Display for ProfileChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProfileChoice::Auto => write!(f, "Auto"),
            ProfileChoice::Hdr10 => write!(f, "HDR10"),
            ProfileChoice::Hdr => write!(f, "HDR"),
            ProfileChoice::Sdr => write!(f, "SDR"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FixedOptions {
    pub source: SourceChoice,
    pub card: String,
    pub connector: String,
    pub allow_fallback_connector: bool,

    pub output_width: String,
    pub output_height: String,
    pub dump_frames: bool,
    pub dump_dir: PathBuf,
    pub dump_every: String,

    pub encoder: EncoderChoice,
    pub codec: CodecChoice,
    pub quality: QualityChoice,
    pub frame_rate_mode: FrameRateModeChoice,
    pub bitrate_mode: BitrateModeChoice,
    pub bitrate_input: String,
    pub color_range: ColorRangeChoice,
    pub colorimetry: ColorimetryChoice,
    pub profile: ProfileChoice,

    pub cursor_composition: bool,
    pub wayland_sync_frequency: String,
    pub cursor_hotspot_x: String,
    pub cursor_hotspot_y: String,

    pub output_path: Option<PathBuf>,
}

impl Default for FixedOptions {
    fn default() -> Self {
        Self {
            source: SourceChoice::MonitorKms,
            card: String::new(),
            connector: String::new(),
            allow_fallback_connector: false,
            output_width: String::new(),
            output_height: String::new(),
            dump_frames: false,
            dump_dir: PathBuf::from("./frames"),
            dump_every: "30".to_string(),
            encoder: EncoderChoice::Qsv,
            codec: CodecChoice::H264,
            quality: QualityChoice::High,
            frame_rate_mode: FrameRateModeChoice::Cfr,
            bitrate_mode: BitrateModeChoice::Default,
            bitrate_input: "15000".to_string(),
            color_range: ColorRangeChoice::Full,
            colorimetry: ColorimetryChoice::Bt709,
            profile: ProfileChoice::Auto,
            cursor_composition: true,
            wayland_sync_frequency: "0.5".to_string(),
            cursor_hotspot_x: "0".to_string(),
            cursor_hotspot_y: "0".to_string(),
            output_path: None,
        }
    }
}
