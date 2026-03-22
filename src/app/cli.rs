use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

use crate::drm_kms::types::{
    BitrateMode, ColorRange, Colorimetry, EncoderBackend, FrameRateMode, QualityPreset,
    VideoCodec,
};
use crate::postfx::types::{BlendMode, MouseEffect};

#[derive(Parser, Debug)]
#[command(name = "openstudio")]
#[command(about = "GPU screen capture prototype", long_about = None)]
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
    Postfx {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[command(flatten)]
        options: PostfxArgs,
    },
    /// Preview live frames until stopped
    Preview {
        #[command(flatten)]
        capture: CaptureArgs,
    },
    Test {
        #[arg(long, default_value_t = 0.1)]
        sync_frequency_hz: f64,
    },
}

#[derive(Args, Debug, Clone)]
pub struct PostfxArgs {
    #[arg(long, default_value = "openstudio-cursor.bitcode")]
    pub mouse_track: PathBuf,
    #[arg(long, value_enum, default_value_t = MouseEffect::Sprite)]
    pub effect: MouseEffect,
    #[arg(long)]
    pub sprite: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t = BlendMode::Alpha)]
    pub blend_mode: BlendMode,
    #[arg(long, default_value_t = 1.0)]
    pub opacity: f32,
    #[arg(long, default_value_t = 1.0)]
    pub scale: f32,
    #[arg(long, default_value_t = 0)]
    pub hotspot_x: i32,
    #[arg(long, default_value_t = 0)]
    pub hotspot_y: i32,
    #[arg(long, default_value_t = 24)]
    pub smoothing_ms: u32,
    #[arg(long, default_value_t = 2.0)]
    pub zoom_factor: f32,
    #[arg(long, default_value_t = 220.0)]
    pub zoom_radius_px: f32,
    #[arg(long, default_value_t = 170.0)]
    pub spotlight_radius_px: f32,
    #[arg(long, default_value_t = 0.45)]
    pub spotlight_softness: f32,
    #[arg(short = 'e', long, default_value_t = EncoderBackend::Qsv)]
    pub encoder_backend: EncoderBackend,
    #[arg(short = 'v', long, default_value_t = VideoCodec::H264)]
    pub video_codec: VideoCodec,
    #[arg(short = 'q', long, default_value_t = QualityPreset::High)]
    pub quality: QualityPreset,
    #[arg(short = 'r', long = "rate-control", default_value_t = BitrateMode::Default)]
    pub bitrate_mode: BitrateMode,
    #[arg(short = 'b', long, default_value_t = 15000)]
    pub bitrate_kbps: u32,
    #[arg(long, default_value_t = 60)]
    pub fps: u32,
    #[arg(long, default_value_t = ColorRange::Full)]
    pub color_range: ColorRange,
    #[arg(short = 'i', long, default_value_t = Colorimetry::Bt709)]
    pub colorimetry: Colorimetry,
}

#[derive(Args, Debug, Clone)]
pub struct CaptureArgs {
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
    #[arg(short = 'e', long, default_value_t = EncoderBackend::Vaapi)]
    pub encoder_backend: EncoderBackend,
    #[arg(short = 'v', long, default_value_t = VideoCodec::H264)]
    pub video_codec: VideoCodec,
    #[arg(short = 'm', long, default_value_t = false)]
    pub mouse_tracking: bool,
    #[arg(long, default_value_t = 0.1, help = "Frequency to sync mouse tracking data to Wayland layer (in Hz)")]
    pub wayland_sync_frequency: f64,
    #[arg(long, default_value = "openstudio-cursor.bitcode", help = "File path to write mouse tracking data to")]
    pub mouse_tracking_file: String,
}
