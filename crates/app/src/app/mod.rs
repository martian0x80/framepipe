pub mod app;
pub mod cli;
pub mod config;
pub mod signals;

use eyre::Result;
use std::thread;

use crate::drm_kms::{probe, types::CaptureOutput};

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
            let resolved_output = output.unwrap_or_else(|| {
                let fmt = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
                let filename = format!("framepipe_record_{}.mp4", fmt);
                if let Some(mut dir) = dirs::video_dir() {
                    dir.push(filename);
                    dir
                } else {
                    std::path::PathBuf::from(filename)
                }
            });
            let backend = capture.capture_backend;
            let options = config::build_capture_options(capture, CaptureOutput::File(resolved_output))?;
            let control = CaptureControl::register().map_err(|e| eyre::eyre!(e))?;
            RecordingSession::new(options, backend)?.run(control)
        }
        Commands::Preview { capture } => {
            let backend = capture.capture_backend;
            let options = config::build_capture_options(capture, CaptureOutput::Preview)?;
            let control = CaptureControl::register().map_err(|e| eyre::eyre!(e))?;
            RecordingSession::new(options, backend)?.run(control)
        }
        Commands::Test => {
            thread::spawn(|| {
                let egl_ctx = crate::drm_kms::egl_context::init_egl("").unwrap();
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async {
                    crate::portal::pipewire::screencast_session(1000, &egl_ctx.egl, egl_ctx.display)
                        .await
                        .unwrap();
                    println!("portal dmabuf/cursor poc done");
                })
            })
            .join()
            .unwrap();
            Ok(())
        }
    }
}
