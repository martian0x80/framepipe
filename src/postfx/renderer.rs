use std::path::PathBuf;

use crate::postfx::mouse::load_mouse_track;
use crate::postfx::types::MouseEffectConfig;
use crate::wayland::types::MouseTrackFile;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;

#[derive(Debug, Clone)]
pub struct PostFxSetup {
    pub input: PathBuf,
    pub output: PathBuf,
    pub mouse_track: PathBuf,
    pub mouse: MouseEffectConfig,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum PostFxError {
    #[error("GStreamer init error: {0}")]
    Init(#[from] gst::glib::Error),
    #[error("GStreamer parse error: {0}")]
    Parse(#[from] gst::glib::BoolError),
    #[error("GStreamer state change error")]
    StateChange,
    #[error("Unsupported media format")]
    UnsupportedFormat,
    #[error("Other error: {0}")]
    Other(String),
}

pub fn decode_loop(path: &str, mouse_track: &MouseTrackFile) -> Result<(), PostFxError> {
    gst::init()?;
    let desc = format!(
        "filesrc location={} ! decodebin ! videoconvert ! video/x-raw,format=RGBA ! appsink name=out sync=false",
        path
    );
    let element = gst::parse::launch(&desc)?;
    let pipeline = element
        .downcast::<gst::Pipeline>()
        .map_err(|_| gst::glib::bool_error!("Failed to downcast to Pipeline"))?;
    let appsink = pipeline
        .by_name("out")
        .ok_or_else(|| gst::glib::bool_error!("Failed to get appsink element"))?
        .downcast::<gst_app::AppSink>()
        .map_err(|_| gst::glib::bool_error!("Failed to downcast to AppSink"))?;
    pipeline
        .set_state(gst::State::Playing)
        .map_err(|_| PostFxError::StateChange)?;
    loop {
        match appsink.try_pull_sample(None) {
            Some(sample) => {
                let buffer = sample
                    .buffer()
                    .ok_or_else(|| gst::glib::bool_error!("Failed to get buffer from sample"))?;
                let map = buffer
                    .map_readable()
                    .map_err(|_| gst::glib::bool_error!("Failed to map buffer"))?;
                let pts_ns = buffer.pts().unwrap_or(gst::ClockTime::ZERO).nseconds();
                log::info!(
                    "[Postfx] Got frame with PTS: {} ns, size: {} bytes",
                    pts_ns,
                    map.size()
                );
            }
            None => {
                if appsink.is_eos() {
                    log::info!("[Postfx] End of stream");
                } else {
                    log::error!("[Postfx] Failed to pull sample from appsink");
                }
                break;
            }
        }
    }
    let _ = pipeline.set_state(gst::State::Null);
    Ok(())
}

pub fn apply_postfx(setup: PostFxSetup) -> eyre::Result<()> {
    let track =
        load_mouse_track(&setup.mouse_track).map_err(|e| eyre::eyre!("failed to load mouse track: {e}"))?;
    log::info!(
        "Applying postfx with setup: input={} output={} samples={} effect={:?}",
        setup.input.display(),
        setup.output.display(),
        track.samples.len(),
        setup.mouse.effect
    );
    decode_loop(&setup.input.to_string_lossy(), &track).map_err(|e| eyre::eyre!(e))
}
