use env_logger;
// rust's default termination behavior is to print the error with `Debug` instead of `Display`
use eyre::Result;

mod drm_kms;


#[tokio::main]
async fn main() -> Result<()>  {
    env_logger::builder().filter_level(log::LevelFilter::Debug).init();

    drm_kms::egl::egl_main()?;

    Ok(())
}
