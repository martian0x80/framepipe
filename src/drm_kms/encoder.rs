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

fn quality_bpp_floor(mode: &BitrateMode) -> f64 {
    // High-quality floors for desktop capture with H.264.
    // High resolutions/high FPS need far more bits/frame than camera footage.
    match mode {
        BitrateMode::Vbr => 0.12,
        BitrateMode::Cbr => 0.15,
        BitrateMode::Qvbr => 0.10,
    }
}

fn auto_bitrate_floor_kbps(width: i32, height: i32, fps: u32, mode: &BitrateMode) -> u32 {
    let pixels_per_sec = (width.max(1) as f64) * (height.max(1) as f64) * (fps.max(1) as f64);
    let bits_per_sec = pixels_per_sec * quality_bpp_floor(mode);
    // Keep sane lower bound for low-res streams.
    ((bits_per_sec / 1000.0).ceil() as u32).max(25_000)
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

fn get_rate_control(mode: &BitrateMode) -> u64 {
    match mode {
        BitrateMode::Vbr => 4, // VBR
        BitrateMode::Cbr => 2, // CBR
        BitrateMode::Qvbr => 1024, // QVBR (if supported by encoder)
    }
}

impl GstEncoder {
    pub fn new_with_output(
        output: EncoderOutput<'_>,
        ex: &ExportedDmabuf,
        options: EncoderOptions,
    ) -> Result<Self, EncodeError> {
        gst::init()?;
        let fps = options.fps.max(1);
        let auto_floor = auto_bitrate_floor_kbps(ex.width, ex.height, fps, &options.bitrate_mode);
        let requested = options.bitrate_kbps.max(1);
        let bitrate = requested.max(auto_floor);
        let rate_control = get_rate_control(&options.bitrate_mode);
        let enc_quality_props = match options.bitrate_mode {
            // High-quality constrained VBR tuned for desktop capture.
            BitrateMode::Vbr => "target-usage=1 target-percentage=95 min-qp=1 max-qp=35 qpi=18",
            // Tight CBR with bounded QP so quality does not collapse.
            BitrateMode::Cbr => "target-usage=1 min-qp=1 max-qp=30 qpi=20",
            // Let encoder choose defaults for QVBR, which should be good quality.
            BitrateMode::Qvbr => "",
        };
        if bitrate > requested {
            log::warn!(
                "Raising bitrate from {} to {} kbps to avoid quality collapse at {}x{}@{} ({:?})",
                requested,
                bitrate,
                ex.width,
                ex.height,
                fps,
                options.bitrate_mode
            );
        }

        let desc = match output {
            EncoderOutput::File(out_path) => format!(
                concat!(
                    "appsrc name=src is-live=true format=time do-timestamp=false block=true ",
                    "! queue ",
                    "! vapostproc ",
                    "! video/x-raw(memory:VAMemory),format=NV12,width={w},height={h},framerate={fps}/1 ",
                    "! vah264enc name=enc rate-control={rate_control} bitrate={bitrate} key-int-max={gop} b-frames=0 cabac=true dct8x8=true trellis=false cpb-size=0 {enc_quality_props} ",
                    "! h264parse ",
                    "! mp4mux faststart=true ",
                    "! filesink location={out}"
                ),
                rate_control = rate_control,
                bitrate = bitrate,
                gop = fps * 2,
                enc_quality_props = enc_quality_props,
                w = ex.width.max(1),
                h = ex.height.max(1),
                fps = fps,
                out = out_path
            ),
            EncoderOutput::Preview => format!(
                concat!(
                    "appsrc name=src is-live=true format=time do-timestamp=false block=true ",
                    "! queue ",
                    "! vapostproc ",
                    "! video/x-raw(memory:VAMemory),format=NV12,width={w},height={h},framerate={fps}/1 ",
                    "! vah264enc name=enc rate-control={rate_control} bitrate={bitrate} key-int-max={gop} b-frames=0 cabac=true dct8x8=true trellis=false cpb-size=0 {enc_quality_props} ",
                    "! h264parse ",
                    "! avdec_h264 ",
                    "! videoconvert ",
                    "! autovideosink sync=false"
                ),
                rate_control = rate_control,
                bitrate = bitrate,
                gop = fps * 2,
                enc_quality_props = enc_quality_props,
                w = ex.width.max(1),
                h = ex.height.max(1),
                fps = fps,
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

        appsrc.set_max_bytes(0);
        appsrc.set_property("max-buffers", 6u64);
        appsrc.set_property("max-time", 1_000_000_000u64);
        appsrc.set_block(true);

        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| EncodeError::Bus(format!("failed to set Playing: {e:?}")))?;

        if let Some(enc) = pipeline.by_name("enc") {
            let rate = enc.property_value("rate-control");
            let br = enc.property_value("bitrate");
            log::info!(
                "Encoder properties after start: rate-control={:?} bitrate={:?}",
                rate,
                br
            );
            if let Some(sink_pad) = enc.static_pad("sink") {
                let caps = sink_pad.current_caps();
                log::info!("Encoder sink negotiated caps: {:?}", caps);
            }
        }

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
        let elapsed_ns = self.start.elapsed().as_nanos() as u64;
        let pts_ns = match self.options.frame_rate_mode {
            // Keep CFR cadence, but never allow encoded timeline to run ahead of wall clock.
            // Without this, backpressure can make output play too fast.
            FrameRateMode::Cfr => self.next_pts_ns.max(elapsed_ns),
            FrameRateMode::Vfr => elapsed_ns,
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
