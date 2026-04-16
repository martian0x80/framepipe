pub mod app;
pub mod capture;
pub mod cursor;
pub mod drm_kms;
pub mod embedded_preview;
pub mod encode;
pub mod portal;
pub mod shared;
pub mod wayland;

use eyre::Result;

pub fn run_cli(cli: app::cli::Cli) -> Result<()> {
    app::run(cli)
}
