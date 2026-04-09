use std::{sync::Arc, thread, time::Duration};
use khronos_egl as egl;

use crate::{
    capture::types::CaptureFrame,
    drm_kms::{egl_context::EglError, privd, types::CaptureOptions},
    portal::pipewire::{
        PipeWireCaptureProducer, build_pipewire_enum_format_bytes, start_capture_producer,
    },
    shared::pipewire_frame_ring::PipeWireFrameRing,
};

use super::{backend::CaptureBackend, types::CaptureBackendKind};

#[derive(Default)]
pub struct PipeWirePortalBackend {
    ring: Option<Arc<PipeWireFrameRing>>,
    producer: Option<PipeWireCaptureProducer>,
    first_frame_seen: bool,
    privd: Option<privd::PrivdSession>,
}

impl CaptureBackend for PipeWirePortalBackend {
    fn start(&mut self, options: &CaptureOptions) -> Result<(), EglError> {
        let ring = Arc::new(PipeWireFrameRing::new(64));
        self.ring = Some(ring);
        self.producer = None;
        self.first_frame_seen = false;
        let include_input_fds = options.mouse_tracking || options.cursor_composition;
        let privd_session = privd::acquire_device_fds(&options.card_path, include_input_fds)
            .map_err(|e| EglError::Pipeline(format!("failed to acquire device fds from privd: {e}")))?;
        self.privd = Some(privd_session);
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
        let ring = self
            .ring
            .as_ref()
            .ok_or_else(|| EglError::Pipeline("pipewire backend ring not initialized".to_string()))?;
        let enum_params = build_pipewire_enum_format_bytes(egl_i, display)
            .map_err(|e| EglError::Pipeline(format!("failed to build pipewire enum formats: {e}")))?;
        let producer = start_capture_producer(Arc::clone(ring), enum_params)
            .map_err(|e| EglError::Pipeline(format!("failed to start pipewire producer: {e}")))?;
        self.producer = Some(producer);
        Ok(())
    }

    fn kind(&self) -> CaptureBackendKind {
        CaptureBackendKind::PipewirePortal
    }

    fn next_frame(&mut self) -> Result<CaptureFrame, EglError> {
        let ring = self
            .ring
            .as_ref()
            .ok_or_else(|| EglError::Pipeline("pipewire backend not started".to_string()))?;

        let max_wait_iters = if self.first_frame_seen { 2_000 } else { usize::MAX };
        for _ in 0..max_wait_iters {
            if let Some(frame) = ring.pop_latest() {
                self.first_frame_seen = true;
                return Ok(frame);
            }
            thread::sleep(Duration::from_nanos(1));
        }

        Err(EglError::Pipeline(
            "timeout waiting for pipewire frame".to_string(),
        ))
    }

    fn stop(&mut self) -> Result<(), EglError> {
        if let Some(producer) = self.producer.take() {
            producer.stop();
        }
        self.ring = None;
        Ok(())
    }

    fn take_input_fds(&mut self) -> Option<std::collections::HashMap<std::path::PathBuf, std::os::fd::OwnedFd>> {
        self.privd.as_mut().map(|s| std::mem::take(&mut s.input_fds))
    }
}
