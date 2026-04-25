use std::os::fd::AsRawFd;

use crate::capture::types::CaptureFrame;
use crate::drm_kms::{
    privd::PrivdSession,
    probe::ProbeSession,
    types::{Card, ProbeResult},
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

// const EGL_PLATFORM_GBM_KHR: u32 = 0x31D7;
const EGL_PLATFORM_SURFACELESS_MESA: u32 = 0x31DD;

#[derive(Debug, Clone, Copy)]
pub enum EglBackend {
    // Gbm,
    Surfaceless,
}

#[derive(Debug)]
pub struct EglCtx {
    pub egl: khronos_egl::Instance<khronos_egl::Static>,
    pub display: khronos_egl::Display,
    pub context: khronos_egl::Context,
    #[expect(unused)]
    pub surface: khronos_egl::Surface,
    #[expect(unused)]
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

    if with_mods && let Some(m) = modifier {
        attrs.extend_from_slice(&[
            lo_k as usize,
            (m as u32) as usize,
            hi_k as usize,
            ((m >> 32) as u32) as usize,
        ]);
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

    attrs.push(khronos_egl::ATTRIB_NONE);
    attrs
}

fn client_exts(egl: &khronos_egl::Instance<khronos_egl::Static>) -> String {
    egl.query_string(None, khronos_egl::EXTENSIONS)
        .ok()
        .and_then(|s| s.to_str().ok().map(|x| x.to_owned()))
        .unwrap_or_default()
}

fn has_ext(exts: &str, name: &str) -> bool {
    exts.split_whitespace().any(|e| e == name)
}

fn init_display_common(
    egl_i: &khronos_egl::Instance<khronos_egl::Static>,
    display: khronos_egl::Display,
) -> Result<(khronos_egl::Context, khronos_egl::Surface), EglError> {
    egl_i.initialize(display).map_err(EglError::EglInit)?;

    let dext = egl_i
        .query_string(Some(display), khronos_egl::EXTENSIONS)
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

    let config = egl_i
        .choose_first_config(display, &cfg_attribs)
        .map_err(|_| EglError::ChooseConfig)?
        .ok_or(EglError::ChooseConfig)?;

    let pbuf_attribs = [
        khronos_egl::WIDTH,
        1,
        khronos_egl::HEIGHT,
        1,
        khronos_egl::NONE,
    ];
    let surface = egl_i
        .create_pbuffer_surface(display, config, &pbuf_attribs)
        .map_err(|_| EglError::ChooseConfig)?;

    egl_i
        .bind_api(khronos_egl::OPENGL_ES_API)
        .map_err(EglError::EglInit)?;
    let ctx_attribs = [khronos_egl::CONTEXT_CLIENT_VERSION, 3, khronos_egl::NONE];
    let context = egl_i
        .create_context(display, config, None, &ctx_attribs)
        .map_err(EglError::CreateContext)?;

    egl_i
        .make_current(display, Some(surface), Some(surface), Some(context))
        .map_err(EglError::MakeCurrent)?;

    Ok((context, surface))
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
    .map_err(EglError::EglInit)?;

    let (ctx, surf) = init_display_common(egl_i, display)?;
    Ok((display, ctx, surf))
}

pub fn init_egl(_card_path: &str) -> Result<EglCtx, EglError> {
    let egl_i = khronos_egl::Instance::new(khronos_egl::Static);
    let cext = client_exts(&egl_i);
    log::debug!("EGL client extensions: {}", cext);

    // i have no idea if the order or dependecies are correct here
    // let can_gbm = (has_ext(&cext, "EGL_EXT_platform_base")
    //     && has_ext(&cext, "EGL_KHR_platform_gbm"))
    //     || has_ext(&cext, "EGL_MESA_platform_gbm");

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

fn egl_image_to_texture_target(
    egl: &khronos_egl::Instance<khronos_egl::Static>,
    image: khronos_egl::Image,
    target: u32,
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
        let gl_get_error = egl
            .get_proc_address("glGetError")
            .ok_or(EglError::MissingExt("glGetError"))?;
        let gl_tex_param_i = egl
            .get_proc_address("glTexParameteri")
            .ok_or(EglError::MissingExt("glTexParameteri"))?;
        let gl_egl_image_target = egl
            .get_proc_address("glEGLImageTargetTexture2DOES")
            .ok_or(EglError::MissingExt("glEGLImageTargetTexture2DOES"))?;

        let gl_gen_textures: unsafe extern "C" fn(i32, *mut u32) =
            std::mem::transmute(gl_gen_textures);
        let gl_bind_texture: unsafe extern "C" fn(u32, u32) = std::mem::transmute(gl_bind_texture);
        let gl_get_error: unsafe extern "C" fn() -> u32 = std::mem::transmute(gl_get_error);
        let gl_tex_param_i: unsafe extern "C" fn(u32, u32, i32) =
            std::mem::transmute(gl_tex_param_i);
        let gl_egl_image_target: unsafe extern "C" fn(u32, *const std::ffi::c_void) =
            std::mem::transmute(gl_egl_image_target);

        let mut tex: u32 = 0;
        gl_gen_textures(1, &mut tex);

        const GL_LINEAR: u32 = 0x2601;
        const GL_CLAMP_TO_EDGE: u32 = 0x812F;
        const GL_TEXTURE_MIN_FILTER: u32 = 0x2801;
        const GL_TEXTURE_MAG_FILTER: u32 = 0x2800;
        const GL_TEXTURE_WRAP_S: u32 = 0x2802;
        const GL_TEXTURE_WRAP_T: u32 = 0x2803;

        while gl_get_error() != 0 {}

        gl_bind_texture(target, tex);

        gl_tex_param_i(target, GL_TEXTURE_MIN_FILTER, GL_LINEAR as i32);
        gl_tex_param_i(target, GL_TEXTURE_MAG_FILTER, GL_LINEAR as i32);
        gl_tex_param_i(target, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as i32);
        gl_tex_param_i(target, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as i32);

        gl_egl_image_target(target, image.as_ptr() as *const std::ffi::c_void);

        let err = gl_get_error();
        if err != 0 {
            return Err(EglError::Pipeline(format!(
                "GL bind of EGLImage failed with error 0x{err:04x}"
            )));
        }

        Ok(tex)
    }
}

fn try_bind_as_2d(
    egl: &khronos_egl::Instance<khronos_egl::Static>,
    image: khronos_egl::Image,
) -> Result<u32, EglError> {
    egl_image_to_texture_target(egl, image, glow::TEXTURE_2D)
}

fn try_bind_as_external(
    egl: &khronos_egl::Instance<khronos_egl::Static>,
    image: khronos_egl::Image,
) -> Result<u32, EglError> {
    const GL_TEXTURE_EXTERNAL_OES: u32 = 0x8D65;
    egl_image_to_texture_target(egl, image, GL_TEXTURE_EXTERNAL_OES)
}

pub(crate) fn delete_gl_texture(
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

// this is now abstracted in the capture backend trait
#[expect(dead_code)]
pub(crate) fn import_current_capture_texture(
    probe_session: &mut ProbeSession,
    privd_session: &mut PrivdSession,
    egl: &khronos_egl::Instance<khronos_egl::Static>,
    display: khronos_egl::Display,
) -> Result<(u32, i32, i32, u32, bool), EglError> {
    let ProbeResult {
        fb_id,
        fb_info: _,
        plane_fds: _,
    } = probe_session.capture_frame().map_err(EglError::Probe)?;
    let exported = privd_session
        .export_framebuffer(fb_id)
        .map_err(|e| EglError::Pipeline(format!("privd framebuffer export failed: {e}")))?;
    let frame_info = exported.info;
    let capture_frame = CaptureFrame {
        fb_id: frame_info.fb_id,
        width: frame_info.width,
        height: frame_info.height,
        fourcc: frame_info.fourcc,
        modifier: frame_info.modifier,
        plane_fds: exported.fds,
        offsets: frame_info
            .offsets
            .iter()
            .map(|v| (*v).max(0) as u32)
            .collect(),
        strides: frame_info
            .strides
            .iter()
            .map(|v| (*v).max(0) as u32)
            .collect(),
    };

    import_capture_frame_texture(capture_frame, egl, display)
}

pub(crate) fn import_capture_frame_texture(
    frame: CaptureFrame,
    egl: &khronos_egl::Instance<khronos_egl::Static>,
    display: khronos_egl::Display,
) -> Result<(u32, i32, i32, u32, bool), EglError> {
    let fb_id = frame.fb_id;
    let w = frame.width;
    let h = frame.height;
    let fourcc = frame.fourcc;
    let modifier = frame.modifier;

    let mut planes = Vec::new();
    for i in 0..frame.plane_fds.len().min(4) {
        let fd = frame.plane_fds[i].as_raw_fd();
        let offset = *frame.offsets.get(i).unwrap_or(&0_u32);
        let pitch = *frame.strides.get(i).unwrap_or(&0_u32);
        planes.push((i, fd, offset, pitch));
    }
    log::trace!(
        "import fb={} size={}x{} fourcc=0x{:08x} modifier={:?} planes={:?}",
        fb_id,
        w,
        h,
        fourcc,
        modifier,
        planes
    );

    let attrs_mod = build_attrs(w, h, fourcc, &planes, modifier, true);
    let image_try_mod = unsafe {
        egl.create_image(
            display,
            khronos_egl::Context::from_ptr(khronos_egl::NO_CONTEXT),
            EGL_LINUX_DMA_BUF_EXT as u32,
            khronos_egl::ClientBuffer::from_ptr(std::ptr::null_mut()),
            &attrs_mod,
        )
    };
    let image = match image_try_mod {
        Ok(img) => img,
        Err(mod_err) => {
            log::warn!(
                "eglCreateImage dmabuf import with modifier failed for fb={} fourcc=0x{:08x} modifier={:?}: {}",
                fb_id,
                fourcc,
                modifier,
                mod_err
            );
            let attrs_nomod = build_attrs(w, h, fourcc, &planes, None, false);
            match unsafe {
                egl.create_image(
                    display,
                    khronos_egl::Context::from_ptr(khronos_egl::NO_CONTEXT),
                    EGL_LINUX_DMA_BUF_EXT as u32,
                    khronos_egl::ClientBuffer::from_ptr(std::ptr::null_mut()),
                    &attrs_nomod,
                )
            } {
                Ok(img) => img,
                Err(nomod_err) => {
                    log::error!(
                        "eglCreateImage dmabuf import without modifier failed for fb={} fourcc=0x{:08x}: {} (mod-attempt error was: {})",
                        fb_id,
                        fourcc,
                        nomod_err,
                        mod_err
                    );
                    return Err(EglError::CreateImage(nomod_err));
                }
            }
        }
    };

    let (texture, use_external_texture) = match try_bind_as_2d(egl, image) {
        Ok(tex) => (tex, false),
        Err(err) => {
            log::warn!(
                "egl image bind to GL_TEXTURE_2D failed for fb={} fourcc=0x{:08x} modifier={:?}: {err}; retrying external texture",
                fb_id,
                fourcc,
                modifier,
            );
            (try_bind_as_external(egl, image)?, true)
        }
    };
    egl.destroy_image(display, image)
        .map_err(EglError::CreateImage)?;
    Ok((texture, w, h, fb_id, use_external_texture))
}
