use arc_swap::ArcSwap;
use drm::Device as BasicDevice;
use drm::control::Device as ControlDevice;
use std::fmt::Display;
use std::fs::{File, OpenOptions};
use std::os::fd::OwnedFd;
use std::os::unix::io::{AsFd, BorrowedFd};
use std::path::PathBuf;
use std::sync::Arc;

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

impl Display for FrameRateMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameRateMode::Cfr => write!(f, "cfr"),
            FrameRateMode::Vfr => write!(f, "vfr"),
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

impl Display for BitrateMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BitrateMode::Cbr => write!(f, "cbr"),
            BitrateMode::Vbr => write!(f, "vbr"),
            BitrateMode::Qvbr => write!(f, "qvbr"),
            BitrateMode::Vcm => write!(f, "vcm"),
            BitrateMode::Cqp => write!(f, "cqp"),
            BitrateMode::Icq => write!(f, "icq"),
            BitrateMode::Default => write!(f, "default"),
            BitrateMode::Quant => write!(f, "quant"),
            BitrateMode::Qual => write!(f, "qual"),
            BitrateMode::Pass1 => write!(f, "pass1"),
            BitrateMode::Pass2 => write!(f, "pass2"),
            BitrateMode::Pass3 => write!(f, "pass3"),
        }
    }
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum ColorRange {
    Full,
    Limited,
}

impl Display for ColorRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ColorRange::Full => write!(f, "full"),
            ColorRange::Limited => write!(f, "limited"),
        }
    }
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum Colorimetry {
    Bt601,
    Bt709,
    Bt2020,
}

impl Display for Colorimetry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Colorimetry::Bt601 => write!(f, "bt601"),
            Colorimetry::Bt709 => write!(f, "bt709"),
            Colorimetry::Bt2020 => write!(f, "bt2020"),
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

impl Display for QualityPreset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QualityPreset::Low => write!(f, "low"),
            QualityPreset::Medium => write!(f, "medium"),
            QualityPreset::High => write!(f, "high"),
            QualityPreset::VeryHigh => write!(f, "veryhigh"),
            QualityPreset::Ultra => write!(f, "ultra"),
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

impl Display for EncoderBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EncoderBackend::Vaapi => write!(f, "vaapi"),
            EncoderBackend::Qsv => write!(f, "qsv"),
            EncoderBackend::Vulkan => write!(f, "vulkan"),
            EncoderBackend::Cpu => write!(f, "cpu"),
        }
    }
}

#[derive(Debug, Clone, clap::ValueEnum, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
    H265,
    Av1,
}

impl Display for VideoCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VideoCodec::H264 => write!(f, "h264"),
            VideoCodec::H265 => write!(f, "h265"),
            VideoCodec::Av1 => write!(f, "av1"),
        }
    }
}

#[derive(Debug, Clone, Copy, clap::ValueEnum, PartialEq, Eq)]
pub enum Profile {
    Hdr10, // 10 bit (main-10) + bt2020 + P010_10LE format (+ transfer function)
    Hdr,   // default bit depth (main) + bt2020 + NV12 format
    Sdr,   // default bit depth (main) + bt709 + NV12 format
}

