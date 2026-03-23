use std::path::PathBuf;

use crate::drm_kms::types::{BitrateMode, EncoderBackend, VideoCodec};
use crate::encode::EncoderOptions;
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
    pub transcode: PostFxTranscodeConfig,
}

#[derive(Debug, Clone)]
pub struct PostFxTranscodeConfig {
    pub decode_backend: EncoderBackend,
    pub decode_codec: Option<VideoCodec>,
    pub encode: EncoderOptions,
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

fn decode_caps_for_backend(backend: &EncoderBackend) -> &'static str {
    match backend {
        EncoderBackend::Vaapi => "video/x-raw(memory:VAMemory),format=NV12",
        // On Linux, qsv decoders often expose plain video/x-raw NV12 on src pad.
        EncoderBackend::Qsv => "video/x-raw,format=NV12",
        EncoderBackend::Vulkan => "video/x-raw(memory:VulkanImage),format=NV12",
        EncoderBackend::Cpu => "video/x-raw,format=RGBA",
    }
}

fn decode_codec(cfg: &PostFxTranscodeConfig) -> VideoCodec {
    cfg.decode_codec
        .clone()
        .unwrap_or_else(|| cfg.encode.video_codec.clone())
}

fn demux_for_path(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".mkv") || lower.ends_with(".webm") {
        "matroskademux"
    } else if lower.ends_with(".ts") || lower.ends_with(".m2ts") {
        "tsdemux"
    } else {
        "qtdemux"
    }
}

fn parser_for_codec(codec: &VideoCodec) -> &'static str {
    match codec {
        VideoCodec::H264 => "h264parse",
        VideoCodec::H265 => "h265parse",
        VideoCodec::Av1 => "av1parse",
    }
}

fn decoder_for_backend_codec(backend: &EncoderBackend, codec: &VideoCodec) -> &'static str {
    match (backend, codec) {
        (EncoderBackend::Vaapi, VideoCodec::H264) => "vah264dec",
        (EncoderBackend::Vaapi, VideoCodec::H265) => "vah265dec",
        (EncoderBackend::Vaapi, VideoCodec::Av1) => "vaav1dec",
        (EncoderBackend::Qsv, VideoCodec::H264) => "qsvh264dec",
        (EncoderBackend::Qsv, VideoCodec::H265) => "qsvh265dec",
        (EncoderBackend::Qsv, VideoCodec::Av1) => "qsvav1dec",
        (EncoderBackend::Vulkan, VideoCodec::H264) => "vulkanh264dec",
        (EncoderBackend::Vulkan, VideoCodec::H265) => "vulkanh265dec",
        (EncoderBackend::Vulkan, VideoCodec::Av1) => "vulkanav1dec",
        (EncoderBackend::Cpu, VideoCodec::H264) => "avdec_h264",
        (EncoderBackend::Cpu, VideoCodec::H265) => "avdec_h265",
        (EncoderBackend::Cpu, VideoCodec::Av1) => "avdec_av1",
    }
}

fn build_decode_pipeline(path: &str, cfg: &PostFxTranscodeConfig, cpu_fallback: bool) -> String {
    let backend = if cpu_fallback {
        EncoderBackend::Cpu
    } else {
        cfg.decode_backend.clone()
    };
    let codec = decode_codec(cfg);
    let demux = demux_for_path(path);
    let parser = parser_for_codec(&codec);
    let decoder = decoder_for_backend_codec(&backend, &codec);
    let caps = decode_caps_for_backend(&backend);

    if matches!(backend, EncoderBackend::Cpu) {
        return format!(
            "filesrc location={} ! {} name=demux demux. ! queue ! {} ! {} ! videoconvert ! video/x-raw,format=RGBA ! appsink name=out sync=false",
            path, demux, parser, decoder
        );
    }

    format!(
        "filesrc location={} ! {} name=demux demux. ! queue ! {} ! {} ! {} ! appsink name=out sync=false",
        path, demux, parser, decoder, caps
    )
}

fn encode_element_name(backend: &EncoderBackend, codec: &VideoCodec) -> &'static str {
    match (backend, codec) {
        (EncoderBackend::Vaapi, VideoCodec::H264) => "vah264enc",
        (EncoderBackend::Vaapi, VideoCodec::H265) => "vah265enc",
        (EncoderBackend::Vaapi, VideoCodec::Av1) => "vaav1enc",
        (EncoderBackend::Qsv, VideoCodec::H264) => "qsvh264enc",
        (EncoderBackend::Qsv, VideoCodec::H265) => "qsvh265enc",
        (EncoderBackend::Qsv, VideoCodec::Av1) => "qsvav1enc",
        (EncoderBackend::Vulkan, VideoCodec::H264) => "vulkanh264enc",
        (EncoderBackend::Vulkan, VideoCodec::H265) => "vulkanh265enc",
        (EncoderBackend::Vulkan, VideoCodec::Av1) => "vulkanav1enc",
        (EncoderBackend::Cpu, VideoCodec::H264) => "x264enc",
        (EncoderBackend::Cpu, VideoCodec::H265) => "x265enc",
        (EncoderBackend::Cpu, VideoCodec::Av1) => "av1enc",
    }
}

fn build_reencode_hint(cfg: &PostFxTranscodeConfig) -> String {
    let enc = encode_element_name(&cfg.encode.encoder_backend, &cfg.encode.video_codec);
    let rc = match cfg.encode.bitrate_mode {
        BitrateMode::Default => "default".to_string(),
        _ => cfg.encode.bitrate_mode.to_string(),
    };
    format!(
        "{} bitrate={} rate-control={} fps={} color-range={} colorimetry={}",
        enc,
        cfg.encode.bitrate_kbps,
        rc,
        cfg.encode.fps,
        cfg.encode.color_range.to_string(),
        cfg.encode.colorimetry.to_string()
    )
}

pub fn decode_loop(
    path: &str,
    _mouse_track: &MouseTrackFile,
    cfg: &PostFxTranscodeConfig,
) -> Result<(), PostFxError> {
    gst::init()?;
    let desc = build_decode_pipeline(path, cfg, false);
    log::debug!("[Postfx] explicit decode pipeline: {}", desc);
    let element = match gst::parse::launch(&desc) {
        Ok(e) => e,
        Err(e) => {
            if matches!(cfg.decode_backend, EncoderBackend::Cpu) {
                return Err(PostFxError::Other(format!(
                    "failed to launch decode pipeline: {e}"
                )));
            }
            let fallback = build_decode_pipeline(path, cfg, true);
            log::warn!(
                "[Postfx] explicit GPU decode launch failed ({}), falling back to CPU: {}",
                e,
                fallback
            );
            gst::parse::launch(&fallback)?
        }
    };
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
                let pts_ns = buffer.pts().unwrap_or(gst::ClockTime::ZERO).nseconds();
                log::info!(
                    "[Postfx] Got frame with PTS: {} ns, size: {} bytes, memories={}",
                    pts_ns,
                    buffer.size(),
                    buffer.n_memory()
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
        "Applying postfx with setup: input={} output={} samples={} effect={:?} decode_backend={:?} decode_codec={:?}",
        setup.input.display(),
        setup.output.display(),
        track.samples.len(),
        setup.mouse.effect,
        setup.transcode.decode_backend,
        setup.transcode.decode_codec
    );
    log::info!("[Postfx] reencode hint: {}", build_reencode_hint(&setup.transcode));
    decode_loop(&setup.input.to_string_lossy(), &track, &setup.transcode)
        .map_err(|e| eyre::eyre!(e))
}
