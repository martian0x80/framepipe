use std::str::FromStr;

use gstreamer::prelude::*;
use gstreamer::{self as gst, glib};
use gstreamer_app as gst_app;

use crate::drm_kms::gstreamer_export::{ExportError, push_exported_dmabuf};
use crate::drm_kms::types::ExportedDmabuf;

#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    #[error("gst init failed: {0}")]
    Init(#[from] glib::Error),
    #[error("pipeline parse failed: {0}")]
    Parse(#[from] glib::BoolError),
    #[error("missing appsrc")]
    MissingAppSrc,
    #[error("push failed: {0}")]
    Push(#[from] ExportError),
    #[error("bus error: {0}")]
    Bus(String),
}

pub struct GstEncoder {
    pipeline: gst::Pipeline,
    appsrc: gst_app::AppSrc,
    frame_ns: u64,
    next_pts_ns: u64,
}

fn fourcc_to_drm_format(fourcc: u32) -> Option<&'static str> {
    match fourcc {
        0x34324241 => Some("AB24"), // DRM_FORMAT_ABGR8888
        0x34324258 => Some("XB24"), // DRM_FORMAT_XBGR8888
        0x34325258 => Some("XR24"), // DRM_FORMAT_XRGB8888
        0x34325241 => Some("AR24"), // DRM_FORMAT_ARGB8888
        0x3231564e => Some("NV12"), // DRM_FORMAT_NV12
        _ => None,
    }
}

fn fourcc_to_raw_format(fourcc: u32) -> Option<&'static str> {
    match fourcc {
        0x34324241 => Some("RGBA"), // DRM_FORMAT_ABGR8888 (LE memory: RGBA)
        0x34324258 => Some("RGBx"), // DRM_FORMAT_XBGR8888
        0x34325241 => Some("BGRA"), // DRM_FORMAT_ARGB8888
        0x34325258 => Some("BGRx"), // DRM_FORMAT_XRGB8888
        0x3231564e => Some("NV12"), // DRM_FORMAT_NV12
        _ => None,
    }
}

fn set_appsrc_caps(appsrc: &gst_app::AppSrc, ex: &ExportedDmabuf, fps: u32) -> Result<(), String> {
    let drm = fourcc_to_drm_format(ex.fourcc);
    let raw = fourcc_to_raw_format(ex.fourcc)
        .ok_or_else(|| format!("unsupported fourcc 0x{:08x}", ex.fourcc))?;

    // Some drivers expose DMA_DRM AB24 only for specific non-linear modifiers.
    // If exporter gives linear modifier (0), prefer plain raw caps for compatibility.
    if ex.modifier == 0 {
        let raw_fallback = format!(
            "video/x-raw,format=(string){},width=(int){},height=(int){},framerate=(fraction){}/1",
            raw, ex.width, ex.height, fps
        );
        if let Ok(caps) = gst::Caps::from_str(&raw_fallback) {
            log::debug!("Using appsrc caps (linear modifier fallback): {raw_fallback}");
            appsrc.set_caps(Some(&caps));
            return Ok(());
        }
    }

    if let Some(drm) = drm {
        let drm_with_mod = format!("{drm}:0x{:016x}", ex.modifier);
        let full_with_mod = format!(
            "video/x-raw(memory:DMABuf),format=(string)DMA_DRM,drm-format=(string){},width=(int){},height=(int){},framerate=(fraction){}/1",
            drm_with_mod, ex.width, ex.height, fps
        );
        match gst::Caps::from_str(&full_with_mod) {
            Ok(caps) => {
                log::debug!("Using appsrc caps: {full_with_mod}");
                appsrc.set_caps(Some(&caps));
                return Ok(());
            }
            Err(e) => {
                log::warn!("DMA_DRM+modifier caps parse failed: {e}");
            }
        }

        let full = format!(
            "video/x-raw(memory:DMABuf),format=(string)DMA_DRM,drm-format=(string){},width=(int){},height=(int){},framerate=(fraction){}/1",
            drm, ex.width, ex.height, fps
        );
        match gst::Caps::from_str(&full) {
            Ok(caps) => {
                log::debug!("Using appsrc caps: {full}");
                appsrc.set_caps(Some(&caps));
                return Ok(());
            }
            Err(e) => {
                log::warn!("DMA_DRM caps parse failed: {e}");
            }
        }
    }

    let dmabuf_raw = format!(
        "video/x-raw(memory:DMABuf),format=(string){},width=(int){},height=(int){},framerate=(fraction){}/1",
        raw, ex.width, ex.height, fps
    );
    if let Ok(caps) = gst::Caps::from_str(&dmabuf_raw) {
        log::debug!("Using appsrc caps: {dmabuf_raw}");
        appsrc.set_caps(Some(&caps));
        return Ok(());
    }

    let raw_fallback = format!(
        "video/x-raw,format=(string){},width=(int){},height=(int){},framerate=(fraction){}/1",
        raw, ex.width, ex.height, fps
    );
    let caps = gst::Caps::from_str(&raw_fallback)
        .map_err(|e| format!("fallback caps parse failed: {e}"))?;
    log::debug!("Using appsrc caps: {raw_fallback}");
    appsrc.set_caps(Some(&caps));
    Ok(())
}

