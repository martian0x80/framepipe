use crate::drm_kms::types::Card;

#[derive(Debug, thiserror::Error)]
pub(crate) enum DrmInitError {
    #[error("Failed to open DRM device: {0}")]
    OpenDevice(#[source] std::io::Error),
}

pub(crate) fn init_drm_device(card_path: &str) -> Result<Card, DrmInitError> {
    let card = Card::open(card_path).map_err(DrmInitError::OpenDevice)?;
    Ok(card)
}