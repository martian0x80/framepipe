use env_logger;

mod drm_kms;


#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::builder().filter_level(log::LevelFilter::Debug).init();

    drm_kms::probe::probe()?;

    Ok(())
}
