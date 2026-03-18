use eyre::Result;
use clap::Parser;

mod wayland;
mod drm_kms;
mod encode;
mod app;

fn main() -> Result<()> {
    env_logger::builder().format_timestamp_nanos().filter_level(log::LevelFilter::Debug).init();
    app::run(app::cli::Cli::parse())
}
