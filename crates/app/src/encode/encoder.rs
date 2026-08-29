use std::borrow::Cow;
use std::collections::HashSet;
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use gstreamer::prelude::*;
use gstreamer::{self as gst, glib};
use gstreamer_app as gst_app;
use gstreamer_video::DownstreamForceKeyUnitEvent;

use crate::drm_kms::gstreamer::{ExportError, push_exported_dmabuf};
use crate::drm_kms::types::{
    BitrateMode, ColorRange, Colorimetry, EncoderBackend, ExportedDmabuf, FrameRateMode,
    OutputContainer, Profile, QualityPreset, VideoCodec,
};

use super::replay_buffer::ReplayBuffer;

#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    #[error("gst init failed: {0}")]
    Init(#[from] glib::Error),
    #[error("pipeline parse failed: {0}")]
    Parse(#[from] glib::BoolError),
    #[error("missing appsrc")]
    MissingAppSrc,
    #[error("missing appsink")]
    MissingAppSink,
    #[error("push failed: {0}")]
    Push(#[from] ExportError),
    #[error("bus error: {0}")]
    Bus(String),
    #[error("invalid encoder configuration: {0}")]
    InvalidConfig(String),
    #[error("replay buffer error: {0}")]
    Replay(String),
}

pub struct GstEncoder {
    pipeline: gst::Pipeline,
    appsrc: gst_app::AppSrc,
    options: EncoderOptions,
    frame_ns: u64,
    next_pts_ns: u64,
    paused: bool,
    recording_started: Instant,
    pause_started: Option<Instant>,
    paused_total_ns: u64,
    last_pts_ns: Option<u64>,
    last_push_wall: Option<Instant>,
    replay: Option<Arc<Mutex<ReplayBuffer>>>,
    failure: Arc<PipelineFailure>,
}

#[derive(Default)]
struct PipelineFailure {
    message: Mutex<Option<String>>,
}

impl PipelineFailure {
    fn record(&self, message: String) {
        match self.message.lock() {
            Ok(mut slot) if slot.is_none() => *slot = Some(message),
            Ok(_) => {}
            Err(e) => log::error!("pipeline failure lock poisoned: {e}"),
        }
    }

    fn check(&self) -> Result<(), EncodeError> {
        let failure = self
            .message
            .lock()
            .map_err(|e| EncodeError::Bus(format!("pipeline failure lock poisoned: {e}")))?
            .clone();
        failure.map_or(Ok(()), |message| Err(EncodeError::Bus(message)))
    }
}

pub enum EncoderOutput<'a> {
    File(&'a str),
    Preview,
    ReplayBuffer { seconds: u32 },
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
    pub output_container: OutputContainer,
    pub profile: Option<Profile>,
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
        0x30334241 => Some("AB30"), // DRM_FORMAT_ABGR2101010
        0x30334258 => Some("XB30"), // DRM_FORMAT_XBGR2101010
        0x30335241 => Some("AR30"), // DRM_FORMAT_ARGB2101010
        0x30335258 => Some("XR30"), // DRM_FORMAT_XRGB2101010
        0x48344241 => Some("AB4H"), // DRM_FORMAT_ABGR16161616F
        0x48344258 => Some("XB4H"), // DRM_FORMAT_XBGR16161616F
        0x48345241 => Some("AR4H"), // DRM_FORMAT_ARGB16161616F
        0x48345258 => Some("XR4H"), // DRM_FORMAT_XRGB16161616F
        0x3231564e => Some("NV12"), // DRM_FORMAT_NV12
        _ => None,
    }
}

fn fourcc_to_raw_format(fourcc: u32) -> Option<&'static str> {
    match fourcc {
        0x34324241 => Some("RGBA"),    // DRM_FORMAT_ABGR8888 (LE memory: RGBA)
        0x34324258 => Some("RGBx"),    // DRM_FORMAT_XBGR8888
        0x34325241 => Some("BGRA"),    // DRM_FORMAT_ARGB8888
        0x34325258 => Some("BGRx"),    // DRM_FORMAT_XRGB8888
        0x30334241 => Some("RGB10A2"), // DRM_FORMAT_ABGR2101010
        0x30334258 => Some("RGB10A2"), // DRM_FORMAT_XBGR2101010
        0x30335241 => Some("BGR10A2"), // DRM_FORMAT_ARGB2101010
        0x30335258 => Some("BGR10A2"), // DRM_FORMAT_XRGB2101010
        0x3231564e => Some("NV12"),    // DRM_FORMAT_NV12
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

struct ProfileSelection {
    encoder_profile: &'static str,
    input_format: &'static str,
    colorimetry: Colorimetry,
}

fn resolve_profile(
    profile: Profile,
    backend: &EncoderBackend,
    codec: &VideoCodec,
) -> Result<ProfileSelection, EncodeError> {
    if !matches!(backend, EncoderBackend::Vaapi | EncoderBackend::Qsv) {
        return Err(EncodeError::InvalidConfig(format!(
            "profile {:?} is only supported on vaapi/qsv backends",
            profile
        )));
    }
    if !matches!(codec, VideoCodec::H265 | VideoCodec::Av1) {
        return Err(EncodeError::InvalidConfig(format!(
            "profile {:?} is only supported for h265/av1 codecs",
            profile
        )));
    }

    let out = match (codec, profile) {
        // HDR10 = 10-bit path
        (VideoCodec::H265, Profile::Hdr10) => ProfileSelection {
            encoder_profile: "main-10",
            input_format: "P010_10LE",
            colorimetry: Colorimetry::Bt2020,
        },
        (VideoCodec::Av1, Profile::Hdr10) => ProfileSelection {
            encoder_profile: "main",
            input_format: "P010_10LE",
            colorimetry: Colorimetry::Bt2020,
        },

        // Non-HDR10 profiles stay 8-bit NV12.
        (VideoCodec::H265, Profile::Hdr) => ProfileSelection {
            encoder_profile: "main",
            input_format: "NV12",
            colorimetry: Colorimetry::Bt2020,
        },
        (VideoCodec::Av1, Profile::Hdr) => ProfileSelection {
            encoder_profile: "main",
            input_format: "NV12",
            colorimetry: Colorimetry::Bt2020,
        },
        (VideoCodec::H265, Profile::Sdr) | (VideoCodec::Av1, Profile::Sdr) => ProfileSelection {
            encoder_profile: "main",
            input_format: "NV12",
            colorimetry: Colorimetry::Bt709,
        },
        (VideoCodec::H264, _) => unreachable!("validated codec/profile mismatch"),
    };

    Ok(out)
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

fn encoder_factory_name(backend: &EncoderBackend, codec: &VideoCodec) -> &'static str {
    match backend {
        EncoderBackend::Vaapi => vaapi_encoder_name(codec),
        EncoderBackend::Qsv => qsv_encoder_name(codec),
        EncoderBackend::Vulkan => vulkan_encoder_name(codec),
        EncoderBackend::Cpu => cpu_encoder_name(codec),
    }
}

fn require_gst_elements(elements: &[&str]) -> Result<(), EncodeError> {
    let missing: Vec<_> = elements
        .iter()
        .filter(|name| gst::ElementFactory::find(**name).is_none())
        .copied()
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(EncodeError::Bus(format!(
            "missing GStreamer OpenGL element(s): {}; install your distribution's GStreamer OpenGL plugin package",
            missing.join(", ")
        )))
    }
}

fn h264_profile_from_quality(quality: &QualityPreset) -> &'static str {
    match quality {
        QualityPreset::Low => "constrained-baseline",
        QualityPreset::Medium | QualityPreset::High => "main",
        QualityPreset::VeryHigh | QualityPreset::Ultra => "high",
    }
}

