use crate::{
    capture::types::CaptureFrame,
    drm_kms::{egl_context::EglError, types::CaptureOptions},
};
use khronos_egl as egl;

use super::{
    drm_kms::DrmKmsBackend, pipewire_portal::PipeWirePortalBackend, types::CaptureBackendKind,
};

pub trait CaptureBackend: Send {
    fn start(&mut self, options: &CaptureOptions) -> Result<(), EglError>;
    fn on_egl_ready(
        &mut self,
        _egl_i: &egl::Instance<egl::Static>,
        _display: egl::Display,
    ) -> Result<(), EglError> {
        Ok(())
    }
    fn next_frame(&mut self) -> Result<CaptureFrame, EglError>;
    fn stop(&mut self) -> Result<(), EglError>;
    fn kind(&self) -> CaptureBackendKind;
    fn take_input_fds(&mut self) -> Option<std::collections::HashMap<std::path::PathBuf, std::os::fd::OwnedFd>> {
        None
    }
}

pub fn build_backend(kind: CaptureBackendKind) -> Box<dyn CaptureBackend> {
    match kind {
        CaptureBackendKind::DrmKms => Box::new(DrmKmsBackend::default()),
        CaptureBackendKind::PipewirePortal => Box::new(PipeWirePortalBackend::default()),
    }
}
