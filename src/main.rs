use eyre::Result;
use clap::Parser;

mod app;
mod drm_kms;
mod encode;
mod postfx;
mod shared;
mod wayland;

fn main() -> Result<()> {
    env_logger::builder().format_timestamp_nanos().filter_level(log::LevelFilter::Debug).init();
    app::run(app::cli::Cli::parse())
}
