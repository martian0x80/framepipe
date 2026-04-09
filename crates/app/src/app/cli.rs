use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

use crate::capture::types::CaptureBackendKind;
use crate::drm_kms::types::{
    BitrateMode, ColorRange, Colorimetry, EncoderBackend, FrameRateMode, Profile, QualityPreset,
    VideoCodec,
};

#[derive(Parser, Debug)]
#[command(name = "framepipe")]
#[command(about = "GPU Screen Recorder", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// List connected connectors in probe order
    ListConnectors {
        #[arg(long)]
        card: Option<String>,
    },
    /// Record to file until stopped
    Record {
        #[command(flatten)]
        capture: CaptureArgs,
        #[arg(long, default_value = "output.mp4")]
        output: PathBuf,
    },
    /// Preview live frames until stopped
    Preview {
        #[command(flatten)]
        capture: CaptureArgs,
    },
    Test,
}

#[derive(Args, Debug, Clone)]
pub struct CaptureArgs {
    #[arg(long = "capture-backend", default_value_t = CaptureBackendKind::DrmKms)]
    pub capture_backend: CaptureBackendKind,
    #[arg(long)]
    pub card: Option<String>,
    #[arg(long)]
    pub connector: Option<String>,
    #[arg(long, default_value_t = false)]
    pub allow_fallback_connector: bool,
    #[arg(long, default_value_t = 60)]
    pub fps: u32,
    #[arg(long)]
    pub output_width: Option<u32>,
    #[arg(long)]
    pub output_height: Option<u32>,
    #[arg(long, default_value_t = false)]
    pub dump_frames: bool,
    #[arg(long, default_value = "./frames")]
    pub dump_dir: PathBuf,
    #[arg(long, default_value_t = 30)]
    pub dump_every: u32,
    #[arg(short = 'b', long, default_value_t = 15000)]
    pub bitrate_kbps: u32,
    #[arg(short = 'f', long, default_value_t = FrameRateMode::Cfr)]
    pub frame_rate_mode: FrameRateMode,
    #[arg(
        short = 'r',
        long = "rate-control",
        alias = "bitrate-mode",
        default_value_t = BitrateMode::Default
    )]
    pub bitrate_mode: BitrateMode,
    #[arg(short = 'q', long, default_value_t = QualityPreset::High)]
    pub quality: QualityPreset,
    #[arg(long, default_value_t = ColorRange::Full)]
    pub color_range: ColorRange,
    #[arg(short = 'i', long, default_value_t = Colorimetry::Bt709)]
    pub colorimetry: Colorimetry,
    #[arg(short = 'e', long, default_value_t = EncoderBackend::Qsv)]
    pub encoder_backend: EncoderBackend,
    #[arg(short = 'v', long, default_value_t = VideoCodec::H264)]
    pub video_codec: VideoCodec,
    #[arg(short = 'm', long, default_value_t = false)]
    pub mouse_tracking: bool,
    #[arg(long, default_value_t = false)]
    pub cursor_composition: bool,
    #[arg(long, help = "PNG sprite for live cursor composition")]
    pub cursor_sprite: Option<PathBuf>,
    #[arg(long, default_value_t = 0, help = "Cursor hotspot X in sprite pixels")]
    pub cursor_hotspot_x: i32,
    #[arg(long, default_value_t = 0, help = "Cursor hotspot Y in sprite pixels")]
    pub cursor_hotspot_y: i32,
    #[arg(long, default_value_t = 1.0, help = "Cursor sprite scale factor")]
    pub cursor_scale: f32,
    #[arg(long, default_value_t = false, help = "Enable cursor smoothing")]
    pub cursor_smooth: bool,
    #[arg(
        long,
        default_value_t = false,
        help = "Enable cursor smear (motion trail)"
    )]
    pub cursor_smear: bool,
    #[arg(
        long = "cursor-smooth-spring-k",
        default_value_t = 120.0,
        help = "Cursor spring stiffness (only used with --cursor-smooth)"
    )]
    pub cursor_spring_k: f32,
    #[arg(
        long = "cursor-smooth-spring-d",
        default_value_t = 18.0,
        help = "Cursor spring damping (only used with --cursor-smooth)"
    )]
    pub cursor_spring_d: f32,
    #[arg(
        long = "cursor-smooth-max-speed",
        default_value_t = 3000.0,
        help = "Cursor spring max speed (px/s, only used with --cursor-smooth)"
    )]
    pub cursor_max_speed: f32,
    #[arg(
        long = "cursor-smooth-snap-px",
        default_value_t = 0.0,
        help = "Snap to target when distance exceeds this (px, only used with --cursor-smooth)"
    )]
    pub cursor_snap_px: f32,
    #[arg(
        long = "cursor-smooth-ms",
        default_value_t = 12.0,
        help = "Cursor target smoothing time constant (ms, only used with --cursor-smooth)"
    )]
    pub cursor_smooth_ms: f32,
    #[arg(
        long = "cursor-smooth-deadzone-px",
        default_value_t = 0.5,
        help = "Ignore jitter under this distance (px, only used with --cursor-smooth)"
    )]
    pub cursor_deadzone_px: f32,
    #[arg(
        long = "cursor-smear-speed-threshold",
        default_value_t = 100.0,
        help = "Minimum cursor speed (px/s) required to emit smear trail"
    )]
    pub cursor_smear_speed_threshold: f32,
    #[arg(
        long = "cursor-smear-shutter-scale",
        default_value_t = 4.0,
        help = "Smear shutter multiplier in frame-time units"
    )]
    pub cursor_smear_shutter_scale: f32,
    #[arg(
        long = "cursor-smear-min-len",
        default_value_t = 4.0,
        help = "Minimum smear length in pixels once smear is active"
    )]
    pub cursor_smear_min_len: f32,
    #[arg(
        long = "cursor-smear-max-len",
        default_value_t = 220.0,
        help = "Maximum smear length in pixels"
    )]
    pub cursor_smear_max_len: f32,
    #[arg(
        long = "cursor-smear-taps",
        default_value_t = 8,
        help = "Number of additional smear taps (max 8)"
    )]
    pub cursor_smear_taps: u32,
    #[arg(
        long = "cursor-smear-alpha-exp",
        default_value_t = 1.1,
        help = "Smear alpha falloff exponent"
    )]
    pub cursor_smear_alpha_exp: f32,
    #[arg(
        long = "cursor-smear-alpha-scale",
        default_value_t = 0.4,
        help = "Overall smear alpha scale"
    )]
    pub cursor_smear_alpha_scale: f32,
    #[arg(
        long = "cursor-smear-stretch-threshold",
        default_value_t = 300.0,
        help = "Speed threshold (px/s) where stretch deformation starts"
    )]
    pub cursor_smear_stretch_threshold: f32,
    #[arg(
        long = "cursor-smear-stretch-range",
        default_value_t = 1800.0,
        help = "Speed range (px/s) to reach maximum stretch"
    )]
    pub cursor_smear_stretch_range: f32,
    #[arg(
        long = "cursor-smear-max-stretch",
        default_value_t = 2.0,
        help = "Maximum extra stretch factor (final = 1 + value)"
    )]
    pub cursor_smear_max_stretch: f32,
    #[arg(
        long = "cursor-smear-max-squash",
        default_value_t = 0.15,
        help = "Maximum perpendicular squash amount (final = 1 - value)"
    )]
    pub cursor_smear_max_squash: f32,
    #[arg(
        long,
        default_value_t = 0.5,
        help = "Frequency to sync mouse tracking data to Wayland layer (in Hz)"
    )]
    pub wayland_sync_frequency: f64,
    #[arg(
        long,
        default_value = "openstudio-cursor.bitcode",
        help = "File path to write mouse tracking data to"
    )]
    pub mouse_tracking_file: String,
    #[arg(long, help = "Capture profile (hdr10, hdr, sdr)")]
    pub profile: Option<Profile>,
}
