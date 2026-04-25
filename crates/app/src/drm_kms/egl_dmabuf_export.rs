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

#[allow(clippy::missing_safety_doc)]
pub unsafe fn export_rgba_tex_to_dmabuf(
    egl: &khronos_egl::Instance<khronos_egl::Static>,
    display: khronos_egl::Display,
    context: khronos_egl::Context,
    tex: u32,
    width: i32,
    height: i32,
) -> Result<crate::drm_kms::types::ExportedDmabuf, String> {
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
            let _ = egl.destroy_image(display, image);
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
            let _ = egl.destroy_image(display, image);
            return Err("eglExportDMABUFImageMESA failed".into());
        }

        if nplanes <= 0 {
            let _ = egl.destroy_image(display, image);
            return Err("eglExportDMABUFImageQueryMESA returned no planes".into());
        }
        if strides.len() != nplanes as usize || offsets.len() != nplanes as usize {
            let _ = egl.destroy_image(display, image);
            return Err("dmabuf layout vectors do not match reported plane count".into());
        }
        for i in 0..(nplanes as usize) {
            if fds[i] < 0 {
                let _ = egl.destroy_image(display, image);
                return Err(format!("invalid fd for exported plane {}", i));
            }
            if strides[i] <= 0 {
                let _ = egl.destroy_image(display, image);
                return Err(format!("invalid stride {} for plane {}", strides[i], i));
            }
            if offsets[i] < 0 {
                let _ = egl.destroy_image(display, image);
                return Err(format!("invalid offset {} for plane {}", offsets[i], i));
            }
            log::trace!(
                "exported plane {}: fd={} stride={} offset={}",
                i,
                fds[i],
                strides[i],
                offsets[i]
            );
        }

        let owned = fds
            .into_iter()
            .map(|fd| OwnedFd::from_raw_fd(fd))
            .collect::<Vec<_>>();

        egl.destroy_image(display, image)
            .map_err(|e| format!("DestroyImageKHR failed: {e:?}"))?;

        Ok(crate::drm_kms::types::ExportedDmabuf {
            width,
            height,
            fourcc: fourcc as u32,
            modifier,
            fds: owned,
            strides,
            offsets,
            acquire_fence_fd: None,
        })
    }
}
