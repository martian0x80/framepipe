use clap::{Args, Parser, Subcommand};
use env_logger;
use eyre::Result;
use std::path::PathBuf;

use crate::drm_kms::probe;
use crate::drm_kms::types::{
    BitrateMode, CaptureOptions, CaptureOutput, ColorRange, EncoderBackend, FrameRateMode,
};

mod drm_kms;

#[derive(Parser, Debug)]
#[command(name = "openstudio")]
#[command(about = "GPU screen capture prototype", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
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
}


#[derive(Args, Debug, Clone)]
struct CaptureArgs {
    #[arg(long)]
    card: Option<String>,
    #[arg(long)]
    connector: Option<String>,
    #[arg(long, default_value_t = false)]
    allow_fallback_connector: bool,
    #[arg(long, default_value_t = 60)]
    fps: u32,
    #[arg(long)]
    output_width: Option<u32>,
    #[arg(long)]
    output_height: Option<u32>,
    #[arg(long, default_value_t = false)]
    dump_frames: bool,
    #[arg(long, default_value = "./frames")]
    dump_dir: PathBuf,
    #[arg(long, default_value_t = 30)]
    dump_every: u32,
    #[arg(long, default_value_t = 15000)]
    bitrate_kbps: u32,
    #[arg(short = 'f', long, default_value_t = FrameRateMode::Cfr)]
    frame_rate_mode: FrameRateMode,
    #[arg(short = 'b', long, default_value_t = BitrateMode::Cbr)]
    bitrate_mode: BitrateMode,
    #[arg(short = 'c', long, default_value_t = ColorRange::Full)]
    color_range: ColorRange,
    #[arg(long, default_value_t = EncoderBackend::Vaapi)]
    encoder_backend: EncoderBackend,
}

fn resolve_card_path(card: Option<String>) -> Result<String> {
    if let Some(card) = card {
        return Ok(card);
    }
    let cards = probe::get_dri_cards()?;
    cards
        .last()
        .cloned()
        .ok_or_else(|| eyre::eyre!("no DRM cards found"))
}

fn main() -> Result<()> {
    env_logger::builder().format_timestamp_nanos().filter_level(log::LevelFilter::Debug).init();
    let cli = Cli::parse();

    match cli.command {
        Commands::ListConnectors { card } => {
            let card_path = resolve_card_path(card)?;
            let connectors = probe::list_connectors(&card_path)?;
            for (idx, c) in connectors.iter().enumerate() {
                println!("{idx}: {c}");
            }
        }
        Commands::Record { capture, output } => {
            if capture.output_width.is_some() != capture.output_height.is_some() {
                return Err(eyre::eyre!(
                    "both --output-width and --output-height must be set together"
                ));
            }
            let card_path = resolve_card_path(capture.card)?;
            let opts = CaptureOptions {
                card_path,
                connector: capture.connector,
                allow_fallback_connector: capture.allow_fallback_connector,
                fps: capture.fps,
                output_width: capture.output_width,
                output_height: capture.output_height,
                dump_frames: capture.dump_frames,
                dump_dir: capture.dump_dir,
                dump_every: capture.dump_every,
                output: CaptureOutput::File(output),
                bitrate_kbps: capture.bitrate_kbps,
                frame_rate_mode: capture.frame_rate_mode,
                bitrate_mode: capture.bitrate_mode,
                color_range: capture.color_range,
                encoder_backend: capture.encoder_backend,
            };
            drm_kms::egl::egl_main(opts)?;
        }
        Commands::Preview { capture } => {
            if capture.output_width.is_some() != capture.output_height.is_some() {
                return Err(eyre::eyre!(
                    "both --output-width and --output-height must be set together"
                ));
            }
            let card_path = resolve_card_path(capture.card)?;
            let opts = CaptureOptions {
                card_path,
                connector: capture.connector,
                allow_fallback_connector: capture.allow_fallback_connector,
                fps: capture.fps,
                output_width: capture.output_width,
                output_height: capture.output_height,
                dump_frames: capture.dump_frames,
                dump_dir: capture.dump_dir,
                dump_every: capture.dump_every,
                output: CaptureOutput::Preview,
                bitrate_kbps: capture.bitrate_kbps,
                frame_rate_mode: capture.frame_rate_mode,
                bitrate_mode: capture.bitrate_mode,
                color_range: capture.color_range,
                encoder_backend: capture.encoder_backend,
            };
            drm_kms::egl::egl_main(opts)?;
        }
    }

    Ok(())
}