// note to self: this doesn't work actually, ffprobe report color_transfer=bt2020-12 no matter what transfer function is used
fn transfer_for_profile(profile: Option<Profile>) -> Option<&'static str> {
    match profile {
        Some(Profile::Hdr10) => Some("smpte-st-2084"),
        Some(Profile::Hdr) => Some("arib-std-b67"),
        _ => None,
    }
}

fn encoded_profile_caps(
    codec: &VideoCodec,
    profile: Option<&'static str>,
    quality: &QualityPreset,
    colorimetry: &str,
    transfer_fn: Option<&str>,
) -> Cow<'static, str> {
    let (media, prof) = match (codec, profile) {
        (VideoCodec::H264, _) => ("video/x-h264", h264_profile_from_quality(quality)),
        (VideoCodec::H265, Some(x)) => ("video/x-h265", x),
        (VideoCodec::Av1, Some("main")) => ("video/x-av1", "main"),
        _ => return Cow::Borrowed(""),
    };
    let s = match transfer_fn {
        Some(tf) => format!(
            "! {media},profile=(string){prof},colorimetry=(string){colorimetry},transfer-function=(string){tf} "
        ),
        None => format!("! {media},profile=(string){prof},colorimetry=(string){colorimetry} "),
    };
    Cow::Owned(s)
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

