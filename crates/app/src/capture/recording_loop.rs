use glow::{HasContext, NativeTexture};
use std::{
    fs, num::NonZero, ops::Div as _, path::{Path, PathBuf}, sync::{Arc, atomic::Ordering}, thread, time::{Duration, Instant, SystemTime, UNIX_EPOCH}
};

use crate::{app::signals::CaptureControl, drm_kms::types::Profile, portal::notifs::ProcessState};
use crate::capture::backend::CaptureBackend;
use crate::cursor::cursor::*;
use crate::drm_kms::{
    debug, egl_dmabuf_export, gpu_pipeline,
    gpu_pipeline::create_default_cursor_texture,
    types::{CaptureOptions, CaptureOutput, LiveSettings},
};
use crate::shared::mouse_ring::RingBuffer;
use crate::wayland::layer::{TrackingControl, init_wayland};
use crate::wayland::types::MouseTrackRecordingInfo;

use crate::drm_kms::egl_context::{
    EglCtx, EglError, delete_gl_texture, import_capture_frame_texture, init_egl,
};

pub fn run_capture_session(
    options: CaptureOptions,
    control: CaptureControl,
    backend: &mut dyn CaptureBackend,
) -> Result<(), EglError> {
    log::info!(
        "capture session start: card={} connector={:?} fps={}",
        options.card_path,
        options.connector,
        options.fps
    );
    let use_mouse_tracking =
        (options.cursor_composition)
            && !matches!(options.output, CaptureOutput::EmbeddedPreview);
    let input_fds_for_tracker = if use_mouse_tracking {
        backend.take_input_fds()
    } else {
        None
    };

    let mouse_ring: Option<Arc<RingBuffer>> = if use_mouse_tracking {
        Some(Arc::new(RingBuffer::new(512)))
    } else {
        None
    };

    let _mouse_tracking_worker = if use_mouse_tracking {
        // if options.cursor_composition && !options.mouse_tracking {
        //     log::info!(
        //         "Cursor composition requested without --mouse-tracking; enabling internal mouse tracking automatically"
        //     );
        // }
        // todo: remove this deprecated option
        let tracking_path = PathBuf::from("/tmp/openstudio-mouse-tracking.bitcode");
        let sync_frequency_hz = options.wayland_sync_frequency;
        let started_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis().min(u64::MAX as u128) as u64)
            .unwrap_or(0);
        let recording_info = MouseTrackRecordingInfo {
            started_unix_ms,
            output_path: match &options.output {
                // CaptureOutput::File(path) => Some(path.to_string_lossy().into_owned()),
                // disabling this for now, since we don't really have a use case for it yet
                CaptureOutput::File(_) => None,
                CaptureOutput::Preview => None,
                CaptureOutput::EmbeddedPreview => None,
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
    backend.on_egl_ready(&egl, display)?;
    let pstate = match options.output {
        CaptureOutput::File(_) => crate::portal::notifs::ProcessState::Running,
        CaptureOutput::Preview => crate::portal::notifs::ProcessState::Preview,
        CaptureOutput::EmbeddedPreview => crate::portal::notifs::ProcessState::Preview,
    };
    let _ = backend.send_notification(pstate, 1);
    let mut first_frame = None;
    while !control.stop_requested.load(Ordering::Relaxed) {
        match backend.next_frame(Duration::from_millis(100)) {
            Ok(Some(frame)) => {
                first_frame = Some(frame);
                break;
            }
            Ok(None) => continue,
            Err(e) => {
                log::error!("initial backend frame acquisition failed: {}", e);
                return Err(e);
            }
        }
    }
    let first_frame = first_frame.ok_or_else(|| EglError::Pipeline("Capture stopped before first frame".to_string()))?;
    log::debug!("backend returned first frame, importing texture");
    let (texture, initial_source_w, initial_source_h, mut prev_fb_id, first_use_external_texture) =
        import_capture_frame_texture(first_frame, &egl, display).map_err(|e| {
            log::error!("initial frame import failed: {}", e);
            e
        })?;
    let mut source_w = initial_source_w;
    let mut source_h = initial_source_h;
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
    let fallback_live: std::sync::Arc<LiveSettings> =
        std::sync::Arc::new(LiveSettings::from_options(&options));
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
        profile: options.profile,
    };

    let preview_mailbox = if matches!(options.output, CaptureOutput::EmbeddedPreview) {
        options
            .preview_mailbox
            .clone()
            .or_else(|| Some(common::types::PreviewMailbox::new()))
    } else {
        None
    };

    let inflight_slots = crate::encode::recommended_slots(&enc_opts).max(3);
    log::info!("Using {} in-flight render surfaces", inflight_slots);
    let mut pipelines: Vec<gpu_pipeline::GpuPipeline> = Vec::with_capacity(inflight_slots);
    for _ in 0..inflight_slots {
        pipelines.push(
            unsafe { gpu_pipeline::GpuPipeline::new(&egl, output_w, output_h, options.profile.unwrap_or(Profile::Sdr)) }
                .map_err(EglError::Pipeline)?,
        );
    }

    let cursor_state_empty = gpu_pipeline::CursorState::empty();
    let mut cursor_smoother = CursorSmoother::default();
    let (cursor_tex, cursor_w, cursor_h, hotspot_x, hotspot_y) = if options.cursor_composition {
        let (tex, base_w, base_h, auto_hotspot) =
            if let Some(sprite_path) = options.cursor_sprite.as_ref() {
                let (tex, w, h) = load_rgba_texture(&pipelines[0].gl, sprite_path)
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
                let tex =
                    create_default_cursor_texture(&pipelines[0].gl).map_err(EglError::Pipeline)?;
                log::info!("Cursor composition enabled, created default cursor texture");
                (tex, 24.0_f32, 24.0_f32, None)
            };

        let scale = options.cursor_scale.clamp(1.0, 100.0).div(100.0);
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
    // Allow live-reloading the cursor sprite and hotspot via LiveSettings.
    let (mut cursor_tex, mut cursor_w, mut cursor_h) = (cursor_tex, cursor_w, cursor_h);

    let first_slot = 0usize;
    let fence = unsafe {
        pipelines[first_slot].render_with_cursor(
            NativeTexture(NonZero::new(texture).unwrap()),
            first_use_external_texture,
            &cursor_state_empty,
            None,  // no background yet
            1.0,   // no frame zoom
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

    let first_exported = if matches!(options.output, CaptureOutput::EmbeddedPreview) {
        None
    } else {
        let ex = unsafe {
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
            ex.width,
            ex.height,
            ex.fourcc,
            ex.fds.len()
        );
        Some(ex)
    };

    let frame_period = Duration::from_nanos(1_000_000_000u64 / fps as u64);
    let dump_frames = options.dump_frames;
    let dump_every = options.dump_every.max(1);
    if dump_frames {
        fs::create_dir_all(&options.dump_dir)
            .map_err(|e| EglError::Pipeline(format!("failed to create dump dir: {e}")))?;
    }

    let mut encoder = match (&options.output, first_exported.as_ref()) {
        (CaptureOutput::EmbeddedPreview, _) => None,
        (CaptureOutput::Preview, Some(ex)) => Some(
            crate::encode::GstEncoder::new_with_output(
                crate::encode::EncoderOutput::Preview,
                ex,
                enc_opts.clone(),
            )
            .map_err(|e| EglError::Pipeline(e.to_string()))?,
        ),
        (CaptureOutput::File(path), Some(ex)) => Some(
            crate::encode::GstEncoder::new(&path.to_string_lossy(), ex, enc_opts.clone())
                .map_err(|e| EglError::Pipeline(e.to_string()))?,
        ),
        _ => {
            return Err(EglError::Pipeline(
                "missing first exported dmabuf for encoder init".to_string(),
            ));
        }
    };

    let mut next_deadline = Instant::now();
    let mut frame_idx: u64 = 0;
    let mut last_forced_keyframe_frame: u64 = 0;
    let mut last_cursor_update = Instant::now();
    let mut last_cursor_sample: Option<(f32, f32, Instant)> = None;
    if let (Some(enc), Some(ex)) = (encoder.as_mut(), first_exported.as_ref()) {
        enc.push_frame(ex)
            .map_err(|e| EglError::Pipeline(e.to_string()))?;
    }
    let _ = delete_gl_texture(&egl, texture);
    frame_idx += 1;

    // Version counters for live-reloadable assets.  Using u64::MAX as the initial
    // sentinel guarantees the first frame always triggers a load, which correctly
    // handles both the CLI case (initial --background / --cursor-sprite from
    // CaptureOptions) and the GUI case (mailbox provides the real version 0).
    let mut last_cursor_sprite_version: u64 = u64::MAX;
    let mut last_bg_version: u64 = u64::MAX;
    // Background texture handle (None until a path is resolved on first frame).
    let mut live_bg_tex: Option<glow::NativeTexture> = None;

    while !control.stop_requested.load(Ordering::Relaxed) {
        // Snapshot live-mutable settings once at the top of each iteration.
        let live: std::sync::Arc<LiveSettings> = options
            .live_settings
            .as_ref()
            .map(|m| m.get())
            .unwrap_or_else(|| std::sync::Arc::clone(&fallback_live));

        // Recompute frame period from the live fps
        let frame_period = Duration::from_nanos(1_000_000_000u64 / live.fps.max(1) as u64);

        if cursor_tex.is_some() && live.cursor_sprite_version != last_cursor_sprite_version {
            if let Some(old_tex) = cursor_tex.take() {
                let _ = delete_gl_texture(&egl, old_tex.0.into());
            }
            // todo: allow 3x scale for custom cursors
            let scale = live.cursor_scale.clamp(1.0, 100.0).div(100.0);
            let result = match live.cursor_sprite.as_deref() {
                Some(path) => load_rgba_texture(&pipelines[0].gl, path)
                    .map(|(t, w, h)| (Some(t), w * scale, h * scale)),
                None => load_rgba_texture(&pipelines[0].gl, Path::new("assets/default_dark.png"))
                    .map(|(t, w, h)| (Some(t), w * 2.0 * scale, h * 2.0 * scale)),
            };
            match result {
                Ok((t, w, h)) => {
                    cursor_tex = t;
                    cursor_w = w;
                    cursor_h = h;
                    log::info!(
                        "Live cursor sprite reloaded: {:?} ({}x{})",
                        live.cursor_sprite.as_deref().unwrap_or(std::path::Path::new("<default>")),
                        w, h
                    );
                }
                Err(e) => log::warn!("Failed to reload cursor sprite: {}", e),
            }
            last_cursor_sprite_version = live.cursor_sprite_version;
        }

        if live.background_version != last_bg_version {
            if let Some(old_bg) = live_bg_tex.take() {
                let _ = delete_gl_texture(&egl, old_bg.0.into());
            }
            live_bg_tex = live.background.as_deref().and_then(|path| {
                match load_rgba_texture(&pipelines[0].gl, path) {
                    Ok((t, _, _)) => {
                        log::info!("Background image loaded: {}", path.display());
                        Some(t)
                    }
                    Err(e) => {
                        log::warn!("Failed to load background {}: {}", path.display(), e);
                        None
                    }
                }
            });
            last_bg_version = live.background_version;
        }
        if control.pause_req.swap(false, Ordering::Relaxed) {
            control.paused.store(true, Ordering::Relaxed);
            let _ = backend.send_notification(ProcessState::Paused, 2);
            log::info!("Recording paused (SIGUSR1)");
        }
        if control.resume_req.swap(false, Ordering::Relaxed) {
            control.paused.store(false, Ordering::Relaxed);
            log::info!("Recording resumed (SIGUSR2)");
            if let Some(enc) = encoder.as_mut() {
                enc.request_keyframe("resume");
            }
            last_forced_keyframe_frame = frame_idx;
        }
        if control.paused.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(100));
            continue;
        }

        let frame = match backend.next_frame(Duration::from_millis(100)) {
            Ok(Some(f)) => f,
            Ok(None) => {
                if control.stop_requested.load(Ordering::Relaxed) {
                    break;
                }
                continue;
            }
            Err(e) => {
                if control.stop_requested.load(Ordering::Relaxed) {
                    log::info!("Capture stopping; ignoring late frame error: {}", e);
                    break;
                }
                return Err(e);
            }
        };

        let (frame_texture, frame_w, frame_h, fb_id, use_external_texture) = match import_capture_frame_texture(frame, &egl, display)
        {
            Ok(v) => v,
            Err(e) => {
                if control.stop_requested.load(Ordering::Relaxed) {
                    log::info!("Capture stopping; ignoring late frame import error: {}", e);
                    break;
                }
                return Err(e);
            }
        };
        if frame_w != source_w || frame_h != source_h {
            log::warn!(
                "capture source size changed from {}x{} to {}x{}; keeping encoder output at {}x{}",
                source_w,
                source_h,
                frame_w,
                frame_h,
                output_w,
                output_h
            );
            source_w = frame_w;
            source_h = frame_h;
        }

        let cursor_state = if let (Some(ring), Some(ctex)) =
            (mouse_ring.as_ref(), cursor_tex.as_ref())
        {
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
                let (s_cursor_x, s_cursor_y) = if live.cursor_smooth {
                    cursor_smoother.update(
                        cursor_x,
                        cursor_y,
                        dt,
                        live.cursor_spring_k,
                        live.cursor_spring_d,
                        live.cursor_max_speed,
                        live.cursor_snap_px,
                        live.cursor_smooth_ms,
                        live.cursor_deadzone_px,
                    )
                } else {
                    (cursor_x, cursor_y)
                };
                let mut taps: Vec<[f32; 3]> = Vec::with_capacity(8);
                taps.push([s_cursor_x, s_cursor_y, 1.0]);
                let mut motion_dir_x = 1.0f32;
                let mut motion_dir_y = 0.0f32;
                let mut motion_stretch = 1.0f32;
                let mut motion_squash = 1.0f32;

                if live.cursor_smear {
                    if let Some((prev_x, prev_y, prev_t)) = last_cursor_sample {
                        let vdt = (now - prev_t).as_secs_f32().max(1e-4);
                        let vx = (s_cursor_x - prev_x) / vdt;
                        let vy = (s_cursor_y - prev_y) / vdt;
                        let speed = (vx * vx + vy * vy).sqrt();
                        if speed > live.cursor_smear_speed_threshold.max(0.0) {
                            let shutter_seconds =
                                (1.0 / live.fps.max(1) as f32) * live.cursor_smear_shutter_scale.max(0.0);
                            let min_len = live.cursor_smear_min_len.max(0.0);
                            let max_len = live.cursor_smear_max_len.max(min_len);
                            let blur_len = (speed * shutter_seconds).clamp(min_len, max_len);
                            motion_dir_x = vx / speed;
                            motion_dir_y = vy / speed;
                            let extra_taps = live.cursor_smear_taps.clamp(1, 8) as usize;
                            let alpha_exp = live.cursor_smear_alpha_exp.max(0.05);
                            let alpha_scale = live.cursor_smear_alpha_scale.clamp(0.0, 1.0);
                            for i in 1..=extra_taps {
                                let t = i as f32 / extra_taps as f32;
                                let alpha =
                                    ((1.0 - t).powf(alpha_exp) * alpha_scale).clamp(0.0, 1.0);
                                taps.push([
                                    s_cursor_x - motion_dir_x * blur_len * t,
                                    s_cursor_y - motion_dir_y * blur_len * t,
                                    alpha,
                                ]);
                            }

                            // Stretch cursor shape along motion axis.
                            let stretch_threshold = live.cursor_smear_stretch_threshold.max(0.0);
                            let stretch_range = live.cursor_smear_stretch_range.max(1.0);
                            let s = ((speed - stretch_threshold) / stretch_range).clamp(0.0, 1.0);
                            motion_stretch = 1.0 + live.cursor_smear_max_stretch.max(0.0) * s;
                            motion_squash =
                                1.0 - live.cursor_smear_max_squash.clamp(0.0, 0.95) * s;
                        }
                    }
                }
                last_cursor_sample = Some((s_cursor_x, s_cursor_y, now));

                // Apply background-zoom transform to all tap coordinates so the
                // cursor tracks the shrunken frame rather than the full output.
                let taps = if live.background_enabled && live.background_zoom < 100.0 {
                    let zoom = live.background_zoom.clamp(1.0, 100.0).div(100.0);
                    let off_x = output_w as f32 * (1.0 - zoom) * 0.5;
                    let off_y = output_h as f32 * (1.0 - zoom) * 0.5;
                    taps.into_iter()
                        .map(|[x, y, a]| [x * zoom + off_x, y * zoom + off_y, a])
                        .collect()
                } else {
                    taps
                };

                gpu_pipeline::CursorState::with_blur_samples(
                    *ctex,
                    cursor_w,
                    cursor_h,
                    taps,
                    motion_dir_x,
                    motion_dir_y,
                    motion_stretch,
                    motion_squash,
                )
            } else {
                cursor_state_empty.clone()
            }
        } else {
            cursor_state_empty.clone()
        };

        let slot = (frame_idx as usize) % pipelines.len();
        let bg = if live.background_enabled { live_bg_tex } else { None };
        let zoom = if live.background_enabled { live.background_zoom.clamp(1.0, 100.0).div(100.0) } else { 1.0 };
        let fence = unsafe {
            pipelines[slot].render_with_cursor(
                NativeTexture(NonZero::new(frame_texture).unwrap()),
                use_external_texture,
                &cursor_state,
                bg,
                zoom,
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
            // pipelines[slot].gl.finish();
        }

        if matches!(options.output, CaptureOutput::EmbeddedPreview) {
            if preview_mailbox.is_some() {
                pipelines[slot]
                    .copy_to_mailbox(preview_mailbox.as_ref().unwrap())
                    .map_err(|e| EglError::Pipeline(e.to_string()))?;
            }
            // for debug
            preview_mailbox.as_ref().unwrap().get_frame()
                .map(|f| log::trace!("Updated preview mailbox with frame t_ns={}", f.t_ns));
            let _ = delete_gl_texture(&egl, frame_texture);
            frame_idx += 1;
            next_deadline += frame_period;
            let now = Instant::now();
            if next_deadline > now {
                thread::sleep(next_deadline - now);
            }
            // skip the encoder, since the ui handles rendering directly
            continue;
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

        if let Some(enc) = encoder.as_mut() {
            enc.push_frame(&exported)
                .map_err(|e| EglError::Pipeline(e.to_string()))?;
        } else {
            return Err(EglError::Pipeline(
                "encoder unexpectedly unavailable in non-embedded mode".to_string(),
            ));
        }
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
            if let Some(enc) = encoder.as_mut() {
                enc.request_keyframe("periodic");
            }
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
    if let Some(bg_tex) = live_bg_tex {
        delete_gl_texture(&egl, bg_tex.0.into())
            .map_err(|e| EglError::Pipeline(format!("failed to delete background texture: {e}")))?;
    }

    if let Some(enc) = encoder {
        enc.finish()
            .map_err(|e: crate::encode::EncodeError| EglError::Pipeline(e.to_string()))?;
    }

    log::info!(
        "Shutdown summary: frames_emitted={}",
        frame_idx,
    );

    match &options.output {
        CaptureOutput::Preview | CaptureOutput::EmbeddedPreview => {
            log::info!("Preview stopped");
            backend.send_notification(ProcessState::Stopped(None), 3).ok();
        },
        CaptureOutput::File(path) => {
            log::info!(
                "Video encoding complete, output saved to {}",
                path.to_string_lossy()
            );
            backend.send_notification(ProcessState::Stopped(Some(path.to_string_lossy().into())), 3).ok();
        }
    }

    Ok(())
}
