use crate::{
    app::signals::CaptureControl,
    drm_kms::{egl_context::EglError, types::CaptureOptions},
};

use super::{backend::CaptureBackend, types::CaptureBackendKind};

pub struct PipeWirePortalBackend;

impl CaptureBackend for PipeWirePortalBackend {
    fn kind(&self) -> CaptureBackendKind {
        CaptureBackendKind::PipewirePortal
    }

    fn run(&self, _options: CaptureOptions, _control: CaptureControl) -> Result<(), EglError> {
        Err(EglError::Pipeline(
            "pipewire portal wip".to_string(),
        ))
    }
}
