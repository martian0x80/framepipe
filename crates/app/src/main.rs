use clap::Parser;
use eyre::Result;

fn main() -> Result<()> {
    env_logger::builder()
        .format_timestamp_nanos()
        .filter_level(log::LevelFilter::Trace)
        .filter_module("zbus", log::LevelFilter::Info)
        .init();
    framepipe::run_cli(framepipe::app::cli::Cli::parse())
}
