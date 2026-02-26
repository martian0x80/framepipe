use clap::{Args, Parser, Subcommand};
use env_logger;
use eyre::Result;
use std::path::PathBuf;

use crate::drm_kms::probe;

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
    #[arg(long, default_value_t = false)]
    dump_frames: bool,
    #[arg(long, default_value = "./openstudio-frames")]
    dump_dir: PathBuf,
    #[arg(long, default_value_t = 30)]
    dump_every: u32,
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
    env_logger::builder().filter_level(log::LevelFilter::Debug).init();
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
            let card_path = resolve_card_path(capture.card)?;
            let opts = drm_kms::egl::CaptureOptions {
                card_path,
                connector: capture.connector,
                allow_fallback_connector: capture.allow_fallback_connector,
                fps: capture.fps,
                dump_frames: capture.dump_frames,
                dump_dir: capture.dump_dir,
                dump_every: capture.dump_every,
                output: drm_kms::egl::CaptureOutput::File(output),
            };
            drm_kms::egl::egl_main(opts)?;
        }
        Commands::Preview { capture } => {
            let card_path = resolve_card_path(capture.card)?;
            let opts = drm_kms::egl::CaptureOptions {
                card_path,
                connector: capture.connector,
                allow_fallback_connector: capture.allow_fallback_connector,
                fps: capture.fps,
                dump_frames: capture.dump_frames,
                dump_dir: capture.dump_dir,
                dump_every: capture.dump_every,
                output: drm_kms::egl::CaptureOutput::Preview,
            };
            drm_kms::egl::egl_main(opts)?;
        }
    }

    Ok(())
}
