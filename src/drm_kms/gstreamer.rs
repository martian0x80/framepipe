use std::os::fd::AsRawFd;

use gstreamer::{self as gst};
use gstreamer_allocators::{self as gst_alloc, DmaBufAllocatorExtManual};
use gstreamer_app as gst_app;

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
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "invalid dmabuf size",
        ));
    }
    Ok(st.st_size as usize)
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

    // 1) Create empty buffer
    let mut buffer = gst::Buffer::new();

    {
        let buf = buffer.get_mut().unwrap();

        // 2) Append one DMABuf memory block per plane fd
        // API names can vary slightly by gst-rs version.
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

        // 3) Attach VideoMeta with plane offsets/strides
        // format is DMA_DRM + drm-format in caps, so VideoMeta should match layout.
        // gst_video::VideoMeta::add_full(
        //     buf,
        //     gst_video::VideoFrameFlags::empty(),
        //     gst_video::VideoFormat::Xrgb, // keep DMA_DRM in caps; meta is layout carrier
        //     ex.width as u32,
        //     ex.height as u32,
        //     &ex.offsets.iter().map(|v| *v as usize).collect::<Vec<_>>(),
        //     &ex.strides.iter().map(|v| *v as i32).collect::<Vec<_>>(),
        // )
        // .map_err(|e| ExportError::VideoMeta(format!("Failed to add VideoMeta: {e}")))?;

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
