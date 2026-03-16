use eyre::Result;

use crate::drm_kms::{
    probe,
    types::{CaptureOptions, CaptureOutput},
};

use super::cli::CaptureArgs;

pub fn resolve_card_path(card: Option<String>) -> Result<String> {
    if let Some(card) = card {
        return Ok(card);
    }
    let cards = probe::get_dri_cards()?;
    cards
        .last()
        .cloned()
        .ok_or_else(|| eyre::eyre!("no DRM cards found"))
}

pub fn build_capture_options(capture: CaptureArgs, output: CaptureOutput) -> Result<CaptureOptions> {
    if capture.output_width.is_some() != capture.output_height.is_some() {
        return Err(eyre::eyre!(
            "both --output-width and --output-height must be set together"
        ));
    }

    let card_path = resolve_card_path(capture.card)?;
    Ok(CaptureOptions {
        card_path,
        connector: capture.connector,
        allow_fallback_connector: capture.allow_fallback_connector,
        fps: capture.fps,
        output_width: capture.output_width,
        output_height: capture.output_height,
        dump_frames: capture.dump_frames,
        dump_dir: capture.dump_dir,
        dump_every: capture.dump_every,
        output,
        bitrate_kbps: capture.bitrate_kbps,
        frame_rate_mode: capture.frame_rate_mode,
        bitrate_mode: capture.bitrate_mode,
        quality: capture.quality,
        color_range: capture.color_range,
        colorimetry: capture.colorimetry,
        encoder_backend: capture.encoder_backend,
        video_codec: capture.video_codec,
    })
}
