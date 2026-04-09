use crate::{
    app::signals::CaptureControl,
    drm_kms::{egl_context::EglError, types::CaptureOptions},
};

use super::{
    drm_kms::DrmKmsBackend, pipewire_portal::PipeWirePortalBackend, types::CaptureBackendKind,
};

pub trait CaptureBackend: Send {
    fn kind(&self) -> CaptureBackendKind;
    fn run(&self, options: CaptureOptions, control: CaptureControl) -> Result<(), EglError>;
}

pub fn build_backend(kind: CaptureBackendKind) -> Box<dyn CaptureBackend> {
    match kind {
        CaptureBackendKind::DrmKms => Box::new(DrmKmsBackend),
        CaptureBackendKind::PipewirePortal => Box::new(PipeWirePortalBackend),
    }
}
