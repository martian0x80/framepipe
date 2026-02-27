use std::str::FromStr;
use std::time::Instant;

use gstreamer::prelude::*;
use gstreamer::{self as gst, glib};
use gstreamer_app as gst_app;

use crate::drm_kms::gstreamer::{ExportError, push_exported_dmabuf};
use crate::drm_kms::types::{BitrateMode, ColorRange, ExportedDmabuf, FrameRateMode};

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
    options: EncoderOptions,
    frame_ns: u64,
    next_pts_ns: u64,
    start: Instant,
}

pub enum EncoderOutput<'a> {
    File(&'a str),
    Preview,
}

#[derive(Debug, Clone)]
pub struct EncoderOptions {
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub frame_rate_mode: FrameRateMode,
    pub bitrate_mode: BitrateMode,
    pub color_range: ColorRange,
}

impl EncoderOptions {
    pub fn frame_ns(&self) -> u64 {
        1_000_000_000u64 / self.fps.max(1) as u64
    }
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

fn set_appsrc_caps(
    appsrc: &gst_app::AppSrc,
    ex: &ExportedDmabuf,
    opts: &EncoderOptions,
) -> Result<(), String> {
    let drm = fourcc_to_drm_format(ex.fourcc);
    let raw = fourcc_to_raw_format(ex.fourcc)
        .ok_or_else(|| format!("unsupported fourcc 0x{:08x}", ex.fourcc))?;
    let fps = opts.fps.max(1);
    let range = &opts.color_range.to_string();

    // Some drivers expose DMA_DRM AB24 only for specific non-linear modifiers.
    // If exporter gives linear modifier (0), prefer plain raw caps for compatibility.
    if ex.modifier == 0 {
        let raw_fallback = format!(
            "video/x-raw,format=(string){},width=(int){},height=(int){},framerate=(fraction){}/1,color-range=(string){}",
            raw, ex.width, ex.height, fps, range
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
            "video/x-raw(memory:DMABuf),format=(string)DMA_DRM,drm-format=(string){},width=(int){},height=(int){},framerate=(fraction){}/1,color-range=(string){}",
            drm_with_mod, ex.width, ex.height, fps, range
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
            "video/x-raw(memory:DMABuf),format=(string)DMA_DRM,drm-format=(string){},width=(int){},height=(int){},framerate=(fraction){}/1,color-range=(string){}",
            drm, ex.width, ex.height, fps, range
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
        "video/x-raw(memory:DMABuf),format=(string){},width=(int){},height=(int){},framerate=(fraction){}/1,color-range=(string){}",
        raw, ex.width, ex.height, fps, range
    );
    if let Ok(caps) = gst::Caps::from_str(&dmabuf_raw) {
        log::debug!("Using appsrc caps: {dmabuf_raw}");
        appsrc.set_caps(Some(&caps));
        return Ok(());
    }

    let raw_fallback = format!(
        "video/x-raw,format=(string){},width=(int){},height=(int){},framerate=(fraction){}/1,color-range=(string){}",
        raw, ex.width, ex.height, fps, range
    );
    let caps = gst::Caps::from_str(&raw_fallback)
        .map_err(|e| format!("fallback caps parse failed: {e}"))?;
    log::debug!("Using appsrc caps: {raw_fallback}");
    appsrc.set_caps(Some(&caps));
    Ok(())
}

impl GstEncoder {
    pub fn new_with_output(
        output: EncoderOutput<'_>,
        ex: &ExportedDmabuf,
        options: EncoderOptions,
    ) -> Result<Self, EncodeError> {
        gst::init()?;
        let fps = options.fps.max(1);
        let bitrate = options.bitrate_kbps.max(1);
        let rate_control = &options.bitrate_mode.to_string();

        let desc = match output {
            EncoderOutput::File(out_path) => format!(
                concat!(
                    "appsrc name=src is-live=true format=time do-timestamp=false block=true ",
                    "! queue ",
                    "! vapostproc ",
                    "! video/x-raw(memory:VAMemory),format=NV12 ",
                    "! vah264enc rate-control={rate_control} bitrate={bitrate} key-int-max={gop} ",
                    "! h264parse ",
                    "! mp4mux faststart=true ",
                    "! filesink location={out}"
                ),
                rate_control = rate_control,
                bitrate = bitrate,
                gop = fps * 2,
                out = out_path
            ),
            EncoderOutput::Preview => String::from(
                concat!(
                    "appsrc name=src is-live=true format=time do-timestamp=false block=true ",
                    "! queue ",
                    "! vapostproc ",
                    "! video/x-raw,format=BGRA ",
                    "! videoconvert ",
                    "! autovideosink sync=false"
                ),
            ),
        };
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

        set_appsrc_caps(&appsrc, ex, &options).map_err(EncodeError::Bus)?;

        appsrc.set_max_bytes(0); // unlimited buffering
        appsrc.set_block(true);

        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| EncodeError::Bus(format!("failed to set Playing: {e:?}")))?;

        Ok(Self {
            pipeline,
            appsrc,
            options,
            frame_ns: 1_000_000_000u64 / fps as u64,
            next_pts_ns: 0,
            start: Instant::now(),
        })
    }

    pub fn new(
        out_path: &str,
        ex: &ExportedDmabuf,
        options: EncoderOptions,
    ) -> Result<Self, EncodeError> {
        Self::new_with_output(EncoderOutput::File(out_path), ex, options)
    }

    pub fn push_frame(&mut self, ex: &ExportedDmabuf) -> Result<(), EncodeError> {
        let pts_ns = match self.options.frame_rate_mode {
            FrameRateMode::Cfr => self.next_pts_ns,
            FrameRateMode::Vfr => self.start.elapsed().as_nanos() as u64,
        };
        let duration_ns = match self.options.frame_rate_mode {
            FrameRateMode::Cfr => Some(self.frame_ns),
            FrameRateMode::Vfr => None,
        };

        push_exported_dmabuf(&self.appsrc, ex, pts_ns, duration_ns)?;
        self.next_pts_ns = pts_ns.saturating_add(self.frame_ns);
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
