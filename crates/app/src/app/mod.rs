#[allow(clippy::module_inception)]
pub mod app;
pub mod cli;
pub mod config;
pub mod hotkeys;
pub mod signals;

use eyre::Result;
use std::thread;

use crate::drm_kms::{probe, types::CaptureOutput};

use self::{
    app::RecordingSession,
    cli::{Cli, Commands},
    hotkeys::{HotkeyAction, HotkeyBinding},
    signals::CaptureControl,
};

fn default_output_path(prefix: &str, extension: &str) -> std::path::PathBuf {
    let fmt = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    let filename = format!("{prefix}_{fmt}.{extension}");
    if let Some(mut dir) = dirs::video_dir() {
        dir.push(filename);
        dir
    } else {
        std::path::PathBuf::from(filename)
    }
}

fn add_default_replay_hotkey(capture: &mut cli::CaptureArgs) -> Result<()> {
    if capture.disable_hotkeys
        || capture
            .hotkeys
            .iter()
            .any(|binding| binding.action == HotkeyAction::SaveReplayBuffer)
    {
        return Ok(());
    }
    capture.hotkeys.push(
        "save-replay-buffer=Ctrl+Shift+S"
            .parse::<HotkeyBinding>()
            .map_err(eyre::Report::msg)?,
    );
    Ok(())
}

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
            let output_container = capture.output_container;
            let resolved_output = output.unwrap_or_else(|| {
                default_output_path("framepipe_record", output_container.extension())
            });
            let backend = capture.capture_backend;
            let options =
                config::build_capture_options(capture, CaptureOutput::File(resolved_output))?;
            let control = CaptureControl::register().map_err(|e| eyre::eyre!(e))?;
            RecordingSession::new(options, backend)?.run(control)
        }
        Commands::Replay {
            mut capture,
            seconds,
            output,
        } => {
            add_default_replay_hotkey(&mut capture)?;
            let output_container = capture.output_container;
            let resolved_output = output.unwrap_or_else(|| {
                default_output_path("framepipe_replay", output_container.extension())
            });
            let backend = capture.capture_backend;
            let options = config::build_capture_options(
                capture,
                CaptureOutput::ReplayBuffer(resolved_output, seconds.max(1)),
            )?;
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
                    crate::portal::pipewire::screencast_session(
                        1000,
                        &egl_ctx.egl,
                        egl_ctx.display,
                    )
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
