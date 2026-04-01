use glow::{HasContext, NativeTexture};
use std::{
    fs,
    num::NonZero,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    path::Path,
    sync::{Arc, atomic::Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::app::signals::CaptureControl;
use crate::drm_kms::{
    debug, egl_dmabuf_export, gpu_pipeline,
    gpu_pipeline::create_default_cursor_texture,
    privd,
    probe::ProbeSession,
    types::{CaptureOptions, CaptureOutput},
};
use crate::shared::mouse_ring::RingBuffer;
use crate::wayland::layer::{TrackingControl, init_wayland};
use crate::wayland::types::MouseTrackRecordingInfo;

use super::egl_context::{
    EglCtx, EglError, delete_gl_texture, import_current_capture_texture, init_egl,
};

#[derive(Default, Clone, Copy)]
struct CursorSmoother {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    tx: f32,
    ty: f32,
    initialized: bool,
}

impl CursorSmoother {
    fn update(
        &mut self,
        target_x: f32,
        target_y: f32,
        dt: f32,
        k: f32,
        d: f32,
        max_speed: f32,
        snap_px: f32,
        smooth_ms: f32,
        deadzone_px: f32,
    ) -> (f32, f32) {
        if !self.initialized {
            self.x = target_x;
            self.y = target_y;
            self.vx = 0.0;
            self.vy = 0.0;
            self.tx = target_x;
            self.ty = target_y;
            self.initialized = true;
            return (self.x, self.y);
        }

        let (tx, ty) = if smooth_ms > 0.0 {
            let tau = (smooth_ms / 1000.0).max(0.001);
            let alpha = 1.0 - (-dt / tau).exp();
            self.tx = self.tx + (target_x - self.tx) * alpha;
            self.ty = self.ty + (target_y - self.ty) * alpha;
            (self.tx, self.ty)
        } else {
            (target_x, target_y)
        };

        let dx = tx - self.x;
        let dy = ty - self.y;
        let dist_sq = dx * dx + dy * dy;
        if deadzone_px > 0.0 && dist_sq <= deadzone_px * deadzone_px {
            // Kill micro jitter while keeping velocity under control.
            self.vx *= 0.25;
            self.vy *= 0.25;
            return (self.x, self.y);
        }
        if snap_px > 0.0 && dist_sq > snap_px * snap_px {
            self.x = tx;
            self.y = ty;
            self.vx = 0.0;
            self.vy = 0.0;
            return (self.x, self.y);
        }

        let fx = k * dx - d * self.vx;
        let fy = k * dy - d * self.vy;
        self.vx += fx * dt;
        self.vy += fy * dt;

        let speed_sq = self.vx * self.vx + self.vy * self.vy;
        if max_speed > 0.0 && speed_sq > max_speed * max_speed {
            let inv = max_speed / speed_sq.sqrt();
            self.vx *= inv;
            self.vy *= inv;
        }

        self.x += self.vx * dt;
        self.y += self.vy * dt;
        (self.x, self.y)
    }
}

struct MouseTrackingWorker {
    stop_requested: Arc<std::sync::atomic::AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

fn create_cursor_texture_from_png(
    gl: &glow::Context,
    path: &Path,
) -> Result<(glow::NativeTexture, f32, f32), String> {
    let img = image::open(path)
        .map_err(|e| format!("failed to load cursor sprite {}: {e}", path.display()))?
        .to_rgba8();
    let w = img.width() as i32;
    let h = img.height() as i32;
    if w <= 0 || h <= 0 {
        return Err(format!(
            "invalid cursor sprite dimensions for {}",
            path.display()
        ));
    }

    unsafe {
        let tex = gl.create_texture().map_err(|e| e.to_string())?;
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            w,
            h,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(img.as_raw())),
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_S,
            glow::CLAMP_TO_EDGE as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_T,
            glow::CLAMP_TO_EDGE as i32,
        );
        Ok((tex, w as f32, h as f32))
    }
}

impl Drop for MouseTrackingWorker {
    fn drop(&mut self) {
        self.stop_requested.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            log::info!("Waiting for mouse tracking thread to stop");
            match handle.join() {
                Ok(()) => log::debug!("Mouse tracking thread stopped"),
                Err(_) => log::warn!("Mouse tracking thread join failed"),
            }
        }
    }
}

