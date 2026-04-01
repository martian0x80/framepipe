use std::collections::HashSet;
use std::str::FromStr;
use std::thread;
use std::time::Instant;

use gstreamer::prelude::*;
use gstreamer::{self as gst, glib};
use gstreamer_app as gst_app;
use gstreamer_video::DownstreamForceKeyUnitEvent;

use crate::drm_kms::gstreamer::{ExportError, push_exported_dmabuf};
use crate::drm_kms::types::{
    BitrateMode, ColorRange, Colorimetry, EncoderBackend, ExportedDmabuf, FrameRateMode,
    QualityPreset, VideoCodec,
};

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
    pub quality: QualityPreset,
    pub color_range: ColorRange,
    pub colorimetry: Colorimetry,
    pub encoder_backend: EncoderBackend,
    pub video_codec: VideoCodec,
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
    match mode {
        BitrateMode::Vbr => 0.12,
        BitrateMode::Cbr => 0.15,
        BitrateMode::Qvbr => 0.10,
        BitrateMode::Vcm => 0.12,
        BitrateMode::Default
        | BitrateMode::Cqp
        | BitrateMode::Icq
        | BitrateMode::Quant
        | BitrateMode::Qual
        | BitrateMode::Pass1
        | BitrateMode::Pass2
        | BitrateMode::Pass3 => 0.0,
    }
}

fn auto_bitrate_floor_kbps(width: i32, height: i32, fps: u32, mode: &BitrateMode) -> u32 {
    if quality_bpp_floor(mode) <= 0.0 {
        return 0;
    }
    let pixels_per_sec = (width.max(1) as f64) * (height.max(1) as f64) * (fps.max(1) as f64);
    let bits_per_sec = pixels_per_sec * quality_bpp_floor(mode);
    ((bits_per_sec / 1000.0).ceil() as u32).max(25_000)
}

pub fn recommended_slots(options: &EncoderOptions) -> usize {
    match options.encoder_backend {
        EncoderBackend::Cpu => 4usize,
        EncoderBackend::Vaapi | EncoderBackend::Qsv => match options.video_codec {
            VideoCodec::Av1 => 16usize,
            VideoCodec::H265 => 12usize,
            VideoCodec::H264 => 8usize,
        },
        EncoderBackend::Vulkan => 8usize,
    }
}

fn codec_elements(codec: &VideoCodec) -> (&'static str, &'static str) {
    match codec {
        VideoCodec::H264 => ("h264parse", "avdec_h264"),
        VideoCodec::H265 => ("h265parse", "avdec_h265"),
        VideoCodec::Av1 => ("av1parse", "avdec_av1"),
    }
}

fn vaapi_encoder_name(codec: &VideoCodec) -> &'static str {
    match codec {
        VideoCodec::H264 => "vah264enc",
        VideoCodec::H265 => "vah265enc",
        VideoCodec::Av1 => "vaav1enc",
    }
}

fn vulkan_encoder_name(codec: &VideoCodec) -> &'static str {
    match codec {
        VideoCodec::H264 => "vulkanh264enc",
        VideoCodec::H265 => "vulkanh265enc",
        VideoCodec::Av1 => "vulkanav1enc",
    }
}

fn cpu_encoder_name(codec: &VideoCodec) -> &'static str {
    match codec {
        VideoCodec::H264 => "x264enc",
        VideoCodec::H265 => "x265enc",
        VideoCodec::Av1 => "av1enc",
    }
}

fn qsv_encoder_name(codec: &VideoCodec) -> &'static str {
    match codec {
        VideoCodec::H264 => "qsvh264enc",
        VideoCodec::H265 => "qsvh265enc",
        VideoCodec::Av1 => "qsvav1enc",
    }
}

fn encoder_supported_props(factory_name: &str) -> Option<HashSet<String>> {
    let factory = gst::ElementFactory::find(factory_name)?;
    let elem = factory.create().build().ok()?;
    Some(
        elem.list_properties()
            .into_iter()
            .map(|p| p.name().to_string())
            .collect(),
    )
}

