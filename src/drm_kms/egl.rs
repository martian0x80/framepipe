// thanks to codex for the help here
// wow this was probably most awful part of the codebase to write
// i am just too dumb for this

use gbm::{AsRaw};
use glow::{HasContext, NativeTexture};
use signal_hook::consts::signal::{SIGINT, SIGTERM, SIGUSR1, SIGUSR2};
use signal_hook::flag as signal_flag;
use std::{
    fs,
    num::NonZero,
    os::fd::{AsFd, AsRawFd},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use crate::drm_kms::{
    debug,
    drm::{DrmInitError, init_drm_device},
    egl_dmabuf_export, gpu_pipeline,
    probe::ProbeSession,
    types::{Card, ProbeResult, CaptureOptions, CaptureOutput},
};
extern crate khronos_egl;

// dma_buf import tokens
const EGL_LINUX_DMA_BUF_EXT: i32 = 0x3270;
const EGL_LINUX_DRM_FOURCC_EXT: i32 = 0x3271;
const EGL_DMA_BUF_PLANE0_FD_EXT: i32 = 0x3272;
const EGL_DMA_BUF_PLANE0_OFFSET_EXT: i32 = 0x3273;
const EGL_DMA_BUF_PLANE0_PITCH_EXT: i32 = 0x3274;
const EGL_DMA_BUF_PLANE1_FD_EXT: i32 = 0x3275;
const EGL_DMA_BUF_PLANE1_OFFSET_EXT: i32 = 0x3276;
const EGL_DMA_BUF_PLANE1_PITCH_EXT: i32 = 0x3277;
const EGL_DMA_BUF_PLANE2_FD_EXT: i32 = 0x3278;
const EGL_DMA_BUF_PLANE2_OFFSET_EXT: i32 = 0x3279;
const EGL_DMA_BUF_PLANE2_PITCH_EXT: i32 = 0x327A;

// modifiers ext tokens
const EGL_DMA_BUF_PLANE3_FD_EXT: i32 = 0x3440;
const EGL_DMA_BUF_PLANE3_OFFSET_EXT: i32 = 0x3441;
const EGL_DMA_BUF_PLANE3_PITCH_EXT: i32 = 0x3442;
const EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT: i32 = 0x3443;
const EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT: i32 = 0x3444;
const EGL_DMA_BUF_PLANE1_MODIFIER_LO_EXT: i32 = 0x3445;
const EGL_DMA_BUF_PLANE1_MODIFIER_HI_EXT: i32 = 0x3446;
const EGL_DMA_BUF_PLANE2_MODIFIER_LO_EXT: i32 = 0x3447;
const EGL_DMA_BUF_PLANE2_MODIFIER_HI_EXT: i32 = 0x3448;
const EGL_DMA_BUF_PLANE3_MODIFIER_LO_EXT: i32 = 0x3449;
const EGL_DMA_BUF_PLANE3_MODIFIER_HI_EXT: i32 = 0x344A;

const EGL_PLATFORM_GBM_KHR: u32 = 0x31D7;
const EGL_PLATFORM_SURFACELESS_MESA: u32 = 0x31DD;

#[derive(Debug, Clone, Copy)]
pub enum EglBackend {
    Gbm,
    Surfaceless,
}

#[derive(Debug)]
pub struct EglCtx {
    pub egl: khronos_egl::Instance<khronos_egl::Static>,
    pub display: khronos_egl::Display,
    pub context: khronos_egl::Context,
    pub surface: khronos_egl::Surface,
    pub backend: EglBackend,

    // keep alive for GBM path
    _drm_file: Option<Card>,
    // _gbm_dev: Option<gbm::Device<Card>>,
}

#[derive(Debug, thiserror::Error)]
pub enum EglError {
    #[error("Failed to create GBM device")]
    GbmCreateDevice(#[source] super::drm::DrmInitError),
    #[error("Failed to probe DRM device")]
    Probe(#[source] crate::drm_kms::probe::ProbeError),
    #[error("Failed to initialize EGL")]
    EglInit(#[source] khronos_egl::Error),
    #[error("Failed to create EGL display")]
    EglDisplay,
    #[error("Required EGL extension not supported: {0}")]
    MissingExt(&'static str),
    #[error("Failed to choose EGL config")]
    ChooseConfig,
    #[error("Failed to create EGL context")]
    CreateContext(#[source] khronos_egl::Error),
    #[error("Failed to make EGL context current")]
    MakeCurrent(#[source] khronos_egl::Error),
    #[error("Failed to create EGL image")]
    CreateImage(#[source] khronos_egl::Error),
    #[error("Failed to query EGL extensions")]
    QueryExt(#[source] khronos_egl::Error),
    #[error("GPU pipeline error: {0}")]
    Pipeline(String),
    #[error("DMA-BUF export error: {0}")]
    Export(String),
    #[error("Unknown EGL error")]
    Unknown,
}

fn push_plane(
    attrs: &mut Vec<usize>,
    i: usize,
    p: (usize, i32, u32, u32),
    modifier: Option<u64>,
    with_mods: bool,
) {
    let (fd_k, off_k, pitch_k, lo_k, hi_k) = match i {
        0 => (
            EGL_DMA_BUF_PLANE0_FD_EXT,
            EGL_DMA_BUF_PLANE0_OFFSET_EXT,
            EGL_DMA_BUF_PLANE0_PITCH_EXT,
            EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT,
            EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT,
        ),
        1 => (
            EGL_DMA_BUF_PLANE1_FD_EXT,
            EGL_DMA_BUF_PLANE1_OFFSET_EXT,
            EGL_DMA_BUF_PLANE1_PITCH_EXT,
            EGL_DMA_BUF_PLANE1_MODIFIER_LO_EXT,
            EGL_DMA_BUF_PLANE1_MODIFIER_HI_EXT,
        ),
        2 => (
            EGL_DMA_BUF_PLANE2_FD_EXT,
            EGL_DMA_BUF_PLANE2_OFFSET_EXT,
            EGL_DMA_BUF_PLANE2_PITCH_EXT,
            EGL_DMA_BUF_PLANE2_MODIFIER_LO_EXT,
            EGL_DMA_BUF_PLANE2_MODIFIER_HI_EXT,
        ),
        3 => (
            EGL_DMA_BUF_PLANE3_FD_EXT,
            EGL_DMA_BUF_PLANE3_OFFSET_EXT,
            EGL_DMA_BUF_PLANE3_PITCH_EXT,
            EGL_DMA_BUF_PLANE3_MODIFIER_LO_EXT,
            EGL_DMA_BUF_PLANE3_MODIFIER_HI_EXT,
        ),
        _ => return,
    };

    attrs.extend_from_slice(&[
        fd_k as usize,
        p.1 as usize,
        off_k as usize,
        p.2 as usize,
        pitch_k as usize,
        p.3 as usize,
    ]);

    if with_mods {
        if let Some(m) = modifier {
            attrs.extend_from_slice(&[
                lo_k as usize,
                (m as u32) as usize,
                hi_k as usize,
                ((m >> 32) as u32) as usize,
            ]);
        }
    }
}

fn build_attrs(
    width: i32,
    height: i32,
    fourcc: u32,
    planes: &[(usize, i32, u32, u32)], // (plane_idx, fd, offset, pitch)
    modifier: Option<u64>,
    with_mods: bool,
) -> Vec<usize> {
    let mut attrs = vec![
        khronos_egl::WIDTH as usize,
        width as usize,
        khronos_egl::HEIGHT as usize,
        height as usize,
        EGL_LINUX_DRM_FOURCC_EXT as usize,
        fourcc as usize,
    ];

    for (i, p) in planes.iter().enumerate() {
        push_plane(&mut attrs, i, *p, modifier, with_mods);
    }

    attrs.push(khronos_egl::ATTRIB_NONE as usize);
    attrs
}

fn client_exts(egl: &khronos_egl::Instance<khronos_egl::Static>) -> String {
    unsafe {
        egl.query_string(None, khronos_egl::EXTENSIONS)
            .ok()
            .and_then(|s| s.to_str().ok().map(|x| x.to_owned()))
            .unwrap_or_else(|| "".to_string())
    }
}

fn has_ext(exts: &str, name: &str) -> bool {
    exts.split_whitespace().any(|e| e == name)
}

fn init_display_common(
    egl_i: &khronos_egl::Instance<khronos_egl::Static>,
    display: khronos_egl::Display,
) -> Result<(khronos_egl::Context, khronos_egl::Surface), EglError> {
    unsafe { egl_i.initialize(display) }.map_err(EglError::EglInit)?;

    let dext = unsafe { egl_i.query_string(Some(display), khronos_egl::EXTENSIONS) }
        .map_err(EglError::QueryExt)?
        .to_str()
        .map_err(|_| EglError::Unknown)?
        .to_owned();

    if !has_ext(&dext, "EGL_EXT_image_dma_buf_import")
        && !has_ext(&dext, "EGL_EXT_image_dma_buf_import_modifiers")
    {
        return Err(EglError::MissingExt(
            "EGL_EXT_image_dma_buf_import or EGL_EXT_image_dma_buf_import_modifiers",
        ));
    }

    let cfg_attribs = [
        khronos_egl::SURFACE_TYPE,
        khronos_egl::PBUFFER_BIT,
        khronos_egl::RENDERABLE_TYPE,
        khronos_egl::OPENGL_ES3_BIT,
        khronos_egl::RED_SIZE,
        8,
        khronos_egl::GREEN_SIZE,
        8,
        khronos_egl::BLUE_SIZE,
        8,
        khronos_egl::ALPHA_SIZE,
        8,
        khronos_egl::NONE,
    ];

    let config = unsafe { egl_i.choose_first_config(display, &cfg_attribs) }
        .map_err(|_| EglError::ChooseConfig)?
        .ok_or(EglError::ChooseConfig)?;

    let pbuf_attribs = [
        khronos_egl::WIDTH,
        1,
        khronos_egl::HEIGHT,
        1,
        khronos_egl::NONE,
    ];
    let surface = unsafe { egl_i.create_pbuffer_surface(display, config, &pbuf_attribs) }
        .map_err(|_| EglError::ChooseConfig)?;

    unsafe { egl_i.bind_api(khronos_egl::OPENGL_ES_API) }.map_err(|e| EglError::EglInit(e))?;
    let ctx_attribs = [khronos_egl::CONTEXT_CLIENT_VERSION, 3, khronos_egl::NONE];
    let context = unsafe { egl_i.create_context(display, config, None, &ctx_attribs) }
        .map_err(|e| EglError::CreateContext(e))?;

    unsafe { egl_i.make_current(display, Some(surface), Some(surface), Some(context)) }
        .map_err(|e| EglError::MakeCurrent(e))?;

    Ok((context, surface))
}

// fuck gbm for now
fn try_init_gbm(
    egl_i: &khronos_egl::Instance<khronos_egl::Static>,
    card_path: &str,
) -> Result<
    (
        khronos_egl::Display,
        khronos_egl::Context,
        khronos_egl::Surface,
        Card,
    ),
    EglError,
> {
    log::debug!(
        "Attempting to initialize GBM device for card at {}",
        card_path
    );
    let drm_file = init_drm_device(card_path).map_err(|e| EglError::GbmCreateDevice(e))?;
    let drm_fd = drm_file.as_fd();
    log::debug!(
        "Opened DRM device at {} with fd {}",
        card_path,
        drm_fd.as_raw_fd()
    );

    let gbm_dev = gbm::Device::new(drm_fd)
        .map_err(|e| EglError::GbmCreateDevice(DrmInitError::OpenDevice(e)))?;

    let display = unsafe {
        egl_i.get_platform_display(
            EGL_PLATFORM_GBM_KHR,
            gbm_dev.as_raw() as *mut _,
            &[khronos_egl::ATTRIB_NONE],
        )
    }
    .map_err(EglError::EglInit)?;

    match init_display_common(egl_i, display) {
        // gbm_dev carries the life of drm_file now, no point returning both
        Ok((ctx, surf)) => Ok((display, ctx, surf, drm_file)),
        Err(e) => {
            drop(gbm_dev);
            Err(e)
        }
    }
}

fn try_init_surfaceless(
    egl_i: &khronos_egl::Instance<khronos_egl::Static>,
) -> Result<
    (
        khronos_egl::Display,
        khronos_egl::Context,
        khronos_egl::Surface,
    ),
    EglError,
> {
    log::debug!("Attempting to initialize surfaceless EGL display");
    let display = unsafe {
        egl_i.get_platform_display(
            EGL_PLATFORM_SURFACELESS_MESA,
            std::ptr::null_mut(),
            &[khronos_egl::ATTRIB_NONE],
        )
    }
    .map_err(|e| EglError::EglInit(e))?;

    let (ctx, surf) = init_display_common(egl_i, display)?;
    Ok((display, ctx, surf))
}

pub fn init_egl(card_path: &str) -> Result<EglCtx, EglError> {
    let egl_i = khronos_egl::Instance::new(khronos_egl::Static);
    let cext = client_exts(&egl_i);
    log::debug!("EGL client extensions: {}", cext);

    // i have no idea if the order or dependecies are correct here
    let can_gbm = (has_ext(&cext, "EGL_EXT_platform_base")
        && has_ext(&cext, "EGL_KHR_platform_gbm"))
        || has_ext(&cext, "EGL_MESA_platform_gbm");

    let can_surfaceless = (has_ext(&cext, "EGL_EXT_platform_base")
        && has_ext(&cext, "EGL_MESA_platform_surfaceless"))
        || has_ext(&cext, "EGL_KHR_surfaceless_context")
        || has_ext(&cext, "EGL_MESA_configless_context");

    // if can_gbm {
    //     if let Ok((display, context, surface, gbm_dev)) = try_init_gbm(&egl_i, card_path)
    //     {
    //         return Ok(EglCtx {
    //             egl: egl_i,
    //             display,
    //             context,
    //             surface,
    //             backend: EglBackend::Gbm,
    //             _drm_file: Some(gbm_dev),
    //         });
    //     }
    // }

    if can_surfaceless {
        match try_init_surfaceless(&egl_i) {
            Ok((display, context, surface)) => {
                return Ok(EglCtx {
                    egl: egl_i,
                    display,
                    context,
                    surface,
                    backend: EglBackend::Surfaceless,
                    _drm_file: None,
                });
            }
            Err(e) => {
                log::warn!("Failed to initialize surfaceless EGL display: {}", e);
            }
        }
    }

    Err(EglError::Unknown)
}

fn close_fds(plane_fds: &[Option<std::os::fd::OwnedFd>]) {
    for fd in plane_fds {
        if let Some(fd) = fd {
            let _ = fd.as_raw_fd();
            // die now
        }
    }
}

fn egl_image_to_texture(
    egl: &khronos_egl::Instance<khronos_egl::Static>,
    image: khronos_egl::Image,
) -> Result<u32, EglError> {
    if image.as_ptr().is_null() {
        return Err(EglError::Unknown);
    }

    unsafe {
        // Load GL functions
        let gl_gen_textures = egl
            .get_proc_address("glGenTextures")
            .ok_or(EglError::MissingExt("glGenTextures"))?;
        let gl_bind_texture = egl
            .get_proc_address("glBindTexture")
            .ok_or(EglError::MissingExt("glBindTexture"))?;
        let gl_tex_param_i = egl
            .get_proc_address("glTexParameteri")
            .ok_or(EglError::MissingExt("glTexParameteri"))?;
        let gl_egl_image_target = egl
            .get_proc_address("glEGLImageTargetTexture2DOES")
            .ok_or(EglError::MissingExt("glEGLImageTargetTexture2DOES"))?;

        let gl_gen_textures: unsafe extern "C" fn(i32, *mut u32) =
            std::mem::transmute(gl_gen_textures);
        let gl_bind_texture: unsafe extern "C" fn(u32, u32) = std::mem::transmute(gl_bind_texture);
        let gl_tex_param_i: unsafe extern "C" fn(u32, u32, i32) =
            std::mem::transmute(gl_tex_param_i);
        let gl_egl_image_target: unsafe extern "C" fn(u32, *const std::ffi::c_void) =
            std::mem::transmute(gl_egl_image_target);

        let mut tex: u32 = 0;
        gl_gen_textures(1, &mut tex);

        const GL_TEXTURE_2D: u32 = 0x0DE1;
        const GL_LINEAR: u32 = 0x2601;
        const GL_CLAMP_TO_EDGE: u32 = 0x812F;
        const GL_TEXTURE_MIN_FILTER: u32 = 0x2801;
        const GL_TEXTURE_MAG_FILTER: u32 = 0x2800;
        const GL_TEXTURE_WRAP_S: u32 = 0x2802;
        const GL_TEXTURE_WRAP_T: u32 = 0x2803;

        gl_bind_texture(GL_TEXTURE_2D, tex);

        gl_tex_param_i(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as i32);
        gl_tex_param_i(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as i32);
        gl_tex_param_i(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as i32);
        gl_tex_param_i(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as i32);

        gl_egl_image_target(GL_TEXTURE_2D, image.as_ptr() as *const std::ffi::c_void);

        Ok(tex)
    }
}

fn delete_gl_texture(
    egl: &khronos_egl::Instance<khronos_egl::Static>,
    texture: u32,
) -> Result<(), EglError> {
    unsafe {
        let gl_delete_textures = egl
            .get_proc_address("glDeleteTextures")
            .ok_or(EglError::MissingExt("glDeleteTextures"))?;
        let gl_delete_textures: unsafe extern "C" fn(i32, *const u32) =
            std::mem::transmute(gl_delete_textures);
        gl_delete_textures(1, &texture as *const u32);
    }
    Ok(())
}

fn import_current_capture_texture(
    probe_session: &mut ProbeSession,
    egl: &khronos_egl::Instance<khronos_egl::Static>,
    display: khronos_egl::Display,
) -> Result<(u32, i32, i32, u32), EglError> {
    let ProbeResult {
        fb_id,
        fb_info,
        plane_fds,
    } = probe_session.capture_frame().map_err(EglError::Probe)?;
    let (w, h) = (fb_info.size().0 as i32, fb_info.size().1 as i32);
    let fourcc = fb_info.pixel_format() as u32;
    let modifier: Option<u64> = fb_info.modifier().map(|m| m.into());
    let mut planes = Vec::new();
    for i in 0..plane_fds.len().min(4) {
        let fd = match &plane_fds[i] {
            Some(fd) => fd.as_raw_fd(),
            None => continue,
        };
        let offset = fb_info.offsets()[i];
        let pitch = fb_info.pitches()[i];
        planes.push((i, fd, offset, pitch));
    }

    let attrs_mod = build_attrs(w, h, fourcc, &planes, modifier, true);
    let image = unsafe {
        egl.create_image(
            display,
            khronos_egl::Context::from_ptr(khronos_egl::NO_CONTEXT),
            EGL_LINUX_DMA_BUF_EXT as u32,
            khronos_egl::ClientBuffer::from_ptr(std::ptr::null_mut()),
            &attrs_mod,
        )
    }
    .or_else(|_| {
        let attrs_nomod = build_attrs(w, h, fourcc, &planes, None, false);
        unsafe {
            egl.create_image(
                display,
                khronos_egl::Context::from_ptr(khronos_egl::NO_CONTEXT),
                EGL_LINUX_DMA_BUF_EXT as u32,
                khronos_egl::ClientBuffer::from_ptr(std::ptr::null_mut()),
                &attrs_nomod,
            )
        }
    })
    .map_err(EglError::CreateImage)?;

    let texture = egl_image_to_texture(egl, image)?;
    unsafe {
        egl.destroy_image(display, image)
            .map_err(EglError::CreateImage)?;
    }
    Ok((texture, w, h, fb_id))
}

pub fn egl_main(options: CaptureOptions) -> Result<(), EglError> {
    let EglCtx {
        egl,
        display,
        context,
        surface: _,
        backend: _,
        _drm_file: _,
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
    // let _ = debug::debug_dump_texture_ppm(&egl, texture, w, h, "debug_output.ppm");

    let mut pipelines: Vec<gpu_pipeline::GpuPipeline> = Vec::with_capacity(3);
    for _ in 0..3 {
        pipelines.push(unsafe { gpu_pipeline::GpuPipeline::new(&egl, output_w, output_h) }
            .map_err(EglError::Pipeline)?);
    }

    // no cursor yet
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
            // Ensure writes to output texture are visible before EGL DMABUF export.
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
        CaptureOutput::Preview => {
        crate::drm_kms::encoder::GstEncoder::new_with_output(
            crate::drm_kms::encoder::EncoderOutput::Preview,
            &first_exported,
            enc_opts.clone(),
        )
        .map_err(|e| EglError::Pipeline(e.to_string()))?
        }
        CaptureOutput::File(path) => crate::drm_kms::encoder::GstEncoder::new(
            &path.to_string_lossy(),
            &first_exported,
            enc_opts.clone(),
        )
            .map_err(|e| EglError::Pipeline(e.to_string()))?
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
            // Ensure writes to output texture are visible before EGL DMABUF export.
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

    encoder.finish().map_err(|e| EglError::Pipeline(e.to_string()))?;
    match &options.output {
        CaptureOutput::Preview => log::info!("Preview stopped"),
        CaptureOutput::File(path) => {
            log::info!("Video encoding complete, output saved to {}", path.to_string_lossy())
        }
    }

    Ok(())
}
