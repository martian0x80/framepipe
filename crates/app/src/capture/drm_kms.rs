use crate::{
    app::signals::CaptureControl,
    capture::recording_loop,
    drm_kms::{egl_context::EglError, types::CaptureOptions},
};

use super::{backend::CaptureBackend, types::CaptureBackendKind};

pub struct DrmKmsBackend;

impl CaptureBackend for DrmKmsBackend {
    fn kind(&self) -> CaptureBackendKind {
        CaptureBackendKind::DrmKms
    }

    fn run(&self, options: CaptureOptions, control: CaptureControl) -> Result<(), EglError> {
        recording_loop::run_capture_session(options, control)
    }
}
