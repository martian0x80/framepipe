// thanks to codex for the help here
// wow this was probably most awful part of the codebase to write
// i am just too dumb for this

use std::os::fd::AsRawFd;

use crate::drm_kms::{probe::probe, types::ProbeResult};
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

const EGL_PLATFORM_SURFACELESS_MESA: u32 = 0x31DD;

#[derive(Debug, thiserror::Error)]
pub enum EglError {
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

#[derive(Debug)]
pub struct EglCtx {
    pub egl: khronos_egl::Instance<khronos_egl::Static>,
    pub display: khronos_egl::Display,
    pub context: khronos_egl::Context,
    pub surface: khronos_egl::Surface, // tiny pbuffer for safe current context
}

#[allow(unused_unsafe)]
pub fn init_egl_headless() -> Result<EglCtx, EglError> {
    let egl = khronos_egl::Instance::new(khronos_egl::Static);

    // 1) Surfaceless platform display (avoids X11 auth / polkit-looking noise)
    let display = unsafe {
        egl.get_platform_display(
            EGL_PLATFORM_SURFACELESS_MESA,
            std::ptr::null_mut(),
            &[khronos_egl::ATTRIB_NONE],
        )
    }.map_err(|e| EglError::EglInit(e))?;

    unsafe { egl.initialize(display) }.map_err(|e| EglError::EglInit(e))?;

    // 2) Check required dma-buf import extension(s)
    let exts = unsafe { egl.query_string(Some(display), khronos_egl::EXTENSIONS) }.unwrap_or_default();
    log::debug!("EGL extensions: {}", exts.to_str().unwrap_or("Invalid UTF-8"));
    if !exts.to_str().map_err(|e| EglError::Unknown)?.contains("EGL_EXT_image_dma_buf_import") {
        return Err(EglError::MissingExt("EGL_EXT_image_dma_buf_import"));
    }
    // Optional but recommended:
    // EGL_EXT_image_dma_buf_import_modifiers

    // 3) GLES3 config + tiny pbuffer + context
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

    let config = unsafe { egl.choose_first_config(display, &cfg_attribs) }
        .map_err(|_| EglError::ChooseConfig)?
        .ok_or(EglError::ChooseConfig)?;

    let pbuf_attribs = [khronos_egl::WIDTH, 1, khronos_egl::HEIGHT, 1, khronos_egl::NONE];
    let surface = unsafe { egl.create_pbuffer_surface(display, config, &pbuf_attribs) }
        .map_err(|_| EglError::ChooseConfig)?;

    unsafe { egl.bind_api(khronos_egl::OPENGL_ES_API) }.map_err(|e| EglError::EglInit(e))?;
    let ctx_attribs = [khronos_egl::CONTEXT_CLIENT_VERSION, 3, khronos_egl::NONE];
    let context = unsafe { egl.create_context(display, config, None, &ctx_attribs) }
        .map_err(|e| EglError::CreateContext(e))?;

    unsafe { egl.make_current(display, Some(surface), Some(surface), Some(context)) }
        .map_err(|e| EglError::MakeCurrent(e))?;

    Ok(EglCtx {
        egl,
        display,
        context,
        surface,
    })
}

fn close_fds(plane_fds: &[Option<std::os::fd::OwnedFd>]) {
    for fd in plane_fds {
        if let Some(fd) = fd {
            let _ = fd.as_raw_fd();
            // die now
        }
    }
}

pub fn egl_main() -> Result<(), EglError> {
    let ProbeResult { fb_info, plane_fds } = probe().map_err(|e| EglError::Probe(e))?;
    let EglCtx { egl, display, context: _, surface: _ } = init_egl_headless()?;
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
    .map_err(|e| EglError::CreateImage(e))?;

    image.as_ptr();
    log::info!("Successfully created EGL image from dma-buf! | Image handle: {:?}", image.as_ptr());

    close_fds(&plane_fds);

    Ok(())
}