impl GstEncoder {
    pub fn new(out_path: &str, ex: &ExportedDmabuf, fps: u32) -> Result<Self, EncodeError> {
        gst::init()?;

        let desc = format!(
            concat!(
                "appsrc name=src is-live=true format=time do-timestamp=false block=true ",
                "! queue ",
                "! vapostproc ",
                "! video/x-raw(memory:VAMemory),format=NV12 ",
                "! vah264enc rate-control=cbr bitrate=12000 key-int-max=120 ",
                "! h264parse ",
                "! mp4mux faststart=true ",
                "! filesink location={out}"
            ),
            out = out_path
        );
        log::debug!("GStreamer pipeline: {desc}");

        let element = gst::parse::launch(&desc)?;
        let pipeline = element
            .downcast::<gst::Pipeline>()
            .map_err(|_| glib::bool_error!("parsed element is not a pipeline"))?;

        let appsrc = pipeline
            .by_name("src")
            .ok_or(EncodeError::MissingAppSrc)?
            .downcast::<gst_app::AppSrc>()
            .map_err(|_| EncodeError::MissingAppSrc)?;

        set_appsrc_caps(&appsrc, ex, fps).map_err(|e| EncodeError::Bus(e))?;

        appsrc.set_max_bytes(0); // unlimited buffering
        appsrc.set_block(true);

        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| EncodeError::Bus(format!("failed to set Playing: {e:?}")))?;

        Ok(Self {
            pipeline,
            appsrc,
            frame_ns: 1_000_000_000u64 / fps as u64,
            next_pts_ns: 0,
        })
    }

    pub fn push_frame(&mut self, ex: &ExportedDmabuf) -> Result<(), EncodeError> {
        push_exported_dmabuf(&self.appsrc, ex, self.next_pts_ns)?;
        self.next_pts_ns += self.frame_ns;
        Ok(())
    }

    pub fn finish(self) -> Result<(), EncodeError> {
        self.appsrc
            .end_of_stream()
            .map_err(|e| EncodeError::Bus(format!("eos failed: {e:?}")))?;

        let bus = self
            .pipeline
            .bus()
            .ok_or_else(|| EncodeError::Bus("no bus".into()))?;

        loop {
            match bus.timed_pop(gst::ClockTime::from_seconds(10)) {
                Some(msg) => match msg.view() {
                    gst::MessageView::Eos(..) => break,
                    gst::MessageView::Error(err) => {
                        return Err(EncodeError::Bus(format!(
                            "{} ({:?})",
                            err.error(),
                            err.debug()
                        )));
                    }
                    _ => {}
                },
                None => return Err(EncodeError::Bus("timeout waiting for EOS".into())),
            }
        }

        self.pipeline
            .set_state(gst::State::Null)
            .map_err(|e| EncodeError::Bus(format!("failed to set Null: {e:?}")))?;
        Ok(())
    }
}

impl Drop for GstEncoder {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}
