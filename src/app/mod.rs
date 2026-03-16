pub mod app;
pub mod cli;
pub mod config;
pub mod pipeline;

use eyre::Result;

use crate::{
    drm_kms::{probe, types::CaptureOutput},
    wayland::layer::init_wayland,
};

use self::{app::RecordingSession, cli::{Cli, Commands}};

pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::ListConnectors { card } => {
            let card_path = config::resolve_card_path(card)?;
            let connectors = probe::list_connectors(&card_path)?;
            for (idx, c) in connectors.iter().enumerate() {
                println!("{idx}: {c}");
            }
            Ok(())
        }
        Commands::Record { capture, output } => {
            let options = config::build_capture_options(
                capture,
                CaptureOutput::File(output),
            )?;
            RecordingSession::new(options)?.run()
        }
        Commands::Preview { capture } => {
            let options = config::build_capture_options(capture, CaptureOutput::Preview)?;
            RecordingSession::new(options)?.run()
        }
        Commands::Test { sync_frequency_hz } => {
            init_wayland(sync_frequency_hz)?;
            Ok(())
        }
    }
}
