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
        privilege_mode: capture.privilege_mode,
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
        output_container: capture.output_container,
        bitrate_kbps: capture.bitrate_kbps,
        frame_rate_mode: capture.frame_rate_mode,
        bitrate_mode: capture.bitrate_mode,
        quality: capture.quality,
        color_range: capture.color_range,
        colorimetry: capture.colorimetry,
        encoder_backend: capture.encoder_backend,
        video_codec: capture.video_codec,
        cursor_composition: capture.cursor_composition,
        hotkeys: if capture.disable_hotkeys {
            Vec::new()
        } else {
            capture.hotkeys
        },
        keyboard_overlay: capture.keyboard_overlay,
        keyboard_overlay_duration_ms: capture.keyboard_overlay_duration_ms.max(1),
        keyboard_overlay_fade_ms: capture.keyboard_overlay_fade_ms.max(1),
        keyboard_overlay_debounce_ms: capture.keyboard_overlay_debounce_ms,
        keyboard_overlay_show_single_modifiers: capture.keyboard_overlay_show_single_modifiers,
        keyboard_overlay_font: capture.keyboard_overlay_font,
        cursor_sprite: capture.cursor_sprite,
        cursor_hotspot_x: capture.cursor_hotspot_x,
        cursor_hotspot_y: capture.cursor_hotspot_y,
        cursor_scale: capture.cursor_scale,
        cursor_smooth: capture.cursor_smooth,
        cursor_smear: capture.cursor_smear,
        cursor_spring_k: capture.cursor_spring_k,
        cursor_spring_d: capture.cursor_spring_d,
        cursor_max_speed: capture.cursor_max_speed,
        cursor_snap_px: capture.cursor_snap_px,
        cursor_smooth_ms: capture.cursor_smooth_ms,
        cursor_deadzone_px: capture.cursor_deadzone_px,
        cursor_smear_speed_threshold: capture.cursor_smear_speed_threshold,
        cursor_smear_shutter_scale: capture.cursor_smear_shutter_scale,
        cursor_smear_min_len: capture.cursor_smear_min_len,
        cursor_smear_max_len: capture.cursor_smear_max_len,
        cursor_smear_taps: capture.cursor_smear_taps,
        cursor_smear_alpha_exp: capture.cursor_smear_alpha_exp,
        cursor_smear_alpha_scale: capture.cursor_smear_alpha_scale,
        cursor_smear_stretch_threshold: capture.cursor_smear_stretch_threshold,
        cursor_smear_stretch_range: capture.cursor_smear_stretch_range,
        cursor_smear_max_stretch: capture.cursor_smear_max_stretch,
        cursor_smear_max_squash: capture.cursor_smear_max_squash,
        wayland_sync_frequency: capture.wayland_sync_frequency,
        profile: capture.profile,
        background: capture.background,
        background_zoom: capture.background_zoom.clamp(1.0, 100.0),
        preview_mailbox: None,
        live_settings: None,
    })
}