impl Display for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Profile::Hdr10 => write!(f, "hdr10"),
            Profile::Hdr => write!(f, "hdr"),
            Profile::Sdr => write!(f, "sdr"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum CaptureOutput {
    File(PathBuf),
    Preview,
    EmbeddedPreview,
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
    pub profile: Option<Profile>,
    /// Background image path for frame compositing.  When `Some`, the source
    /// frame is scaled to `background_zoom` and composited on top of this image.
    pub background: Option<PathBuf>,
    /// Scale factor [1.0, 100.0] for the source frame when background is active.
    pub background_zoom: f32,
    pub preview_mailbox: Option<common::types::PreviewMailbox>,
    /// Live-mutable settings that the recording loop re-reads every frame.
    /// Only fields that are actually used inside the per-frame loop are included;
    /// encoder-only and init-time-only fields are intentionally excluded.
    pub live_settings: Option<LiveSettingsMailbox>,
}

/// Settings that the GUI can update during a running capture session.
///
/// Only fields that are consumed inside the per-frame loop of `recording_loop`
/// are included here. Encoder parameters, session-init parameters (card path,
/// output size, etc.) are intentionally excluded because they
/// cannot take effect without restarting the session.
// This should probably be in `common` but oh well.
#[derive(Debug, Clone)]
pub struct LiveSettings {
    pub fps: u32,

    // --- cursor smoothing ---
    pub cursor_smooth: bool,
    pub cursor_spring_k: f32,
    pub cursor_spring_d: f32,
    pub cursor_max_speed: f32,
    pub cursor_snap_px: f32,
    pub cursor_smooth_ms: f32,
    pub cursor_deadzone_px: f32,

    // --- cursor smear / motion-blur trail ---
    pub cursor_smear: bool,
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

    // --- cursor sprite ---
    /// Path to cursor sprite PNG.  `None` = use the built-in default shape.
    pub cursor_sprite: Option<PathBuf>,
    /// Bumped by the GUI whenever the sprite path changes.  The recording loop
    /// compares this against a local `last_cursor_sprite_version` and rebuilds
    /// the GL texture exactly once on mismatch.
    pub cursor_sprite_version: u64,
    pub cursor_scale: f32,

    // --- background compositing ---
    /// Path to a background PNG.  `None` = no background.
    pub background: Option<PathBuf>,
    /// Bumped by the GUI whenever the background path changes.
    pub background_version: u64,
    /// Whether to composite the background behind the (zoomed) source frame.
    pub background_enabled: bool,
    /// Uniform scale applied to the source frame when background is enabled.
    /// 1.0 = fills output completely, 0.85 = leaves a visible border all around.
    pub background_zoom: f32,
}

impl Default for LiveSettings {
    fn default() -> Self {
        // Mirrors the defaults in CaptureArgs::default().
        Self {
            fps: 60,
            cursor_smooth: false,
            cursor_spring_k: 120.0,
            cursor_spring_d: 18.0,
            cursor_max_speed: 3000.0,
            cursor_snap_px: 0.0,
            cursor_smooth_ms: 12.0,
            cursor_deadzone_px: 0.5,
            cursor_smear: false,
            cursor_smear_speed_threshold: 100.0,
            cursor_smear_shutter_scale: 4.0,
            cursor_smear_min_len: 4.0,
            cursor_smear_max_len: 220.0,
            cursor_smear_taps: 8,
            cursor_smear_alpha_exp: 1.1,
            cursor_smear_alpha_scale: 0.4,
            cursor_smear_stretch_threshold: 300.0,
            cursor_smear_stretch_range: 1800.0,
            cursor_smear_max_stretch: 2.0,
            cursor_smear_max_squash: 0.15,
            cursor_sprite: None,
            cursor_sprite_version: 0,
            cursor_scale: 50.0,
            background: None,
            background_version: 0,
            background_enabled: false,
            background_zoom: 85.0,
        }
    }
}

impl LiveSettings {
    pub fn from_options(opts: &CaptureOptions) -> Self {
        Self {
            fps: opts.fps,
            cursor_smooth: opts.cursor_smooth,
            cursor_spring_k: opts.cursor_spring_k,
            cursor_spring_d: opts.cursor_spring_d,
            cursor_max_speed: opts.cursor_max_speed,
            cursor_snap_px: opts.cursor_snap_px,
            cursor_smooth_ms: opts.cursor_smooth_ms,
            cursor_deadzone_px: opts.cursor_deadzone_px,
            cursor_smear: opts.cursor_smear,
            cursor_smear_speed_threshold: opts.cursor_smear_speed_threshold,
            cursor_smear_shutter_scale: opts.cursor_smear_shutter_scale,
            cursor_smear_min_len: opts.cursor_smear_min_len,
            cursor_smear_max_len: opts.cursor_smear_max_len,
            cursor_smear_taps: opts.cursor_smear_taps,
            cursor_smear_alpha_exp: opts.cursor_smear_alpha_exp,
            cursor_smear_alpha_scale: opts.cursor_smear_alpha_scale,
            cursor_smear_stretch_threshold: opts.cursor_smear_stretch_threshold,
            cursor_smear_stretch_range: opts.cursor_smear_stretch_range,
            cursor_smear_max_stretch: opts.cursor_smear_max_stretch,
            cursor_smear_max_squash: opts.cursor_smear_max_squash,
            // Seed from options so the first version matches the already-loaded texture.
            cursor_sprite: opts.cursor_sprite.clone(),
            cursor_sprite_version: 0,
            cursor_scale: opts.cursor_scale.clamp(1.0, 100.0),
            background: opts.background.clone(),
            background_version: 0,
            background_enabled: opts.background.is_some(),
            background_zoom: opts.background_zoom.clamp(1.0, 100.0),
        }
    }
}

/// Shared handle for live-updating [`LiveSettings`] between the GUI thread
/// and the capture recording loop.
#[derive(Debug)]
pub struct LiveSettingsMailbox {
    inner: Arc<ArcSwap<LiveSettings>>,
}

impl LiveSettingsMailbox {
    pub fn new(initial: LiveSettings) -> Self {
        Self {
            inner: Arc::new(ArcSwap::from_pointee(initial)),
        }
    }

    pub fn update(&self, settings: LiveSettings) {
        self.inner.store(Arc::new(settings));
    }

    pub fn get(&self) -> Arc<LiveSettings> {
        self.inner.load_full()
    }
}

impl Clone for LiveSettingsMailbox {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}