pub fn run_capture_session(
    options: CaptureOptions,
    control: CaptureControl,
) -> Result<(), EglError> {
    log::info!("capture session start: card={} connector={:?} fps={}", options.card_path, options.connector, options.fps);
    let use_mouse_tracking = options.mouse_tracking || options.cursor_composition;
    let mut privd_session = privd::acquire_device_fds(&options.card_path, use_mouse_tracking)
        .map_err(|e| EglError::Pipeline(format!("failed to acquire device fds from privd: {e}")))?;
    let input_fds_for_tracker = if use_mouse_tracking {
        Some(std::mem::take(&mut privd_session.input_fds))
    } else {
        None
    };
    let drm_fd = dup_fd(privd_session.drm_fd.as_raw_fd())
        .map_err(|e| EglError::Pipeline(format!("failed to dup drm fd from privd: {e}")))?;

    let mouse_ring: Option<Arc<RingBuffer>> = if use_mouse_tracking {
        Some(Arc::new(RingBuffer::new(4096)))
    } else {
        None
    };

    let _mouse_tracking_worker = if use_mouse_tracking {
        if options.cursor_composition && !options.mouse_tracking {
            log::info!(
                "Cursor composition requested without --mouse-tracking; enabling internal mouse tracking automatically"
            );
        }
        let tracking_path = options.mouse_tracking_file.clone();
        let sync_frequency_hz = options.wayland_sync_frequency;
        let started_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis().min(u64::MAX as u128) as u64)
            .unwrap_or(0);
        let recording_info = MouseTrackRecordingInfo {
            started_unix_ms,
            output_path: match &options.output {
                CaptureOutput::File(path) => Some(path.to_string_lossy().into_owned()),
                CaptureOutput::Preview => None,
            },
            card_path: Some(options.card_path.clone()),
            connector: options.connector.clone(),
            fps: Some(options.fps),
            width: options.output_width,
            height: options.output_height,
            encoder_backend: Some(options.encoder_backend.to_string()),
            video_codec: Some(options.video_codec.to_string()),
        };
        let tracking_control =
            TrackingControl::new(control.stop_requested.clone(), control.paused.clone());
        let ring_clone = mouse_ring.clone();
        let preopened_input_fds = input_fds_for_tracker;
        let handle = thread::spawn(move || {
            if let Err(e) = init_wayland(
                sync_frequency_hz,
                &tracking_path,
                tracking_control,
                recording_info,
                ring_clone,
                preopened_input_fds,
            ) {
                log::error!("Failed to initialize Wayland mouse tracking: {e}");
            }
        });
        Some(MouseTrackingWorker {
            stop_requested: control.stop_requested.clone(),
            handle: Some(handle),
        })
    } else {
        None
    };

    log::debug!("initializing EGL context");
    let EglCtx {
        egl,
        display,
        context,
        ..
    } = init_egl(&options.card_path).map_err(|e| {
        log::error!("init_egl failed: {}", e);
        e
    })?;
    log::debug!("EGL context initialized");
    log::debug!("creating probe session from privd drm fd");
    let mut probe_session = ProbeSession::new_with_card(
        crate::drm_kms::types::Card::from_owned_fd(drm_fd),
        options.connector.clone(),
        options.allow_fallback_connector,
    )
    .map_err(|e| {
        log::error!("probe session creation failed: {}", e);
        EglError::Probe(e)
    })?;
    log::debug!("probe session ready, importing first texture");
    let (texture, source_w, source_h, mut prev_fb_id) =
        import_current_capture_texture(&mut probe_session, &mut privd_session, &egl, display).map_err(|e| {
            log::error!("initial frame import failed: {}", e);
            e
        })?;
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

    let fps: u32 = options.fps.max(1);
    let enc_opts = crate::encode::EncoderOptions {
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

    let inflight_slots = crate::encode::recommended_slots(&enc_opts).max(3);
    log::info!("Using {} in-flight render surfaces", inflight_slots);
    let mut pipelines: Vec<gpu_pipeline::GpuPipeline> = Vec::with_capacity(inflight_slots);
    for _ in 0..inflight_slots {
        pipelines.push(
            unsafe { gpu_pipeline::GpuPipeline::new(&egl, output_w, output_h) }
                .map_err(EglError::Pipeline)?,
        );
    }

    let cursor_state_empty = gpu_pipeline::CursorState::empty();
    let mut cursor_smoother = CursorSmoother::default();
    let (cursor_tex, cursor_w, cursor_h, hotspot_x, hotspot_y) = if options.cursor_composition {
        let (tex, base_w, base_h, auto_hotspot) =
            if let Some(sprite_path) = options.cursor_sprite.as_ref() {
                let (tex, w, h) = create_cursor_texture_from_png(&pipelines[0].gl, sprite_path)
                    .map_err(EglError::Pipeline)?;
                // Large cursor atlases are commonly centered with transparent borders.
                // let auto_hotspot = if options.cursor_hotspot_x == 0 && options.cursor_hotspot_y == 0 {
                //     Some((w * 0.5_f32, h * 0.5_f32))
                // } else {
                //     None
                // };
                log::info!(
                    "Cursor composition enabled, using custom sprite {} ({}x{})",
                    sprite_path.display(),
                    w,
                    h
                );
                (tex, w, h, Some((0.0, 0.0)))
            } else {
                let tex = create_default_cursor_texture(&pipelines[0].gl)
                    .map_err(EglError::Pipeline)?;
                log::info!("Cursor composition enabled, created default cursor texture");
                (tex, 24.0_f32, 24.0_f32, None)
            };

        let scale = options.cursor_scale.max(0.1);
        let out_w = base_w * scale;
        let out_h = base_h * scale;
        let (hotspot_x, hotspot_y) = if let Some((ax, ay)) = auto_hotspot {
            log::info!(
                "Auto hotspot enabled for custom sprite: ({:.1}, {:.1}) before scale",
                ax,
                ay
            );
            (ax * scale, ay * scale)
        } else {
            (
                (options.cursor_hotspot_x as f32 * scale).max(0.0),
                (options.cursor_hotspot_y as f32 * scale).max(0.0),
            )
        };
        (Some(tex), out_w, out_h, hotspot_x, hotspot_y)
    } else {
        (None, 0.0, 0.0, 0.0, 0.0)
    };

    let first_slot = 0usize;
    let fence = unsafe {
        pipelines[first_slot].render_with_cursor(
            NativeTexture(NonZero::new(texture).unwrap()),
            &cursor_state_empty,
        )
    }
    .map_err(EglError::Pipeline)?;
    unsafe {
        let wait = pipelines[first_slot].gl.client_wait_sync(
            fence,
            glow::SYNC_FLUSH_COMMANDS_BIT,
            1_000_000_000,
        );
        if wait == glow::WAIT_FAILED || wait == glow::TIMEOUT_EXPIRED {
            log::warn!(
                "Initial frame GL fence wait returned {}, forcing glFinish()",
                wait
            );
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

    let frame_period = Duration::from_nanos(1_000_000_000u64 / fps as u64);
    let dump_frames = options.dump_frames;
    let dump_every = options.dump_every.max(1);
    if dump_frames {
        fs::create_dir_all(&options.dump_dir)
            .map_err(|e| EglError::Pipeline(format!("failed to create dump dir: {e}")))?;
    }

    let mut encoder = match &options.output {
        CaptureOutput::Preview => crate::encode::GstEncoder::new_with_output(
            crate::encode::EncoderOutput::Preview,
            &first_exported,
            enc_opts.clone(),
        )
        .map_err(|e| EglError::Pipeline(e.to_string()))?,
        CaptureOutput::File(path) => crate::encode::GstEncoder::new(
            &path.to_string_lossy(),
            &first_exported,
            enc_opts.clone(),
        )
        .map_err(|e| EglError::Pipeline(e.to_string()))?,
    };

    let mut next_deadline = Instant::now();
    let mut frame_idx: u64 = 0;
    let mut last_forced_keyframe_frame: u64 = 0;
    let mut last_cursor_update = Instant::now();
    encoder
        .push_frame(&first_exported)
        .map_err(|e| EglError::Pipeline(e.to_string()))?;
    let _ = delete_gl_texture(&egl, texture);
    frame_idx += 1;

    while !control.stop_requested.load(Ordering::Relaxed) {
        if control.pause_req.swap(false, Ordering::Relaxed) {
            control.paused.store(true, Ordering::Relaxed);
            log::info!("Recording paused (SIGUSR1)");
        }
        if control.resume_req.swap(false, Ordering::Relaxed) {
            control.paused.store(false, Ordering::Relaxed);
            log::info!("Recording resumed (SIGUSR2)");
            encoder.request_keyframe("resume");
            last_forced_keyframe_frame = frame_idx;
        }
        if control.paused.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(100));
            continue;
        }

        let (frame_texture, frame_w, frame_h, fb_id) =
            import_current_capture_texture(&mut probe_session, &mut privd_session, &egl, display)?;
        if frame_w != source_w || frame_h != source_h {
            let _ = delete_gl_texture(&egl, frame_texture);
            return Err(EglError::Pipeline(format!(
                "capture size changed from {}x{} to {}x{} during recording",
                source_w, source_h, frame_w, frame_h
            )));
        }

        let cursor_state =
            if let (Some(ring), Some(ctex)) = (mouse_ring.as_ref(), cursor_tex.as_ref()) {
                if let Some(event) = ring.latest_before(u64::MAX) {
                    log::trace!(
                        "Frame {}: latest mouse at ({:.1}, {:.1})",
                        frame_idx,
                        event.x,
                        event.y
                    );
                    // account for fractional scaling
                    let (mx, my) = if let (Some(max_x), Some(max_y)) = (event.max_x, event.max_y) {
                        if max_x > 0.0 && max_y > 0.0 {
                            (
                                ((event.x / max_x).clamp(0.0, 1.0) * output_w as f64) as f32,
                                ((event.y / max_y).clamp(0.0, 1.0) * output_h as f64) as f32,
                            )
                        } else {
                            let scale_x = output_w as f64 / source_w as f64;
                            let scale_y = output_h as f64 / source_h as f64;
                            ((event.x * scale_x) as f32, (event.y * scale_y) as f32)
                        }
                    } else {
                        let scale_x = output_w as f64 / source_w as f64;
                        let scale_y = output_h as f64 / source_h as f64;
                        ((event.x * scale_x) as f32, (event.y * scale_y) as f32)
                    };
                    // Tracker coordinates are already top-left oriented in output space.
                    let min_x = (-cursor_w + 1.0).min(0.0);
                    let min_y = (-cursor_h + 1.0).min(0.0);
                    let max_x = (output_w as f32 - 1.0).max(min_x);
                    let max_y = (output_h as f32 - 1.0).max(min_y);
                    let cursor_x = (mx - hotspot_x).clamp(min_x, max_x);
                    let cursor_y = (my - hotspot_y).clamp(min_y, max_y);
                    let now = Instant::now();
                    let dt = (now - last_cursor_update).as_secs_f32().clamp(0.0, 0.05);
                    last_cursor_update = now;
                    let (s_cursor_x, s_cursor_y) = if options.cursor_smooth {
                        cursor_smoother.update(
                            cursor_x,
                            cursor_y,
                            dt,
                            options.cursor_spring_k,
                            options.cursor_spring_d,
                            options.cursor_max_speed,
                            options.cursor_snap_px,
                            options.cursor_smooth_ms,
                            options.cursor_deadzone_px,
                        )
                    } else {
                        (cursor_x, cursor_y)
                    };
                    gpu_pipeline::CursorState::with_position(
                        *ctex, s_cursor_x, s_cursor_y, cursor_w, cursor_h,
                    )
                } else {
                    cursor_state_empty.clone()
                }
            } else {
                cursor_state_empty.clone()
            };

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
                    frame_idx,
                    wait
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
            let path = options
                .dump_dir
                .join(format!("debug_frame_{:06}.ppm", frame_idx));
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
            log::trace!(
                "frame {}: fb changed {} -> {}",
                frame_idx,
                prev_fb_id,
                fb_id
            );
            prev_fb_id = fb_id;
        } else {
            log::trace!("frame {}: fb unchanged {}", frame_idx, fb_id);
        }

        if frame_idx.saturating_sub(last_forced_keyframe_frame) >= (fps as u64).saturating_mul(2) {
            encoder.request_keyframe("periodic");
            last_forced_keyframe_frame = frame_idx;
        }
        log::trace!("Captured frame {} (fb {})", frame_idx, fb_id);

        next_deadline += frame_period;
        let now = Instant::now();
        if next_deadline > now {
            thread::sleep(next_deadline - now);
        }
    }

    delete_gl_texture(&egl, texture)
        .map_err(|e| EglError::Pipeline(format!("failed to delete capture texture: {e}")))?;
    if let Some(ctex) = cursor_tex {
        delete_gl_texture(&egl, ctex.0.into())
            .map_err(|e| EglError::Pipeline(format!("failed to delete cursor texture: {e}")))?;
    }

    encoder
        .finish()
        .map_err(|e: crate::encode::EncodeError| EglError::Pipeline(e.to_string()))?;

    let mouse_file = if use_mouse_tracking {
        options.mouse_tracking_file.to_string_lossy().into_owned()
    } else {
        "disabled".to_string()
    };
    log::info!(
        "Shutdown summary: frames_emitted={} mouse_tracking_file={}",
        frame_idx,
        mouse_file
    );

    match &options.output {
        CaptureOutput::Preview => log::info!("Preview stopped"),
        CaptureOutput::File(path) => {
            log::info!(
                "Video encoding complete, output saved to {}",
                path.to_string_lossy()
            )
        }
    }

    Ok(())
}

fn dup_fd(raw_fd: i32) -> std::io::Result<OwnedFd> {
    let dup_fd = unsafe { libc::fcntl(raw_fd, libc::F_DUPFD_CLOEXEC, 0) };
    if dup_fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(dup_fd) })
}
