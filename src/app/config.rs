use std::path::PathBuf;

use eyre::Result;

use crate::{
    app::cli::PostfxArgs,
    drm_kms::{
        probe,
        types::{CaptureOptions, CaptureOutput},
    },
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

pub fn build_capture_options(
    capture: CaptureArgs,
    output: CaptureOutput,
) -> Result<CaptureOptions> {
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
        mouse_tracking: capture.mouse_tracking,
        cursor_composition: capture.cursor_composition,
        cursor_sprite: capture.cursor_sprite,
        cursor_hotspot_x: capture.cursor_hotspot_x,
        cursor_hotspot_y: capture.cursor_hotspot_y,
        cursor_scale: capture.cursor_scale,
        wayland_sync_frequency: capture.wayland_sync_frequency,
        mouse_tracking_file: PathBuf::from(capture.mouse_tracking_file),
    })
}

pub fn build_postfx_setup(
    input: PathBuf,
    output: PathBuf,
    options: PostfxArgs,
) -> Result<crate::postfx::renderer::PostFxSetup> {
    Ok(crate::postfx::renderer::PostFxSetup {
        input,
        output,
        mouse_track: options.mouse_track,
        mouse: crate::postfx::types::MouseEffectConfig {
            effect: options.effect,
            sprite_path: options.sprite,
            blend_mode: options.blend_mode,
            opacity: options.opacity,
            scale: options.scale,
            hotspot_x: options.hotspot_x,
            hotspot_y: options.hotspot_y,
            smoothing_ms: options.smoothing_ms,
            zoom_factor: options.zoom_factor,
            zoom_radius_px: options.zoom_radius_px,
            spotlight_radius_px: options.spotlight_radius_px,
            spotlight_softness: options.spotlight_softness,
        },
        transcode: crate::postfx::renderer::PostFxTranscodeConfig {
            decode_backend: options.decode_backend,
            decode_codec: options.decode_codec.clone(),
            encode: crate::encode::EncoderOptions {
                fps: options.fps,
                bitrate_kbps: options.bitrate_kbps,
                frame_rate_mode: options.frame_rate_mode,
                bitrate_mode: options.bitrate_mode,
                quality: options.quality,
                color_range: options.color_range,
                colorimetry: options.colorimetry,
                encoder_backend: options.encoder_backend,
                video_codec: options.video_codec,
            },
        },
    })
}
