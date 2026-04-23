use clap::Parser;
use eyre::Result;

fn main() -> Result<()> {
    framepipe::init_logging("debug");
    framepipe::run_cli(framepipe::app::cli::Cli::parse())
}
