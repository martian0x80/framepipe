// disclaimer: this was written with codex (ai) assistance

use std::ffi::c_void;
use std::os::fd::{FromRawFd, OwnedFd};

const EGL_GL_TEXTURE_2D_KHR: u32 = 0x30B1;
const EGL_IMAGE_PRESERVED_KHR: i32 = 0x30D2;
const EGL_NONE: i32 = 0x3038;

type EglExportQuery = unsafe extern "C" fn(
    dpy: *const c_void,
    image: *const c_void,
    fourcc: *mut i32,
    num_planes: *mut i32,
    modifier: *mut u64,
) -> u32;

type EglExportImage = unsafe extern "C" fn(
    dpy: *const c_void,
    image: *const c_void,
    fds: *mut i32,
    strides: *mut i32,
    offsets: *mut i32,
) -> u32;

pub unsafe fn export_rgba_tex_to_dmabuf(
    egl: &khronos_egl::Instance<khronos_egl::Static>,
    display: khronos_egl::Display,
    context: khronos_egl::Context,
    tex: u32,
    width: i32,
    height: i32,
) -> Result<crate::drm_kms::gpu_pipeline::ExportedDmabuf, String> {
    unsafe {
        let attrs = [EGL_IMAGE_PRESERVED_KHR as usize, 1, EGL_NONE as usize];
        let image = egl
            .create_image(
                display,
                context,
                EGL_GL_TEXTURE_2D_KHR,
                khronos_egl::ClientBuffer::from_ptr(tex as usize as *mut _),
                &attrs,
            )
            .map_err(|e| format!("CreateImageKHR failed: {e:?}"))?;

        let q = egl
            .get_proc_address("eglExportDMABUFImageQueryMESA")
            .ok_or("missing eglExportDMABUFImageQueryMESA")?;
        let e = egl
            .get_proc_address("eglExportDMABUFImageMESA")
            .ok_or("missing eglExportDMABUFImageMESA")?;

        let q: EglExportQuery = std::mem::transmute(q);
        let e: EglExportImage = std::mem::transmute(e);

        let mut fourcc = 0i32;
        let mut nplanes = 0i32;
        let mut modifier = 0u64;
        if q(
            display.as_ptr(),
            image.as_ptr(),
            &mut fourcc,
            &mut nplanes,
            &mut modifier,
        ) == 0
        {
            return Err("eglExportDMABUFImageQueryMESA failed".into());
        }

        let mut fds = vec![-1; nplanes as usize];
        let mut strides = vec![0; nplanes as usize];
        let mut offsets = vec![0; nplanes as usize];
        if e(
            display.as_ptr(),
            image.as_ptr(),
            fds.as_mut_ptr(),
            strides.as_mut_ptr(),
            offsets.as_mut_ptr(),
        ) == 0
        {
            return Err("eglExportDMABUFImageMESA failed".into());
        }

        let owned = fds
            .into_iter()
            .map(|fd| OwnedFd::from_raw_fd(fd))
            .collect::<Vec<_>>();

        Ok(crate::drm_kms::gpu_pipeline::ExportedDmabuf {
            width,
            height,
            fourcc: fourcc as u32,
            modifier,
            fds: owned,
            strides,
            offsets,
            acquire_fence_fd: None, // fill if you export native fence fd
        })
    }
}
