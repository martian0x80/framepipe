pub mod app;
pub mod cli;
pub mod config;
pub mod pipeline;
pub mod signals;

use eyre::Result;

use crate::{
    drm_kms::{probe, types::CaptureOutput},
    postfx::renderer
};

use self::{
    app::RecordingSession,
    cli::{Cli, Commands},
    signals::CaptureControl,
};

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
            let options = config::build_capture_options(capture, CaptureOutput::File(output))?;
            let control = CaptureControl::register().map_err(|e| eyre::eyre!(e))?;
            RecordingSession::new(options)?.run(control)
        }
        Commands::Preview { capture } => {
            let options = config::build_capture_options(capture, CaptureOutput::Preview)?;
            let control = CaptureControl::register().map_err(|e| eyre::eyre!(e))?;
            RecordingSession::new(options)?.run(control)
        }
        Commands::Postfx {
            input,
            output,
            options,
        } => {
            let setup = config::build_postfx_setup(input, output, options)?;
            renderer::apply_postfx(setup).map_err(Into::into)
        }
    }
}
