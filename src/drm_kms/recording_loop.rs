use glow::{HasContext, NativeTexture};
use signal_hook::consts::signal::{SIGINT, SIGTERM, SIGUSR1, SIGUSR2};
use signal_hook::flag as signal_flag;
use std::{
    fs,
    num::NonZero,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use crate::drm_kms::{
    debug, egl_dmabuf_export, gpu_pipeline,
    probe::ProbeSession,
    types::{CaptureOptions, CaptureOutput},
};

use super::egl_context::{
    delete_gl_texture, import_current_capture_texture, init_egl, EglCtx, EglError,
};

pub fn run_capture_session(options: CaptureOptions) -> Result<(), EglError> {
    let EglCtx {
        egl,
        display,
        context,
        ..
    } = init_egl(&options.card_path)?;
    let mut probe_session = ProbeSession::new_with_connector(
        &options.card_path,
        options.connector.clone(),
        options.allow_fallback_connector,
    )
    .map_err(EglError::Probe)?;
    let (texture, source_w, source_h, mut prev_fb_id) =
        import_current_capture_texture(&mut probe_session, &egl, display)?;
    let output_w = options
        .output_width
        .map(|v| v.max(1) as i32)
        .unwrap_or(source_w);
    let output_h = options
        .output_height
        .map(|v| v.max(1) as i32)
        .unwrap_or(source_h);
    if output_w != source_w || output_h != source_h {
        log::info!(
            "Scaling output from {}x{} to {}x{}",
            source_w,
            source_h,
            output_w,
            output_h
        );
    }
    log::info!(
        "Imported EGLImage into GL texture {} from fb {}",
        texture,
        prev_fb_id
    );

    let mut pipelines: Vec<gpu_pipeline::GpuPipeline> = Vec::with_capacity(3);
    for _ in 0..3 {
        pipelines.push(unsafe { gpu_pipeline::GpuPipeline::new(&egl, output_w, output_h) }
            .map_err(EglError::Pipeline)?);
    }

    let cursor_state = gpu_pipeline::CursorState {
        tex: None,
        x: 0.0,
        y: 0.0,
        w: 0.0,
        h: 0.0,
    };

    let first_slot = 0usize;
    let fence = unsafe {
        pipelines[first_slot]
            .render_with_cursor(NativeTexture(NonZero::new(texture).unwrap()), &cursor_state)
    }
    .map_err(EglError::Pipeline)?;
    unsafe {
        let wait = pipelines[first_slot].gl.client_wait_sync(
            fence,
            glow::SYNC_FLUSH_COMMANDS_BIT,
            1_000_000_000,
        );
        if wait == glow::WAIT_FAILED || wait == glow::TIMEOUT_EXPIRED {
            log::warn!("Initial frame GL fence wait returned {}, forcing glFinish()", wait);
            pipelines[first_slot].gl.finish();
        }
        pipelines[first_slot].gl.delete_sync(fence);
        pipelines[first_slot].gl.finish();
    }

    let first_exported = unsafe {
        egl_dmabuf_export::export_rgba_tex_to_dmabuf(
            &egl,
            display,
            context,
            pipelines[first_slot].output_texture().0.into(),
            output_w,
            output_h,
        )
    }
    .map_err(EglError::Export)?;

    log::info!(
        "Exported dmabuf: {}x{} fourcc=0x{:08x} planes={}",
        first_exported.width,
        first_exported.height,
        first_exported.fourcc,
        first_exported.fds.len()
    );

    let fps: u32 = options.fps.max(1);
    let enc_opts = crate::drm_kms::encoder::EncoderOptions {
        fps,
        bitrate_kbps: options.bitrate_kbps,
        frame_rate_mode: options.frame_rate_mode,
        bitrate_mode: options.bitrate_mode,
        quality: options.quality,
        color_range: options.color_range,
        colorimetry: options.colorimetry,
        encoder_backend: options.encoder_backend,
        video_codec: options.video_codec,
    };
    let frame_period = Duration::from_nanos(1_000_000_000u64 / fps as u64);
    let dump_frames = options.dump_frames;
    let dump_every = options.dump_every.max(1);
    if dump_frames {
        fs::create_dir_all(&options.dump_dir)
            .map_err(|e| EglError::Pipeline(format!("failed to create dump dir: {e}")))?;
    }

    let mut encoder = match &options.output {
        CaptureOutput::Preview => crate::drm_kms::encoder::GstEncoder::new_with_output(
            crate::drm_kms::encoder::EncoderOutput::Preview,
            &first_exported,
            enc_opts.clone(),
        )
        .map_err(|e| EglError::Pipeline(e.to_string()))?,
        CaptureOutput::File(path) => crate::drm_kms::encoder::GstEncoder::new(
            &path.to_string_lossy(),
            &first_exported,
            enc_opts.clone(),
        )
        .map_err(|e| EglError::Pipeline(e.to_string()))?,
    };

    let stop_requested = Arc::new(AtomicBool::new(false));
    let pause_req = Arc::new(AtomicBool::new(false));
    let resume_req = Arc::new(AtomicBool::new(false));
    let paused = Arc::new(AtomicBool::new(false));
    {
        let stop_requested = Arc::clone(&stop_requested);
        signal_flag::register(SIGINT, Arc::clone(&stop_requested))
            .map_err(|e| EglError::Pipeline(format!("failed to register SIGINT: {e}")))?;
        signal_flag::register(SIGTERM, stop_requested)
            .map_err(|e| EglError::Pipeline(format!("failed to register SIGTERM: {e}")))?;
    }
    {
        signal_flag::register(SIGUSR1, Arc::clone(&pause_req))
            .map_err(|e| EglError::Pipeline(format!("failed to register SIGUSR1: {e}")))?;
        signal_flag::register(SIGUSR2, Arc::clone(&resume_req))
            .map_err(|e| EglError::Pipeline(format!("failed to register SIGUSR2: {e}")))?;
    }

    let mut next_deadline = Instant::now();
    let mut frame_idx: u64 = 0;
    encoder
        .push_frame(&first_exported)
        .map_err(|e| EglError::Pipeline(e.to_string()))?;
    let _ = delete_gl_texture(&egl, texture);
    frame_idx += 1;

    while !stop_requested.load(Ordering::Relaxed) {
        if pause_req.swap(false, Ordering::Relaxed) {
            paused.store(true, Ordering::Relaxed);
            log::info!("Recording paused (SIGUSR1)");
        }
        if resume_req.swap(false, Ordering::Relaxed) {
            paused.store(false, Ordering::Relaxed);
            log::info!("Recording resumed (SIGUSR2)");
        }
        if paused.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(100));
            continue;
        }

        let (frame_texture, frame_w, frame_h, fb_id) =
            import_current_capture_texture(&mut probe_session, &egl, display)?;
        if frame_w != source_w || frame_h != source_h {
            let _ = delete_gl_texture(&egl, frame_texture);
            return Err(EglError::Pipeline(format!(
                "capture size changed from {}x{} to {}x{} during recording",
                source_w, source_h, frame_w, frame_h
            )));
        }

        let slot = (frame_idx as usize) % pipelines.len();
        let fence = unsafe {
            pipelines[slot].render_with_cursor(
                NativeTexture(NonZero::new(frame_texture).unwrap()),
                &cursor_state,
            )
        }
        .map_err(EglError::Pipeline)?;

        unsafe {
            let wait = pipelines[slot].gl.client_wait_sync(
                fence,
                glow::SYNC_FLUSH_COMMANDS_BIT,
                1_000_000_000,
            );
            if wait == glow::WAIT_FAILED || wait == glow::TIMEOUT_EXPIRED {
                log::warn!(
                    "Frame {} GL fence wait returned {}, forcing glFinish()",
                    frame_idx, wait
                );
                pipelines[slot].gl.finish();
            }
            pipelines[slot].gl.delete_sync(fence);
            pipelines[slot].gl.finish();
        }

        let exported = unsafe {
            egl_dmabuf_export::export_rgba_tex_to_dmabuf(
                &egl,
                display,
                context,
                pipelines[slot].output_texture().0.into(),
                output_w,
                output_h,
            )
        }
        .map_err(EglError::Export)?;

        encoder
            .push_frame(&exported)
            .map_err(|e| EglError::Pipeline(e.to_string()))?;
        let _ = delete_gl_texture(&egl, frame_texture);
        frame_idx += 1;

        if dump_frames && frame_idx % dump_every as u64 == 0 {
            let path = options.dump_dir.join(format!("debug_frame_{:06}.ppm", frame_idx));
            let _ = debug::debug_dump_texture_ppm(
                &egl,
                pipelines[slot].output_texture().0.into(),
                output_w,
                output_h,
                &path.to_string_lossy(),
            );
            log::debug!("Dumped {}", path.to_string_lossy());
        }

        if fb_id != prev_fb_id {
            log::debug!("frame {}: fb changed {} -> {}", frame_idx, prev_fb_id, fb_id);
            prev_fb_id = fb_id;
        } else {
            log::trace!("frame {}: fb unchanged {}", frame_idx, fb_id);
        }
        log::debug!("Captured frame {} (fb {})", frame_idx, fb_id);

        next_deadline += frame_period;
        let now = Instant::now();
        if next_deadline > now {
            thread::sleep(next_deadline - now);
        }
    }

    encoder
        .finish()
        .map_err(|e: crate::drm_kms::encoder::EncodeError| EglError::Pipeline(e.to_string()))?;
    match &options.output {
        CaptureOutput::Preview => log::info!("Preview stopped"),
        CaptureOutput::File(path) => {
            log::info!("Video encoding complete, output saved to {}", path.to_string_lossy())
        }
    }

    Ok(())
}
