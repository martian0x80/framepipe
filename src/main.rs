use env_logger;
// rust's default termination behavior is to print the error with `Debug` instead of `Display`
use eyre::Result;

use crate::drm_kms::probe;

mod drm_kms;


#[tokio::main]
async fn main() -> Result<()>  {
    env_logger::builder().filter_level(log::LevelFilter::Debug).init();

    let card_path = probe::get_dri_cards()?;
    drm_kms::egl::egl_main(card_path.last().unwrap().as_str())?;

    Ok(())
}
