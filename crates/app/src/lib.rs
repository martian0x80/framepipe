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
use std::sync::Once;

/// Initialize process-global logging once.
///
/// `default_filter` is used when `RUST_LOG` is not set.
pub fn init_logging(default_filter: &str) {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let mut builder = env_logger::Builder::from_env(
            env_logger::Env::default().default_filter_or(default_filter),
        );
        builder
            .format_timestamp_millis()
            .filter_module("zbus", log::LevelFilter::Info);
        let _ = builder.try_init();
    });
}

pub fn run_cli(cli: app::cli::Cli) -> Result<()> {
    app::run(cli)
}