fn render_encoder_props(factory_name: &str, props: Vec<(&'static str, String)>) -> String {
    let Some(supported) = encoder_supported_props(factory_name) else {
        return props
            .into_iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" ");
    };

    props
        .into_iter()
        .filter_map(|(k, v)| {
            if supported.contains(k) {
                Some(format!("{k}={v}"))
            } else {
                log::debug!(
                    "dropping unsupported property '{}' for encoder '{}'",
                    k,
                    factory_name
                );
                None
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_codec_supported(backend: &EncoderBackend, codec: &VideoCodec) -> bool {
    match (backend, codec) {
        (EncoderBackend::Vaapi, VideoCodec::H264 | VideoCodec::H265 | VideoCodec::Av1) => true,
        (EncoderBackend::Qsv, VideoCodec::H264 | VideoCodec::H265 | VideoCodec::Av1) => true,
        (EncoderBackend::Vulkan, VideoCodec::H264 | VideoCodec::H265 | VideoCodec::Av1) => false, // never tested
        (EncoderBackend::Cpu, VideoCodec::H264 | VideoCodec::H265 | VideoCodec::Av1) => true,
        _ => false,
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
    let colorimetry = opts.colorimetry.to_string();

    // Some drivers expose DMA_DRM AB24 only for specific non-linear modifiers.
    // If exporter gives linear modifier (0), prefer plain raw caps for compatibility.
    if ex.modifier == 0 {
        let raw_fallback = format!(
            "video/x-raw,format=(string){},width=(int){},height=(int){},framerate=(fraction){}/1,color-range=(string){},colorimetry=(string){}",
            raw,
            ex.width,
            ex.height,
            fps,
            range,
            colorimetry.as_str()
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
            "video/x-raw(memory:DMABuf),format=(string)DMA_DRM,drm-format=(string){},width=(int){},height=(int){},framerate=(fraction){}/1,color-range=(string){},colorimetry=(string){}",
            drm_with_mod,
            ex.width,
            ex.height,
            fps,
            range,
            colorimetry.as_str()
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
            "video/x-raw(memory:DMABuf),format=(string)DMA_DRM,drm-format=(string){},width=(int){},height=(int){},framerate=(fraction){}/1,color-range=(string){},colorimetry=(string){}",
            drm,
            ex.width,
            ex.height,
            fps,
            range,
            colorimetry.as_str()
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
        "video/x-raw(memory:DMABuf),format=(string){},width=(int){},height=(int){},framerate=(fraction){}/1,color-range=(string){},colorimetry=(string){}",
        raw,
        ex.width,
        ex.height,
        fps,
        range,
        colorimetry.as_str()
    );
    if let Ok(caps) = gst::Caps::from_str(&dmabuf_raw) {
        log::debug!("Using appsrc caps: {dmabuf_raw}");
        appsrc.set_caps(Some(&caps));
        return Ok(());
    }

    let raw_fallback = format!(
        "video/x-raw,format=(string){},width=(int){},height=(int){},framerate=(fraction){}/1,color-range=(string){},colorimetry=(string){}",
        raw,
        ex.width,
        ex.height,
        fps,
        range,
        colorimetry.as_str()
    );
    let caps = gst::Caps::from_str(&raw_fallback)
        .map_err(|e| format!("fallback caps parse failed: {e}"))?;
    log::debug!("Using appsrc caps: {raw_fallback}");
    appsrc.set_caps(Some(&caps));
    Ok(())
}

fn vaapi_rate_control(mode: &BitrateMode, codec: &VideoCodec) -> Result<&'static str, EncodeError> {
    match (codec, mode) {
        (VideoCodec::H264, BitrateMode::Cbr)
        | (VideoCodec::H265, BitrateMode::Cbr)
        | (VideoCodec::Av1, BitrateMode::Cbr) => Ok("cbr"),
        (VideoCodec::H264, BitrateMode::Vbr)
        | (VideoCodec::H265, BitrateMode::Vbr)
        | (VideoCodec::Av1, BitrateMode::Vbr) => Ok("vbr"),
        (VideoCodec::H264, BitrateMode::Cqp)
        | (VideoCodec::H265, BitrateMode::Cqp)
        | (VideoCodec::Av1, BitrateMode::Cqp) => Ok("cqp"),
        (VideoCodec::H264, BitrateMode::Vcm) | (VideoCodec::H265, BitrateMode::Vcm) => Ok("vcm"),
        (VideoCodec::H264, BitrateMode::Icq)
        | (VideoCodec::H265, BitrateMode::Icq)
        | (VideoCodec::Av1, BitrateMode::Icq) => Ok("icq"),
        (VideoCodec::H264, BitrateMode::Qvbr) | (VideoCodec::H265, BitrateMode::Qvbr) => Ok("qvbr"),
        (VideoCodec::H264, BitrateMode::Default) | (VideoCodec::H265, BitrateMode::Default) => {
            Ok("icq")
        }
        (VideoCodec::Av1, BitrateMode::Default) => Ok("icq"),
        _ => Err(EncodeError::Bus(format!(
            "rate-control {:?} is not supported for vaapi {:?}",
            mode, codec
        ))),
    }
}

fn vulkan_rate_control(mode: &BitrateMode) -> Result<&'static str, EncodeError> {
    match mode {
        BitrateMode::Default => Ok("cqp"),
        BitrateMode::Cqp => Ok("cqp"),
        BitrateMode::Cbr => Ok("cbr"),
        BitrateMode::Vbr | BitrateMode::Qvbr => Ok("vbr"),
        _ => Err(EncodeError::Bus(format!(
            "rate-control {:?} is not supported for vulkan backend",
            mode
        ))),
    }
}

fn cpu_rate_control(mode: &BitrateMode) -> Result<&'static str, EncodeError> {
    match mode {
        BitrateMode::Cbr => Ok("cbr"),
        BitrateMode::Quant | BitrateMode::Cqp => Ok("quant"),
        BitrateMode::Qual | BitrateMode::Icq | BitrateMode::Vbr | BitrateMode::Qvbr => Ok("qual"),
        BitrateMode::Pass1 => Ok("pass1"),
        BitrateMode::Pass2 => Ok("pass2"),
        BitrateMode::Pass3 => Ok("pass3"),
        BitrateMode::Default => Ok("qual"),
        _ => Err(EncodeError::Bus(format!(
            "rate-control {:?} is not supported for x264 backend",
            mode
        ))),
    }
}

fn qsv_rate_control(mode: &BitrateMode, codec: &VideoCodec) -> Result<&'static str, EncodeError> {
    match (codec, mode) {
        (VideoCodec::H264, BitrateMode::Cbr)
        | (VideoCodec::H265, BitrateMode::Cbr)
        | (VideoCodec::Av1, BitrateMode::Cbr) => Ok("cbr"),
        (VideoCodec::H264, BitrateMode::Vbr)
        | (VideoCodec::H265, BitrateMode::Vbr)
        | (VideoCodec::Av1, BitrateMode::Vbr) => Ok("vbr"),
        (VideoCodec::H264, BitrateMode::Cqp)
        | (VideoCodec::H265, BitrateMode::Cqp)
        | (VideoCodec::Av1, BitrateMode::Cqp) => Ok("cqp"),
        (VideoCodec::H264, BitrateMode::Icq) | (VideoCodec::H265, BitrateMode::Icq) => Ok("icq"),
        (VideoCodec::H264, BitrateMode::Qvbr) | (VideoCodec::H265, BitrateMode::Qvbr) => Ok("qvbr"),
        (VideoCodec::H264, BitrateMode::Vcm) | (VideoCodec::H265, BitrateMode::Vcm) => Ok("vcm"),
        (_, BitrateMode::Default) => Ok("cqp"),
        _ => Err(EncodeError::Bus(format!(
            "rate-control {:?} is not supported for qsv {:?}",
            mode, codec
        ))),
    }
}

#[derive(Clone, Copy)]
struct QualityTuning {
    qpi: u32,
    qpp: u32,
    qpb: u32,
    min_qp: u32,
    max_qp: u32,
    i_frames: u32,
    b_frames: u32,
    target_usage: u32,
    icq_quality: u32,
    qvbr_quality: u32,
}

fn quality_tuning(preset: &QualityPreset) -> QualityTuning {
    match preset {
        QualityPreset::Low => QualityTuning {
            qpi: 33,
            qpp: 35,
            qpb: 0,
            min_qp: 28,
            max_qp: 51,
            i_frames: 180,
            b_frames: 0,
            target_usage: 4,
            icq_quality: 35,
            qvbr_quality: 35,
        },
        QualityPreset::Medium => QualityTuning {
            qpi: 28,
            qpp: 32,
            qpb: 0,
            min_qp: 28,
            max_qp: 36,
            i_frames: 90,
            b_frames: 0,
            target_usage: 4,
            icq_quality: 28,
            qvbr_quality: 28,
        },
        QualityPreset::High => QualityTuning {
            qpi: 20,
            qpp: 24,
            qpb: 0,
            min_qp: 16,
            max_qp: 32,
            i_frames: 30,
            b_frames: 0,
            target_usage: 3,
            icq_quality: 7,
            qvbr_quality: 7,
        },
        QualityPreset::Ultra => QualityTuning {
            qpi: 1,
            qpp: 1,
            qpb: 0,
            min_qp: 1,
            max_qp: 5,
            // gpu struggles :(
            i_frames: 15,
            b_frames: 0,
            target_usage: 1,
            icq_quality: 1,
            qvbr_quality: 1,
        },
    }
}

fn quality_tuning_for(
    _backend: &EncoderBackend,
    _codec: &VideoCodec,
    _mode: &BitrateMode,
    preset: &QualityPreset,
) -> QualityTuning {
    // Keep this resolver entrypoint so backend/codec/mode-specific tuning can be
    // reintroduced without touching call sites.
    quality_tuning(preset)
}

fn default_rate_control_for(backend: &EncoderBackend, codec: &VideoCodec) -> BitrateMode {
    match (backend, codec) {
        (EncoderBackend::Vaapi, VideoCodec::H264 | VideoCodec::H265) => BitrateMode::Icq,
        (EncoderBackend::Vaapi, VideoCodec::Av1) => BitrateMode::Icq,
        (EncoderBackend::Qsv, VideoCodec::H264 | VideoCodec::H265) => BitrateMode::Icq,
        (EncoderBackend::Vulkan, _) => BitrateMode::Cqp,
        (EncoderBackend::Cpu, _) => BitrateMode::Qual,
        (EncoderBackend::Qsv, VideoCodec::Av1) => BitrateMode::Cqp,
    }
}

impl GstEncoder {
    pub fn new_with_output(
        output: EncoderOutput<'_>,
        ex: &ExportedDmabuf,
        mut options: EncoderOptions,
    ) -> Result<Self, EncodeError> {
        gst::init()?;
        if options.encoder_backend == EncoderBackend::Vulkan {
            return Err(EncodeError::Bus(
                "vulkan encoder backend is temporarily disabled".to_string(),
            ));
        }
        if !is_codec_supported(&options.encoder_backend, &options.video_codec) {
            return Err(EncodeError::Bus(format!(
                "backend {:?} does not support codec {:?}",
                options.encoder_backend, options.video_codec
            )));
        }
        if options.bitrate_mode == BitrateMode::Default {
            let selected = default_rate_control_for(&options.encoder_backend, &options.video_codec);
            log::info!(
                "rate-control=default resolved to {:?} for backend={:?} codec={:?}",
                selected,
                options.encoder_backend,
                options.video_codec
            );
            options.bitrate_mode = selected;
        }
        let fps = options.fps.max(1);
        let auto_floor = auto_bitrate_floor_kbps(ex.width, ex.height, fps, &options.bitrate_mode);
        let requested = options.bitrate_kbps.max(1);
        let bitrate = requested.max(auto_floor).max(1);
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
        let colorimetry = options.colorimetry.to_string();
        let tuning = quality_tuning_for(
            &options.encoder_backend,
            &options.video_codec,
            &options.bitrate_mode,
            &options.quality,
        );

        let w = ex.width.max(1);
        let h = ex.height.max(1);
        if w > 1920 || h > 1200 {
            if options.encoder_backend == EncoderBackend::Cpu {
                log::warn!(
                    "Resolution {}x{} may be too large for cpu to handle efficiently; consider using vaapi, qsv or vulkan backend for better performance",
                    w,
                    h
                );
            }
            if options.video_codec == VideoCodec::H264
                && options.encoder_backend == EncoderBackend::Vaapi
                && (options.bitrate_mode == BitrateMode::Cbr
                    || options.bitrate_mode == BitrateMode::Vbr
                    || options.bitrate_mode == BitrateMode::Qvbr)
            {
                log::warn!(
                    "vah264enc may have poor quality at high fps on resolutions above 1920x1200 with CBR/VBR/QVBR; consider using CQP or ICQ rate control mode for better quality",
                );
            }
        }
        let gop = tuning.i_frames.max(1);
        // let gop = fps * 4;
        let ring_slots = recommended_slots(&options) as u64;
        let (parser, decoder) = codec_elements(&options.video_codec);
        let encode_chain = match options.encoder_backend {
            EncoderBackend::Vaapi => {
                let rc = vaapi_rate_control(&options.bitrate_mode, &options.video_codec)?;
                let enc = vaapi_encoder_name(&options.video_codec);
                let range = options.color_range.to_string();
                let mut vaapi_props: Vec<(&'static str, String)> = vec![
                    ("rate-control", rc.to_string()),
                    ("bitrate", bitrate.to_string()),
                    ("key-int-max", gop.to_string()),
                ];
                if options.video_codec == VideoCodec::Av1 {
                    vaapi_props.push(("ref-frames", "1".to_string()));
                    match options.bitrate_mode {
                        BitrateMode::Cbr => {
                            vaapi_props.push(("target-usage", tuning.target_usage.to_string()));
                            vaapi_props.push(("min-qp", tuning.min_qp.to_string()));
                            vaapi_props.push(("max-qp", tuning.max_qp.to_string()));
                            vaapi_props.push(("qp", tuning.qpi.to_string()));
                        }
                        BitrateMode::Vbr => {
                            vaapi_props.push(("target-usage", tuning.target_usage.to_string()));
                            vaapi_props.push(("target-percentage", "100".to_string()));
                            vaapi_props.push(("min-qp", tuning.min_qp.to_string()));
                            vaapi_props.push(("max-qp", tuning.max_qp.to_string()));
                            vaapi_props.push(("qp", tuning.qpi.to_string()));
                        }
                        BitrateMode::Cqp => {
                            vaapi_props.push(("target-usage", tuning.target_usage.to_string()));
                            vaapi_props.push(("min-qp", tuning.min_qp.to_string()));
                            vaapi_props.push(("max-qp", tuning.max_qp.to_string()));
                            vaapi_props.push(("qp", tuning.qpi.to_string()));
                        }
                        BitrateMode::Icq => {
                            vaapi_props.push(("target-usage", tuning.target_usage.to_string()));
                            vaapi_props.push(("min-qp", tuning.min_qp.to_string()));
                            vaapi_props.push(("max-qp", tuning.max_qp.to_string()));
                            vaapi_props.push(("qp", tuning.qpi.to_string()));
                        }
                        _ => {}
                    }
                } else {
                    // TODO: debug why setting i-frames makes the encoder shit itself
                    // vaapi_props.push(("i-frames", tuning.i_frames.to_string()));
                    // vaapi_props.push(("b-frames", tuning.b_frames.to_string()));
                    vaapi_props.push(("ref-frames", "1".to_string()));
                    match options.bitrate_mode {
                        // Clamp max-qp for quality consistency.
                        BitrateMode::Cbr => {
                            vaapi_props.push(("target-usage", tuning.target_usage.to_string()));
                            vaapi_props.push(("min-qp", tuning.min_qp.to_string()));
                            vaapi_props.push(("max-qp", tuning.max_qp.to_string()));
                            vaapi_props.push(("qpi", tuning.qpi.to_string()));
                        }
                        BitrateMode::Vbr | BitrateMode::Qvbr => {
                            vaapi_props.push(("target-usage", tuning.target_usage.to_string()));
                            vaapi_props.push(("target-percentage", "100".to_string()));
                            vaapi_props.push(("min-qp", tuning.min_qp.to_string()));
                            vaapi_props.push(("max-qp", tuning.max_qp.to_string()));
                            vaapi_props.push(("qpi", tuning.qpi.to_string()));
                        }
                        BitrateMode::Icq => {
                            vaapi_props.push(("target-usage", tuning.target_usage.to_string()));
                            vaapi_props.push(("min-qp", tuning.min_qp.to_string()));
                            vaapi_props.push(("max-qp", tuning.max_qp.to_string()));
                            vaapi_props.push(("qpi", tuning.qpi.to_string()));
                        }
                        BitrateMode::Cqp => {
                            vaapi_props.push(("target-usage", tuning.target_usage.to_string()));
                            vaapi_props.push(("min-qp", tuning.min_qp.to_string()));
                            vaapi_props.push(("max-qp", tuning.max_qp.to_string()));
                            vaapi_props.push(("qpi", tuning.qpi.to_string()));
                            // only available in cqp
                            // dont set qpb without setting b-frames, gstreamer seems to not like that
                            // vaapi_props.push(("qpb", tuning.qpb.to_string()));
                            vaapi_props.push(("qpp", tuning.qpp.to_string()));
                        }
                        _ => {}
                    }
                }
                let vaapi_props = render_encoder_props(enc, vaapi_props);
                format!(
                    concat!(
                        "! vapostproc ",
                        "! video/x-raw(memory:VAMemory),format=NV12,width={w},height={h},framerate={fps}/1,color-range=(string){range},colorimetry=(string){colorimetry} ",
                        "! {enc} name=enc {vaapi_props} ",
                        "! {parser} "
                    ),
                    w = w,
                    h = h,
                    fps = fps,
                    range = range,
                    colorimetry = colorimetry,
                    enc = enc,
                    vaapi_props = vaapi_props.as_str(),
                    parser = parser,
                )
            }
            EncoderBackend::Qsv => {
                let rc = qsv_rate_control(&options.bitrate_mode, &options.video_codec)?;
                let enc = qsv_encoder_name(&options.video_codec);
                let range = options.color_range.to_string();
                let mut qsv_props: Vec<(&'static str, String)> = vec![
                    ("rate-control", rc.to_string()),
                    ("bitrate", bitrate.to_string()),
                    ("gop-size", gop.to_string()),
                    // ("low-latency", "true".to_string()),
                    // ("target-usage", tuning.target_usage.to_string()),
                    ("b-frames", tuning.b_frames.to_string()),
                    ("ref-frames", "1".to_string()),
                    ("cabac", "on".to_string()),
                ];
                match options.bitrate_mode {
                    BitrateMode::Icq => {
                        qsv_props.push(("icq-quality", tuning.icq_quality.to_string()));
                        qsv_props.push(("max-qp-i", tuning.max_qp.to_string()));
                        qsv_props.push(("max-qp-p", tuning.max_qp.to_string()));
                        qsv_props.push(("max-qp-b", tuning.max_qp.to_string()));
                    }
                    BitrateMode::Cqp => {
                        qsv_props.push(("qp-i", tuning.qpi.to_string()));
                        qsv_props.push(("qp-p", tuning.qpp.to_string()));
                        qsv_props.push(("qp-b", tuning.qpb.to_string()));
                        qsv_props.push(("max-qp-i", tuning.max_qp.to_string()));
                        qsv_props.push(("max-qp-p", tuning.max_qp.to_string()));
                        qsv_props.push(("max-qp-b", tuning.max_qp.to_string()));
                    }
                    BitrateMode::Qvbr => {
                        qsv_props.push(("qvbr-quality", tuning.qvbr_quality.to_string()));
                        qsv_props.push(("max-qp-i", tuning.max_qp.to_string()));
                        qsv_props.push(("max-qp-p", tuning.max_qp.to_string()));
                        qsv_props.push(("max-qp-b", tuning.max_qp.to_string()));
                    }
                    _ => {
                        qsv_props.push(("qp-i", tuning.qpi.to_string()));
                        qsv_props.push(("qp-p", tuning.qpp.to_string()));
                        qsv_props.push(("qp-b", tuning.qpb.to_string()));
                        qsv_props.push(("max-qp-i", tuning.max_qp.to_string()));
                        qsv_props.push(("max-qp-p", tuning.max_qp.to_string()));
                        qsv_props.push(("max-qp-b", tuning.max_qp.to_string()));
                    }
                }
                let qsv_props = render_encoder_props(enc, qsv_props);
                format!(
                    concat!(
                        "! vapostproc ",
                        "! video/x-raw(memory:VAMemory),format=NV12,width={w},height={h},framerate={fps}/1,color-range=(string){range},colorimetry=(string){colorimetry} ",
                        "! {enc} name=enc {qsv_props} ",
                        "! {parser} "
                    ),
                    w = w,
                    h = h,
                    fps = fps,
                    range = range,
                    colorimetry = colorimetry,
                    enc = enc,
                    qsv_props = qsv_props.as_str(),
                    parser = parser,
                )
            }
            EncoderBackend::Vulkan => {
                // this is just a placeholder, i haven't gotten to vulkan yet
                let rc = vulkan_rate_control(&options.bitrate_mode)?;
                let enc = vulkan_encoder_name(&options.video_codec);
                let range = options.color_range.to_string();
                format!(
                    concat!(
                        "! vulkanupload ",
                        "! vulkancolorconvert ",
                        "! video/x-raw(memory:VulkanImage),format=NV12,width={w},height={h},framerate={fps}/1,color-range=(string){range},colorimetry=(string){colorimetry} ",
                        "! {enc} name=enc rate-control={rc} bitrate={bitrate} quality=5 min-qp=1 max-qp=30 ",
                        "! {parser} "
                    ),
                    w = w,
                    h = h,
                    fps = fps,
                    range = range,
                    colorimetry = colorimetry,
                    enc = enc,
                    rc = rc,
                    bitrate = bitrate,
                    parser = parser,
                )
            }
            EncoderBackend::Cpu => {
                let rc = cpu_rate_control(&options.bitrate_mode)?;
                let enc = cpu_encoder_name(&options.video_codec);
                log::debug!(
                    "Only x264enc supports rate-control among CPU encoders, mapping requested {:?} to rate-control={rc}",
                    options.bitrate_mode
                );
                match options.video_codec {
                    VideoCodec::H264 => format!(
                        concat!(
                            "! videoconvert ",
                            "! videorate ",
                            "! video/x-raw,format=NV12,width={w},height={h},framerate={fps}/1,color-range=(string){range},colorimetry=(string){colorimetry} ",
                            "! {enc} name=enc bitrate={bitrate} pass={pass} speed-preset=veryfast tune=zerolatency key-int-max={gop} bframes=0 cabac=true rc-lookahead=0 sync-lookahead=0 threads=0 sliced-threads=true ",
                            "! h264parse "
                        ),
                        w = w,
                        h = h,
                        fps = fps,
                        range = options.color_range.to_string(),
                        colorimetry = colorimetry,
                        enc = enc,
                        bitrate = bitrate,
                        pass = rc,
                        gop = gop,
                    ),
                    VideoCodec::H265 => format!(
                        concat!(
                            "! videoconvert ",
                            "! videorate ",
                            "! video/x-raw,format=NV12,width={w},height={h},framerate={fps}/1,color-range=(string){range},colorimetry=(string){colorimetry} ",
                            "! {enc} name=enc bitrate={bitrate} speed-preset=veryfast key-int-max={gop} ",
                            "! h265parse "
                        ),
                        w = w,
                        h = h,
                        fps = fps,
                        range = options.color_range.to_string(),
                        colorimetry = colorimetry,
                        enc = enc,
                        bitrate = bitrate,
                        gop = gop,
                    ),
                    VideoCodec::Av1 => format!(
                        concat!(
                            "! videoconvert ",
                            "! videorate ",
                            "! video/x-raw,format=NV12,width={w},height={h},framerate={fps}/1,color-range=(string){range},colorimetry=(string){colorimetry} ",
                            "! {enc} name=enc bitrate={bitrate} speed-preset=veryfast tune=0 key-int-max={gop} bframes=0 rc-lookahead=0 sync-lookahead=0 threads=0 sliced-threads=true ",
                            "! av1parse "
                        ),
                        w = w,
                        h = h,
                        fps = fps,
                        range = options.color_range.to_string(),
                        colorimetry = colorimetry,
                        enc = enc,
                        bitrate = bitrate,
                        gop = gop,
                    ),
                }
            }
        };

        let desc = match output {
            EncoderOutput::File(out_path) => format!(
                concat!(
                    "appsrc name=src is-live=true format=time do-timestamp=false block=true ",
                    "! queue max-size-buffers={ring} max-size-bytes=0 max-size-time=0 ",
                    "{encode_chain}",
                    "! mp4mux faststart=true ",
                    "! filesink location={out}"
                ),
                ring = ring_slots,
                encode_chain = encode_chain,
                out = out_path
            ),
            EncoderOutput::Preview => format!(
                concat!(
                    "appsrc name=src is-live=true format=time do-timestamp=false block=true ",
                    "! queue max-size-buffers={ring} max-size-bytes=0 max-size-time=0 ",
                    "{encode_chain}",
                    "! {decoder} ",
                    "! videoconvert ",
                    "! autovideosink sync=false"
                ),
                ring = ring_slots,
                encode_chain = encode_chain,
                decoder = decoder
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

        let max_buffers = ring_slots;
        appsrc.set_max_bytes(0);
        appsrc.set_property("max-buffers", max_buffers);
        appsrc.set_property("max-time", 0u64);
        appsrc.set_block(true);

        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| EncodeError::Bus(format!("failed to set Playing: {e:?}")))?;

        // wait short time for cap negotiation
        thread::sleep(std::time::Duration::from_millis(100));

        if let Some(enc) = pipeline.by_name("enc") {
            let rate = enc
                .find_property("rate-control")
                .map(|_| enc.property_value("rate-control"));
            let br = enc
                .find_property("bitrate")
                .map(|_| enc.property_value("bitrate"));
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
            FrameRateMode::Cfr => {
                let wall_time = (elapsed_ns / self.frame_ns).saturating_mul(self.frame_ns);
                self.next_pts_ns.max(wall_time)
            }
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

    pub fn request_keyframe(&self, reason: &str) {
        let event = DownstreamForceKeyUnitEvent::builder()
            .all_headers(true)
            .build();
        let sent = self.appsrc.upcast_ref::<gst::Element>().send_event(event);
        if sent {
            log::debug!("Requested force keyframe ({reason})");
        } else {
            log::warn!("Failed to request force keyframe ({reason})");
        }
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
