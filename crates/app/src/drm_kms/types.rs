use drm::Device as BasicDevice;
use drm::control::Device as ControlDevice;
use std::fs::{File, OpenOptions};
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::io::{AsFd, BorrowedFd};
use std::path::PathBuf;

#[derive(Debug)]
pub(crate) struct Card(File);

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl BasicDevice for Card {}
impl ControlDevice for Card {}

impl Card {
    pub fn open(path: &str) -> std::io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        Ok(Card(file))
    }

    pub fn from_owned_fd(fd: OwnedFd) -> Self {
        Card(File::from(fd))
    }

    pub fn try_clone(&self) -> std::io::Result<Self> {
        self.0.try_clone().map(Card)
    }

    pub fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

#[derive(Debug)]
pub struct ProbeResult {
    pub fb_id: u32,
    pub fb_info: drm::control::framebuffer::PlanarInfo,
    pub plane_fds: Vec<Option<OwnedFd>>, // index matches fb_info planes
}

/// Represents a framebuffer exported as DMABuf fds, ready for encoding or preview. (not prime fds from drm api)
pub struct ExportedDmabuf {
    pub width: i32,
    pub height: i32,
    pub fourcc: u32,
    pub modifier: u64,
    pub fds: Vec<OwnedFd>,
    pub strides: Vec<i32>,
    pub offsets: Vec<i32>,
    pub acquire_fence_fd: Option<OwnedFd>,
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum FrameRateMode {
    Cfr,
    Vfr,
}

impl ToString for FrameRateMode {
    fn to_string(&self) -> String {
        match self {
            FrameRateMode::Cfr => "cfr".to_string(),
            FrameRateMode::Vfr => "vfr".to_string(),
        }
    }
}

#[derive(Debug, Clone, clap::ValueEnum, PartialEq, Eq)]
pub enum BitrateMode {
    Cbr,
    Vbr,
    Qvbr,
    Vcm,
    Cqp,
    Icq,
    Default,
    Quant,
    Qual,
    Pass1,
    Pass2,
    Pass3,
}

impl ToString for BitrateMode {
    fn to_string(&self) -> String {
        match self {
            BitrateMode::Cbr => "cbr".to_string(),
            BitrateMode::Vbr => "vbr".to_string(),
            BitrateMode::Qvbr => "qvbr".to_string(),
            BitrateMode::Vcm => "vcm".to_string(),
            BitrateMode::Cqp => "cqp".to_string(),
            BitrateMode::Icq => "icq".to_string(),
            BitrateMode::Default => "default".to_string(),
            BitrateMode::Quant => "quant".to_string(),
            BitrateMode::Qual => "qual".to_string(),
            BitrateMode::Pass1 => "pass1".to_string(),
            BitrateMode::Pass2 => "pass2".to_string(),
            BitrateMode::Pass3 => "pass3".to_string(),
        }
    }
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum ColorRange {
    Full,
    Limited,
}

impl ToString for ColorRange {
    fn to_string(&self) -> String {
        match self {
            ColorRange::Full => "full".to_string(),
            ColorRange::Limited => "limited".to_string(),
        }
    }
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum Colorimetry {
    Bt601,
    Bt709,
    Bt2020,
}

impl ToString for Colorimetry {
    fn to_string(&self) -> String {
        match self {
            Colorimetry::Bt601 => "bt601".to_string(),
            Colorimetry::Bt709 => "bt709".to_string(),
            Colorimetry::Bt2020 => "bt2020".to_string(),
        }
    }
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum QualityPreset {
    Low,
    Medium,
    High,
    VeryHigh,
    Ultra,
}

impl ToString for QualityPreset {
    fn to_string(&self) -> String {
        match self {
            QualityPreset::Low => "low".to_string(),
            QualityPreset::Medium => "medium".to_string(),
            QualityPreset::High => "high".to_string(),
            QualityPreset::VeryHigh => "veryhigh".to_string(),
            QualityPreset::Ultra => "ultra".to_string(),
        }
    }
}

#[derive(Debug, Clone, clap::ValueEnum, PartialEq, Eq)]
pub enum EncoderBackend {
    Vaapi,
    Qsv,
    Vulkan,
    Cpu,
}

impl ToString for EncoderBackend {
    fn to_string(&self) -> String {
        match self {
            EncoderBackend::Vaapi => "vaapi".to_string(),
            EncoderBackend::Qsv => "qsv".to_string(),
            EncoderBackend::Vulkan => "vulkan".to_string(),
            EncoderBackend::Cpu => "cpu".to_string(),
        }
    }
}

#[derive(Debug, Clone, clap::ValueEnum, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
    H265,
    Av1,
}

impl ToString for VideoCodec {
    fn to_string(&self) -> String {
        match self {
            VideoCodec::H264 => "h264".to_string(),
            VideoCodec::H265 => "h265".to_string(),
            VideoCodec::Av1 => "av1".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, clap::ValueEnum, PartialEq, Eq)]
pub enum Profile {
    Hdr10, // 10 bit (main-10) + bt2020 + P010_10LE format (+ transfer funct
    Hdr, // default bit depth (main) + bt2020 + NV12 format
    Sdr, // default bit depth (main) + bt709 + NV12 format
}

impl ToString for Profile {
    fn to_string(&self) -> String {
        match self {
            Profile::Hdr10 => "hdr10".to_string(),
            Profile::Hdr => "hdr".to_string(),
            Profile::Sdr => "sdr".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum CaptureOutput {
    File(PathBuf),
    Preview,
}

#[derive(Debug, Clone)]
pub struct CaptureOptions {
    pub card_path: String,
    pub connector: Option<String>,
    pub allow_fallback_connector: bool,
    pub fps: u32,
    pub output_width: Option<u32>,
    pub output_height: Option<u32>,
    pub dump_frames: bool,
    pub dump_dir: PathBuf,
    pub dump_every: u32,
    pub output: CaptureOutput,
    pub bitrate_kbps: u32,
    pub frame_rate_mode: FrameRateMode,
    pub bitrate_mode: BitrateMode,
    pub quality: QualityPreset,
    pub color_range: ColorRange,
    pub colorimetry: Colorimetry,
    pub encoder_backend: EncoderBackend,
    pub video_codec: VideoCodec,
    pub mouse_tracking: bool,
    pub cursor_composition: bool,
    pub cursor_sprite: Option<PathBuf>,
    pub cursor_hotspot_x: i32,
    pub cursor_hotspot_y: i32,
    pub cursor_scale: f32,
    pub cursor_smooth: bool,
    pub cursor_smear: bool,
    pub cursor_spring_k: f32,
    pub cursor_spring_d: f32,
    pub cursor_max_speed: f32,
    pub cursor_snap_px: f32,
    pub cursor_smooth_ms: f32,
    pub cursor_deadzone_px: f32,
    pub cursor_smear_speed_threshold: f32,
    pub cursor_smear_shutter_scale: f32,
    pub cursor_smear_min_len: f32,
    pub cursor_smear_max_len: f32,
    pub cursor_smear_taps: u32,
    pub cursor_smear_alpha_exp: f32,
    pub cursor_smear_alpha_scale: f32,
    pub cursor_smear_stretch_threshold: f32,
    pub cursor_smear_stretch_range: f32,
    pub cursor_smear_max_stretch: f32,
    pub cursor_smear_max_squash: f32,
    pub wayland_sync_frequency: f64,
    pub mouse_tracking_file: PathBuf,
    pub profile: Option<Profile>,
}
