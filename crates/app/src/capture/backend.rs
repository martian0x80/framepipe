use crate::{
    capture::types::CaptureFrame,
    drm_kms::{egl_context::EglError, types::CaptureOptions},
    portal::notifs::{send_notification, ProcessState},
};
use khronos_egl as egl;

use super::{
    drm_kms::DrmKmsBackend, pipewire_portal::PipeWirePortalBackend, types::CaptureBackendKind,
};

/*
Backend has 4 steps
1. start -> init probe session and acquire fds (drm node and input depending on options)
2. on_egl_ready -> only relevant for pipewire backend since we need to pass egl context to pipewire portal
3. next_frame -> render frame
4. stop -> cleanup
*/

pub trait CaptureBackend: Send {
    fn start(&mut self, options: &CaptureOptions) -> Result<(), EglError>;
    fn on_egl_ready(
        &mut self,
        _egl_i: &egl::Instance<egl::Static>,
        _display: egl::Display,
    ) -> Result<(), EglError> {
        Ok(())
    }
    fn next_frame(&mut self, timeout: std::time::Duration) -> Result<Option<CaptureFrame>, EglError>;
    fn stop(&mut self) -> Result<(), EglError>;
    fn kind(&self) -> CaptureBackendKind;
    fn take_input_fds(&mut self) -> Option<std::collections::HashMap<std::path::PathBuf, std::os::fd::OwnedFd>> {
        None
    }
    // i am sorry for this but it is what it is
    fn send_notification(&mut self, state: ProcessState, timeout: u32) -> eyre::Result<()>
    {
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let _ = send_notification(&state, timeout).await;
            });
        });
        Ok(())
    }
}

pub fn build_backend(kind: CaptureBackendKind) -> Box<dyn CaptureBackend> {
    match kind {
        CaptureBackendKind::DrmKms => Box::new(DrmKmsBackend::default()),
        CaptureBackendKind::PipewirePortal => Box::new(PipeWirePortalBackend::default()),
    }
}
