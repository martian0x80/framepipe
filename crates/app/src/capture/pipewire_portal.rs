use khronos_egl as egl;
use std::{
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    sync::Arc,
    thread,
    time::Duration,
};

use crate::{
    capture::types::CaptureFrame,
    drm_kms::{egl_context::EglError, privd, types::CaptureOptions},
    portal::pipewire::{
        PipeWireCaptureProducer, build_pipewire_format_offers_for_session, start_capture_producer,
    },
    shared::pipewire_frame_ring::PipeWireFrameRing,
};

use super::{backend::CaptureBackend, types::CaptureBackendKind};

#[derive(Default)]
pub struct PipeWirePortalBackend {
    ring: Option<Arc<PipeWireFrameRing>>,
    producer: Option<PipeWireCaptureProducer>,
    first_frame_seen: bool,
    last_frame: Option<CaptureFrame>,
    privd: Option<privd::PrivdSession>,
}

impl CaptureBackend for PipeWirePortalBackend {
    fn start(&mut self, options: &CaptureOptions) -> Result<(), EglError> {
        let ring = Arc::new(PipeWireFrameRing::new(8));
        self.ring = Some(ring);
        self.producer = None;
        self.first_frame_seen = false;
        self.last_frame = None;
        let include_input_fds = options.cursor_composition;
        if include_input_fds {
            let privd_session = privd::acquire_device_fds(&options.card_path, include_input_fds)
                .map_err(|e| {
                    EglError::Pipeline(format!("failed to acquire device fds from privd: {e}"))
                })?;
            self.privd = Some(privd_session);
        } else {
            self.privd = None;
        }
        Ok(())
    }

    fn on_egl_ready(
        &mut self,
        egl_i: &egl::Instance<egl::Static>,
        display: egl::Display,
    ) -> Result<(), EglError> {
        if self.producer.is_some() {
            return Ok(());
        }
        let ring = self.ring.as_ref().ok_or_else(|| {
            EglError::Pipeline("pipewire backend ring not initialized".to_string())
        })?;
        let offers = build_pipewire_format_offers_for_session(egl_i, display);
        let producer = start_capture_producer(Arc::clone(ring), offers)
            .map_err(|e| EglError::Pipeline(format!("failed to start pipewire producer: {e}")))?;
        self.producer = Some(producer);
        Ok(())
    }

    fn kind(&self) -> CaptureBackendKind {
        CaptureBackendKind::PipewirePortal
    }

    fn next_frame(&mut self, timeout: Duration) -> Result<Option<CaptureFrame>, EglError> {
        let ring = self
            .ring
            .as_ref()
            .ok_or_else(|| EglError::Pipeline("pipewire backend not started".to_string()))?;
        let producer_ended = || {
            self.producer
                .as_ref()
                .is_some_and(|p| p.is_ended() || p.has_failed())
        };

        let start = std::time::Instant::now();
        if !self.first_frame_seen {
            let mut first_wait_logs: u32 = 0;
            while !self.first_frame_seen {
                if let Some(frame) = ring.pop_latest() {
                    self.first_frame_seen = true;
                    log::info!(
                        "first pipewire frame received: {}x{} format={}",
                        frame.width,
                        frame.height,
                        frame.fourcc
                    );
                    self.last_frame = Some(dup_capture_frame(&frame)?);
                    return Ok(Some(frame));
                }
                if producer_ended() {
                    return Err(EglError::Pipeline(
                        "pipewire stream ended before first frame".to_string(),
                    ));
                }
                if start.elapsed() >= timeout {
                    return Ok(None);
                }
                first_wait_logs = first_wait_logs.saturating_add(1);
                if first_wait_logs.is_multiple_of(100) {
                    log::debug!(
                        "pipewire backend waiting for first frame (stream may be paused until window damage)"
                    );
                }
                thread::sleep(Duration::from_millis(5));
            }
        } else {
            while start.elapsed() < timeout {
                if let Some(frame) = ring.pop_latest() {
                    self.last_frame = Some(dup_capture_frame(&frame)?);
                    return Ok(Some(frame));
                }
                if producer_ended() {
                    return Err(EglError::Pipeline("pipewire stream ended".to_string()));
                }
                thread::sleep(Duration::from_millis(5));
            }
        }

        if let Some(last) = self.last_frame.as_ref() {
            return Ok(Some(dup_capture_frame(last)?));
        }

        Ok(None)
    }

    fn stop(&mut self) -> Result<(), EglError> {
        if let Some(producer) = self.producer.take() {
            producer.stop();
        }
        drop(self.privd.take());
        self.privd = None;
        self.ring = None;
        self.last_frame = None;
        Ok(())
    }

    fn take_input_fds(
        &mut self,
    ) -> Option<std::collections::HashMap<std::path::PathBuf, std::os::fd::OwnedFd>> {
        self.privd
            .as_mut()
            .map(|s| std::mem::take(&mut s.input_fds))
    }
}

fn dup_fd_raw(raw_fd: i32) -> std::io::Result<OwnedFd> {
    let dup_fd = unsafe { libc::fcntl(raw_fd, libc::F_DUPFD_CLOEXEC, 0) };
    if dup_fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(dup_fd) })
}

fn dup_capture_frame(frame: &CaptureFrame) -> Result<CaptureFrame, EglError> {
    let mut fds = Vec::with_capacity(frame.plane_fds.len());
    for fd in &frame.plane_fds {
        let dup = dup_fd_raw(fd.as_raw_fd())
            .map_err(|e| EglError::Pipeline(format!("failed to dup pipewire frame fd: {e}")))?;
        fds.push(dup);
    }
    Ok(CaptureFrame {
        fb_id: frame.fb_id,
        width: frame.width,
        height: frame.height,
        fourcc: frame.fourcc,
        modifier: frame.modifier,
        plane_fds: fds,
        offsets: frame.offsets.clone(),
        strides: frame.strides.clone(),
    })
}
