use std::os::fd::AsRawFd;

use gstreamer::{self as gst};
use gstreamer_allocators::{self as gst_alloc, DmaBufAllocatorExtManual};
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;

use crate::drm_kms::types::ExportedDmabuf;

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("Failed to create DMABuf memory: {0}")]
    DmaBufAllocation(String),
    #[error("Invalid DMABuf layout: {0}")]
    InvalidLayout(String),
    #[error("Failed to add VideoMeta: {0}")]
    VideoMeta(String),
    #[error("Failed to push buffer to AppSrc: {0}")]
    AppSrcPush(#[source] gst::FlowError),
}

fn dmabuf_size(fd: std::os::fd::RawFd) -> std::io::Result<usize> {
    let mut st = std::mem::MaybeUninit::<libc::stat>::uninit();
    let rc = unsafe { libc::fstat(fd, st.as_mut_ptr()) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let st = unsafe { st.assume_init() };
    if st.st_size <= 0 {
        return Err(std::io::Error::other("invalid dmabuf size"));
    }
    Ok(st.st_size as usize)
}

fn video_format_for_fourcc(fourcc: u32) -> Option<gst_video::VideoFormat> {
    match fourcc {
        0x34324241 => Some(gst_video::VideoFormat::Rgba), // DRM_FORMAT_ABGR8888
        0x34324258 => Some(gst_video::VideoFormat::Rgbx), // DRM_FORMAT_XBGR8888
        0x34325241 => Some(gst_video::VideoFormat::Bgra), // DRM_FORMAT_ARGB8888
        0x34325258 => Some(gst_video::VideoFormat::Bgrx), // DRM_FORMAT_XRGB8888
        0x3231564e => Some(gst_video::VideoFormat::Nv12), // DRM_FORMAT_NV12
        0x48344241 => Some(gst_video::VideoFormat::Rgb16), // DRM_FORMAT_ABGR16161616
        0x30314241 => Some(gst_video::VideoFormat::R210), // DRM_FORMAT_ABGR2101010
        0x30334241 => Some(gst_video::VideoFormat::R210), // DRM_FORMAT_ABGR2101010
        _ => None,
    }
}

pub fn push_exported_dmabuf(
    appsrc: &gst_app::AppSrc,
    ex: &ExportedDmabuf,
    pts_ns: u64,
    duration_ns: Option<u64>,
) -> Result<(), ExportError> {
    if ex.fds.is_empty() {
        return Err(ExportError::InvalidLayout(
            "exported dmabuf has zero planes".to_string(),
        ));
    }
    if ex.strides.len() != ex.fds.len() || ex.offsets.len() != ex.fds.len() {
        return Err(ExportError::InvalidLayout(format!(
            "planes/strides/offsets mismatch: planes={} strides={} offsets={}",
            ex.fds.len(),
            ex.strides.len(),
            ex.offsets.len()
        )));
    }
    for i in 0..ex.fds.len() {
        if ex.strides[i] <= 0 {
            return Err(ExportError::InvalidLayout(format!(
                "invalid stride {} on plane {}",
                ex.strides[i], i
            )));
        }
        if ex.offsets[i] < 0 {
            return Err(ExportError::InvalidLayout(format!(
                "invalid offset {} on plane {}",
                ex.offsets[i], i
            )));
        }
        log::trace!(
            "push dmabuf plane {}: stride={} offset={}",
            i,
            ex.strides[i],
            ex.offsets[i]
        );
    }

    let mut buffer = gst::Buffer::new();

    {
        let buf = buffer.get_mut().unwrap();

        let allocator = gst_alloc::DmaBufAllocator::new();
        for fd in &ex.fds {
            let size = dmabuf_size(fd.as_raw_fd())
                .map_err(|e| ExportError::DmaBufAllocation(e.to_string()))?;
            let fd_alloc = fd
                .try_clone()
                .map_err(|e| ExportError::DmaBufAllocation(format!("Failed to clone fd: {e}")))?;
            let dmabuf_mem = unsafe {
                allocator.alloc_dmabuf(fd_alloc, size).map_err(|e| {
                    ExportError::DmaBufAllocation(format!("Failed to allocate DMABuf memory: {e}"))
                })
            }?;
            buf.append_memory(dmabuf_mem);
        }

        let video_format = video_format_for_fourcc(ex.fourcc).ok_or_else(|| {
            ExportError::VideoMeta(format!("unsupported video format 0x{:08x}", ex.fourcc))
        })?;

        gst_video::VideoMeta::add_full(
            buf,
            gst_video::VideoFrameFlags::empty(),
            video_format,
            ex.width as u32,
            ex.height as u32,
            &ex.offsets.iter().map(|v| *v as usize).collect::<Vec<_>>(),
            &ex.strides.iter().copied().collect::<Vec<_>>(),
        )
        .map_err(|e| ExportError::VideoMeta(format!("Failed to add VideoMeta: {e}")))?;

        buf.set_pts(gst::ClockTime::from_nseconds(pts_ns));
        buf.set_dts(gst::ClockTime::from_nseconds(pts_ns));
        match duration_ns {
            Some(ns) => buf.set_duration(gst::ClockTime::from_nseconds(ns)),
            None => buf.set_duration(gst::ClockTime::NONE),
        }
    }

    appsrc
        .push_buffer(buffer)
        .map_err(ExportError::AppSrcPush)?;
    Ok(())
}