fn encoder_rate_control_values(factory_name: &str) -> Option<HashSet<String>> {
    let factory = gst::ElementFactory::find(factory_name)?;
    let elem = factory.create().build().ok()?;
    let pspec = elem.find_property("rate-control")?;
    let enum_class = glib::EnumClass::with_type(pspec.value_type())?;
    Some(
        enum_class
            .values()
            .iter()
            .map(|v| v.nick().to_string())
            .collect(),
    )
}

fn is_codec_supported(backend: &EncoderBackend, codec: &VideoCodec) -> bool {
    match (backend, codec) {
        (
            EncoderBackend::Vaapi | EncoderBackend::Qsv | EncoderBackend::Cpu,
            VideoCodec::H264 | VideoCodec::H265 | VideoCodec::Av1,
        ) => true,
        (EncoderBackend::Vulkan, VideoCodec::H264 | VideoCodec::H265 | VideoCodec::Av1) => false, // never tested
    }
}

fn set_appsrc_caps(
    appsrc: &gst_app::AppSrc,
    ex: &ExportedDmabuf,
    opts: &EncoderOptions,
) -> Result<(), String> {
    let drm = fourcc_to_drm_format(ex.fourcc);
    let raw = fourcc_to_raw_format(ex.fourcc);
    let fps_fraction = match opts.frame_rate_mode {
        FrameRateMode::Cfr => format!("{}/1", opts.fps.max(1)),
        FrameRateMode::Vfr => "0/1".to_string(),
    };
    let range = &opts.color_range.to_string();
    let colorimetry = opts.colorimetry.to_string();
    let transfer_suffix = transfer_for_profile(opts.profile)
        .map(|tf| format!(",transfer-function=(string){tf}"))
        .unwrap_or_default();

    // Modifier 0 is linear, so no layout information is lost by using raw caps.
    // This keeps linear KMS exports usable on GL stacks that cannot import
    // DMA_DRM directly; non-linear buffers always retain their DRM modifier.
    if ex.modifier == 0
        && let Some(raw) = raw
    {
        let linear_raw = format!(
            "video/x-raw,format=(string){},width=(int){},height=(int){},framerate=(fraction){},color-range=(string){},colorimetry=(string){}{}",
            raw,
            ex.width,
            ex.height,
            fps_fraction,
            range,
            colorimetry.as_str(),
            transfer_suffix.as_str()
        );
        let caps = gst::Caps::from_str(&linear_raw)
            .map_err(|e| format!("linear raw caps parse failed: {e}"))?;
        log::debug!("Using appsrc caps (linear modifier fallback): {linear_raw}");
        appsrc.set_caps(Some(&caps));
        return Ok(());
    }

    if let Some(drm) = drm {
        let drm_with_mod = format!("{drm}:0x{:016x}", ex.modifier);
        let full_with_mod = format!(
            "video/x-raw(memory:DMABuf),format=(string)DMA_DRM,drm-format=(string){},width=(int){},height=(int){},framerate=(fraction){},color-range=(string){},colorimetry=(string){}{}",
            drm_with_mod,
            ex.width,
            ex.height,
            fps_fraction,
            range,
            colorimetry.as_str(),
            transfer_suffix.as_str()
        );
        match gst::Caps::from_str(&full_with_mod) {
            Ok(caps) => {
                log::debug!("Using appsrc caps: {full_with_mod}");
                appsrc.set_caps(Some(&caps));
                return Ok(());
            }
            Err(e) => {
                return Err(format!("DMA_DRM+modifier caps parse failed: {e}"));
            }
        }
    }

    if ex.modifier != 0 {
        return Err(format!(
            "unsupported DRM fourcc=0x{:08x} with modifier=0x{:016x}",
            ex.fourcc, ex.modifier
        ));
    }

    if let Some(raw) = raw {
        let dmabuf_raw = format!(
            "video/x-raw(memory:DMABuf),format=(string){},width=(int){},height=(int){},framerate=(fraction){},color-range=(string){},colorimetry=(string){}{}",
            raw,
            ex.width,
            ex.height,
            fps_fraction,
            range,
            colorimetry.as_str(),
            transfer_suffix.as_str()
        );
        if let Ok(caps) = gst::Caps::from_str(&dmabuf_raw) {
            log::debug!("Using appsrc caps: {dmabuf_raw}");
            appsrc.set_caps(Some(&caps));
            return Ok(());
        }

        let raw_fallback = format!(
            "video/x-raw,format=(string){},width=(int){},height=(int){},framerate=(fraction){},color-range=(string){},colorimetry=(string){}{}",
            raw,
            ex.width,
            ex.height,
            fps_fraction,
            range,
            colorimetry.as_str(),
            transfer_suffix.as_str()
        );
        let caps = gst::Caps::from_str(&raw_fallback)
            .map_err(|e| format!("fallback caps parse failed: {e}"))?;
        log::debug!("Using appsrc caps: {raw_fallback}");
        appsrc.set_caps(Some(&caps));
        return Ok(());
    }

    Err(format!(
        "unsupported fourcc=0x{:08x} modifier=0x{:016x} (no usable raw/drm mapping)",
        ex.fourcc, ex.modifier
    ))
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
            min_qp: 24,
            max_qp: 48,
            i_frames: 120,
            b_frames: 0,
            target_usage: 3,
            icq_quality: 28,
            qvbr_quality: 28,
        },
        QualityPreset::High => QualityTuning {
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
        QualityPreset::VeryHigh => QualityTuning {
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

fn bitrate_mode_priority(backend: &EncoderBackend, codec: &VideoCodec) -> Vec<BitrateMode> {
    match (backend, codec) {
        (EncoderBackend::Vaapi, VideoCodec::H264 | VideoCodec::H265) => vec![
            BitrateMode::Icq,
            BitrateMode::Cqp,
            BitrateMode::Qvbr,
            BitrateMode::Vbr,
            BitrateMode::Cbr,
            BitrateMode::Vcm,
        ],
        (EncoderBackend::Vaapi, VideoCodec::Av1) => {
            vec![
                BitrateMode::Icq,
                BitrateMode::Cqp,
                BitrateMode::Vbr,
                BitrateMode::Cbr,
            ]
        }
        (EncoderBackend::Qsv, VideoCodec::H264 | VideoCodec::H265) => vec![
            BitrateMode::Icq,
            BitrateMode::Cqp,
            BitrateMode::Qvbr,
            BitrateMode::Vbr,
            BitrateMode::Cbr,
            BitrateMode::Vcm,
        ],
        (EncoderBackend::Qsv, VideoCodec::Av1) => {
            vec![BitrateMode::Cqp, BitrateMode::Vbr, BitrateMode::Cbr]
        }
        (EncoderBackend::Vulkan, _) => vec![BitrateMode::Cqp, BitrateMode::Vbr, BitrateMode::Cbr],
        (EncoderBackend::Cpu, _) => vec![
            BitrateMode::Qual,
            BitrateMode::Quant,
            BitrateMode::Cbr,
            BitrateMode::Pass1,
            BitrateMode::Pass2,
            BitrateMode::Pass3,
        ],
    }
}

fn rate_control_nick_for(
    backend: &EncoderBackend,
    codec: &VideoCodec,
    mode: &BitrateMode,
) -> Option<&'static str> {
    match backend {
        EncoderBackend::Vaapi => vaapi_rate_control(mode, codec).ok(),
        EncoderBackend::Qsv => qsv_rate_control(mode, codec).ok(),
        EncoderBackend::Vulkan => vulkan_rate_control(mode).ok(),
        EncoderBackend::Cpu => cpu_rate_control(mode).ok(),
    }
}

fn resolve_rate_control_mode(
    backend: &EncoderBackend,
    codec: &VideoCodec,
    requested_mode: &BitrateMode,
) -> Result<BitrateMode, EncodeError> {
    let factory_name = encoder_factory_name(backend, codec);
    let supported = encoder_rate_control_values(factory_name);

    let mut candidates = Vec::new();
    candidates.push(requested_mode.clone());
    for mode in bitrate_mode_priority(backend, codec) {
        if !candidates.contains(&mode) {
            candidates.push(mode);
        }
    }

    for mode in candidates {
        let Some(rc_nick) = rate_control_nick_for(backend, codec, &mode) else {
            continue;
        };
        if let Some(supported) = &supported
            && !supported.contains(rc_nick)
        {
            continue;
        }
        if &mode != requested_mode {
            log::warn!(
                "rate-control {:?} unavailable on {} for {:?}/{:?}; falling back to {:?} ({})",
                requested_mode,
                factory_name,
                backend,
                codec,
                mode,
                rc_nick
            );
        }
        return Ok(mode);
    }

    Err(EncodeError::InvalidConfig(format!(
        "no supported rate-control found for backend={:?} codec={:?} encoder={}",
        backend, codec, factory_name
    )))
}

impl GstEncoder {
    pub fn new_with_output(
        output: EncoderOutput<'_>,
        ex: &ExportedDmabuf,
        mut options: EncoderOptions,
    ) -> Result<Self, EncodeError> {
        gst::init()?;
        if options.encoder_backend == EncoderBackend::Cpu {
            require_gst_elements(&["glupload", "glcolorconvert", "gldownload"])?;
        }
        if matches!(&output, EncoderOutput::Preview) {
            require_gst_elements(&["glupload", "glcolorconvert", "glimagesink"])?;
        }
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
        let requested_mode = if options.bitrate_mode == BitrateMode::Default {
            default_rate_control_for(&options.encoder_backend, &options.video_codec)
        } else {
            options.bitrate_mode.clone()
        };
        let selected_mode = resolve_rate_control_mode(
            &options.encoder_backend,
            &options.video_codec,
            &requested_mode,
        )?;
        if options.bitrate_mode == BitrateMode::Default {
            log::info!(
                "rate-control=default resolved to {:?} for backend={:?} codec={:?}",
                selected_mode,
                options.encoder_backend,
                options.video_codec
            );
        } else if selected_mode != options.bitrate_mode {
            log::warn!(
                "requested rate-control {:?} resolved to {:?} for backend={:?} codec={:?}",
                options.bitrate_mode,
                selected_mode,
                options.encoder_backend,
                options.video_codec
            );
        }
        options.bitrate_mode = selected_mode;
        let fps = options.fps.max(1);
        let fps_fraction = match options.frame_rate_mode {
            FrameRateMode::Cfr => format!("{fps}/1"),
            FrameRateMode::Vfr => "0/1".to_string(),
        };
        let videorate = match options.frame_rate_mode {
            FrameRateMode::Cfr => "! videorate ",
            FrameRateMode::Vfr => "",
        };
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
        let mut colorimetry = options.colorimetry.to_string();
        let transfer_fn = transfer_for_profile(options.profile);
        let mut encoder_profile: Option<&'static str> = None;
        let mut encoder_input_format = "NV12";
        if let Some(p) = options.profile {
            let resolved = resolve_profile(p, &options.encoder_backend, &options.video_codec)?;
            encoder_profile = Some(resolved.encoder_profile);
            encoder_input_format = resolved.input_format;
            options.colorimetry = resolved.colorimetry;
            colorimetry = options.colorimetry.to_string();
            log::info!(
                "Using profile {:?}: encoder_profile={} input_format={} colorimetry={} transfer={:?}",
                p,
                resolved.encoder_profile,
                resolved.input_format,
                colorimetry,
                transfer_fn
            );
        }
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
                let profile_caps = encoded_profile_caps(
                    &options.video_codec,
                    encoder_profile,
                    &options.quality,
                    colorimetry.as_str(),
                    transfer_fn,
                );
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
                    vaapi_props.push(("ref-frames", "3".to_string()));
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
                        "! video/x-raw(memory:VAMemory),format={fmt},width={w},height={h},framerate={fps_fraction},color-range=(string){range},colorimetry=(string){colorimetry} ",
                        "! {enc} name=enc {vaapi_props} ",
                        "{profile_caps}",
                        "! {parser} "
                    ),
                    fmt = encoder_input_format,
                    w = w,
                    h = h,
                    fps_fraction = fps_fraction,
                    range = range,
                    colorimetry = colorimetry,
                    enc = enc,
                    vaapi_props = vaapi_props.as_str(),
                    profile_caps = profile_caps,
                    parser = parser,
                )
            }
            EncoderBackend::Qsv => {
                let rc = qsv_rate_control(&options.bitrate_mode, &options.video_codec)?;
                let enc = qsv_encoder_name(&options.video_codec);
                let range = options.color_range.to_string();
                let profile_caps = encoded_profile_caps(
                    &options.video_codec,
                    encoder_profile,
                    &options.quality,
                    colorimetry.as_str(),
                    transfer_fn,
                );
                let mut qsv_props: Vec<(&'static str, String)> = vec![
                    ("rate-control", rc.to_string()),
                    ("bitrate", bitrate.to_string()),
                    ("gop-size", gop.to_string()),
                    // ("low-latency", "true".to_string()),
                    // ("target-usage", tuning.target_usage.to_string()),
                    ("b-frames", tuning.b_frames.to_string()),
                    ("ref-frames", "3".to_string()),
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
                        "! video/x-raw(memory:VAMemory),format={fmt},width={w},height={h},framerate={fps_fraction},color-range=(string){range},colorimetry=(string){colorimetry} ",
                        "! {enc} name=enc {qsv_props} ",
                        "{profile_caps}",
                        "! {parser} "
                    ),
                    fmt = encoder_input_format,
                    w = w,
                    h = h,
                    fps_fraction = fps_fraction,
                    range = range,
                    colorimetry = colorimetry,
                    enc = enc,
                    qsv_props = qsv_props.as_str(),
                    profile_caps = profile_caps,
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
                        "! video/x-raw(memory:VulkanImage),format=NV12,width={w},height={h},framerate={fps_fraction},color-range=(string){range},colorimetry=(string){colorimetry} ",
                        "! {enc} name=enc rate-control={rc} bitrate={bitrate} quality=5 min-qp=1 max-qp=30 ",
                        "! {parser} "
                    ),
                    w = w,
                    h = h,
                    fps_fraction = fps_fraction,
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
                let gl_import = concat!(
                    "! glupload ",
                    "! glcolorconvert ",
                    "! video/x-raw(memory:GLMemory),format=RGBA ",
                    "! gldownload ",
                    "! video/x-raw,format=RGBA "
                );
                log::debug!(
                    "Only x264enc supports rate-control among CPU encoders, mapping requested {:?} to rate-control={rc}",
                    options.bitrate_mode
                );
                match options.video_codec {
                    VideoCodec::H264 => format!(
                        concat!(
                            "{gl_import}",
                            "! videoconvert ",
                            "{videorate}",
                            "! video/x-raw,format=NV12,width={w},height={h},framerate={fps_fraction},color-range=(string){range},colorimetry=(string){colorimetry} ",
                            "! {enc} name=enc bitrate={bitrate} pass={pass} speed-preset=veryfast tune=zerolatency key-int-max={gop} bframes=0 cabac=true rc-lookahead=0 sync-lookahead=0 threads=0 sliced-threads=true ",
                            "! h264parse "
                        ),
                        w = w,
                        h = h,
                        gl_import = gl_import,
                        fps_fraction = fps_fraction,
                        videorate = videorate,
                        range = options.color_range.to_string(),
                        colorimetry = colorimetry,
                        enc = enc,
                        bitrate = bitrate,
                        pass = rc,
                        gop = gop,
                    ),
                    VideoCodec::H265 => format!(
                        concat!(
                            "{gl_import}",
                            "! videoconvert ",
                            "{videorate}",
                            "! video/x-raw,format=NV12,width={w},height={h},framerate={fps_fraction},color-range=(string){range},colorimetry=(string){colorimetry} ",
                            "! {enc} name=enc bitrate={bitrate} speed-preset=veryfast key-int-max={gop} ",
                            "! h265parse "
                        ),
                        w = w,
                        h = h,
                        gl_import = gl_import,
                        fps_fraction = fps_fraction,
                        videorate = videorate,
                        range = options.color_range.to_string(),
                        colorimetry = colorimetry,
                        enc = enc,
                        bitrate = bitrate,
                        gop = gop,
                    ),
                    VideoCodec::Av1 => format!(
                        concat!(
                            "{gl_import}",
                            "! videoconvert ",
                            "{videorate}",
                            "! video/x-raw,format=NV12,width={w},height={h},framerate={fps_fraction},color-range=(string){range},colorimetry=(string){colorimetry} ",
                            "! {enc} name=enc bitrate={bitrate} speed-preset=veryfast tune=0 key-int-max={gop} bframes=0 rc-lookahead=0 sync-lookahead=0 threads=0 sliced-threads=true ",
                            "! av1parse "
                        ),
                        w = w,
                        h = h,
                        gl_import = gl_import,
                        fps_fraction = fps_fraction,
                        videorate = videorate,
                        range = options.color_range.to_string(),
                        colorimetry = colorimetry,
                        enc = enc,
                        bitrate = bitrate,
                        gop = gop,
                    ),
                }
            }
        };

        let desc = match &output {
            EncoderOutput::File(out_path) => format!(
                concat!(
                    "appsrc name=src is-live=true format=time do-timestamp=false block=true ",
                    "! queue max-size-buffers={ring} max-size-bytes=0 max-size-time=0 ",
                    "{encode_chain}",
                    "{mux_chain}",
                    "! filesink location={out}"
                ),
                ring = ring_slots,
                encode_chain = encode_chain,
                mux_chain = options.output_container.mux_chain(),
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
            EncoderOutput::ReplayBuffer { seconds: _ } => format!(
                concat!(
                    "appsrc name=src is-live=true format=time do-timestamp=false block=true ",
                    "! queue max-size-buffers={ring} max-size-bytes=0 max-size-time=0 ",
                    "{encode_chain}",
                    "! appsink name=replay_sink emit-signals=false sync=false max-buffers=0 drop=false"
                ),
                ring = ring_slots,
                encode_chain = encode_chain,
            ),
        };
        log::debug!("GStreamer pipeline: {desc}");

        let replay_seconds = match output {
            EncoderOutput::ReplayBuffer { seconds } => Some(seconds),
            EncoderOutput::File(_) | EncoderOutput::Preview => None,
        };
        Self::start_pipeline(&desc, ex, options, ring_slots, replay_seconds)
    }

    fn start_pipeline(
        desc: &str,
        ex: &ExportedDmabuf,
        options: EncoderOptions,
        ring_slots: u64,
        replay_seconds: Option<u32>,
    ) -> Result<Self, EncodeError> {
        let fps = options.fps.max(1);

        let element = gst::parse::launch(desc).map_err(|e| {
            let detail = e.to_string();
            let hint = if detail.contains("no element") || detail.contains("no plugin") {
                "; install the GStreamer plugin package that provides the missing element"
            } else {
                ""
            };
            EncodeError::Bus(format!(
                "failed to parse GStreamer pipeline: {detail}{hint}"
            ))
        })?;
        let pipeline = element
            .downcast::<gst::Pipeline>()
            .map_err(|_| glib::bool_error!("parsed element is not a pipeline"))?;

        let failure = Arc::new(PipelineFailure::default());
        let bus = pipeline
            .bus()
            .ok_or_else(|| EncodeError::Bus("pipeline has no bus".to_string()))?;
        let failure_for_bus = Arc::clone(&failure);
        bus.set_sync_handler(move |_, message| {
            let source = message
                .src()
                .map(|source| source.name().to_string())
                .unwrap_or_else(|| "unknown source".to_string());
            match message.view() {
                gst::MessageView::Error(error) => {
                    let details = error
                        .debug()
                        .map(|details| details.to_string())
                        .unwrap_or_else(|| "no debug details".to_string());
                    let failure_message = format!(
                        "GStreamer error from {source}: {} ({details}); verify the required GStreamer plugins and drivers are installed",
                        error.error()
                    );
                    log::error!("{failure_message}");
                    failure_for_bus.record(failure_message);
                }
                gst::MessageView::Warning(warning) => {
                    log::warn!(
                        "GStreamer warning from {source}: {} ({})",
                        warning.error(),
                        warning
                            .debug()
                            .map(|details| details.to_string())
                            .unwrap_or_else(|| "no debug details".to_string())
                    );
                }
                gst::MessageView::StateChanged(state) => {
                    log::debug!(
                        "GStreamer state change from {source}: {:?} -> {:?} (pending {:?})",
                        state.old(),
                        state.current(),
                        state.pending()
                    );
                }
                _ => {}
            }
            gst::BusSyncReply::Pass
        });

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
        appsrc.set_is_live(true);
        appsrc.set_do_timestamp(false);
        appsrc.set_format(gst::Format::Time);

        let replay = if let Some(seconds) = replay_seconds {
            let appsink = pipeline
                .by_name("replay_sink")
                .ok_or(EncodeError::MissingAppSink)?
                .downcast::<gst_app::AppSink>()
                .map_err(|_| EncodeError::MissingAppSink)?;
            let replay = Arc::new(Mutex::new(ReplayBuffer::new(
                seconds,
                options.video_codec.clone(),
                options.output_container,
            )));
            let replay_for_cb = Arc::clone(&replay);
            appsink.set_callbacks(
                gst_app::AppSinkCallbacks::builder()
                    .new_sample(move |sink| {
                        let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                        match replay_for_cb.lock() {
                            Ok(mut replay) => {
                                if let Err(e) = replay.push_sample(&sample) {
                                    log::warn!("failed to store replay packet: {e}");
                                }
                            }
                            Err(e) => log::warn!("replay buffer lock poisoned: {e}"),
                        }
                        Ok(gst::FlowSuccess::Ok)
                    })
                    .build(),
            );
            Some(replay)
        } else {
            None
        };

        if let Err(e) = pipeline.set_state(gst::State::Playing) {
            let message = format!("failed to set Playing: {e:?}");
            log::error!("{message}");
            return Err(EncodeError::Bus(message));
        }
        failure.check()?;

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
                log::info!(
                    "Encoder sink caps before the first frame is pushed: {:?}",
                    caps
                );
            }
        }

        Ok(Self {
            pipeline,
            appsrc,
            options,
            frame_ns: 1_000_000_000u64 / fps as u64,
            next_pts_ns: 0,
            paused: false,
            recording_started: Instant::now(),
            pause_started: None,
            paused_total_ns: 0,
            last_pts_ns: None,
            last_push_wall: None,
            replay,
            failure,
        })
    }

    pub fn new(
        out_path: &str,
        ex: &ExportedDmabuf,
        options: EncoderOptions,
    ) -> Result<Self, EncodeError> {
        Self::new_with_output(EncoderOutput::File(out_path), ex, options)
    }

    fn recording_elapsed_ns(&self) -> u64 {
        let now = Instant::now();
        let paused_now = self
            .pause_started
            .map(|p| now.duration_since(p).as_nanos() as u64)
            .unwrap_or(0);

        self.recording_started.elapsed().as_nanos() as u64 - self.paused_total_ns - paused_now
    }

    pub fn push_frame(&mut self, ex: &ExportedDmabuf) -> Result<(), EncodeError> {
        self.failure.check()?;
        if self.paused {
            return Ok(());
        }

        if matches!(self.options.frame_rate_mode, FrameRateMode::Cfr) {
            let now = Instant::now();

            if let Some(last) = self.last_push_wall {
                let elapsed = now.duration_since(last).as_nanos() as u64;

                // Drop frames that arrive much faster than target fps.
                // Use 80% to allow jitter.
                if elapsed < self.frame_ns * 8 / 10 {
                    return Ok(());
                }
            }

            self.last_push_wall = Some(now);
        }

        let pts_ns = match self.options.frame_rate_mode {
            FrameRateMode::Cfr => {
                let pts = self.next_pts_ns;
                self.next_pts_ns += self.frame_ns;
                pts
            }
            FrameRateMode::Vfr => self.recording_elapsed_ns(),
        };

        let duration = match self.options.frame_rate_mode {
            FrameRateMode::Cfr => Some(self.frame_ns),
            FrameRateMode::Vfr => None,
        };

        if let Some(last) = self.last_pts_ns {
            let delta_ms = (pts_ns.saturating_sub(last)) as f64 / 1_000_000.0;
            log::trace!(
                "push_frame pts={}ms delta={}ms",
                pts_ns / 1_000_000,
                delta_ms
            );
        } else {
            log::trace!("push_frame pts={}ms first", pts_ns / 1_000_000);
        }

        let first_frame = self.last_pts_ns.is_none();
        push_exported_dmabuf(&self.appsrc, ex, pts_ns, duration)?;
        self.failure.check()?;
        self.last_pts_ns = Some(pts_ns);

        if first_frame
            && let Some(enc) = self.pipeline.by_name("enc")
            && let Some(sink_pad) = enc.static_pad("sink")
        {
            log::info!(
                "Encoder sink caps after the first frame was pushed: {:?}",
                sink_pad.current_caps()
            );
        }

        Ok(())
    }

    pub fn request_keyframe(&self, reason: &str) {
        let event = DownstreamForceKeyUnitEvent::builder()
            .all_headers(true)
            .build();
        let sent = self.appsrc.upcast_ref::<gst::Element>().send_event(event);
        if sent {
            log::trace!("Requested force keyframe ({reason})");
        } else {
            log::warn!("Failed to request force keyframe ({reason})");
        }
    }

    pub fn save_replay_buffer(&self, path: &Path) -> Result<(), EncodeError> {
        let replay = self.replay.as_ref().ok_or_else(|| {
            EncodeError::Replay("encoder is not in replay-buffer mode".to_string())
        })?;
        replay
            .lock()
            .map_err(|e| EncodeError::Replay(format!("replay buffer lock poisoned: {e}")))?
            .save_snapshot(path)
    }

    pub fn pause(&mut self) {
        if !self.paused {
            log::info!("Encoder pause");
            self.paused = true;
            self.pause_started = Some(Instant::now());
        }
    }

    pub fn resume(&mut self) {
        if self.paused {
            log::info!("Encoder resume");
            if let Some(p) = self.pause_started.take() {
                self.paused_total_ns += p.elapsed().as_nanos() as u64;
            }
            self.paused = false;
            self.last_push_wall = Some(Instant::now());
            self.request_keyframe("resume");
        }
    }

    pub fn finish(self) -> Result<(), EncodeError> {
        self.failure.check()?;
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
