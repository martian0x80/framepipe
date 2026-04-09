use crate::{
    app::signals::CaptureControl,
    drm_kms::{egl_context::EglError, types::CaptureOptions},
};

use super::{backend::build_backend, types::CaptureBackendKind};

pub fn run_capture_session(
    options: CaptureOptions,
    control: CaptureControl,
    backend: CaptureBackendKind,
) -> Result<(), EglError> {
    let backend_impl = build_backend(backend);
    log::info!("capture backend selected: {}", backend_impl.kind());
    backend_impl.run(options, control)
}
