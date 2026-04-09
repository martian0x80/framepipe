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
    let mut backend_impl = build_backend(backend);
    log::info!("capture backend selected: {}", backend_impl.kind());
    backend_impl.start(&options)?;
    let result = super::recording_loop::run_capture_session(
        options,
        control,
        backend_impl.as_mut(),
    );
    let stop_result = backend_impl.stop();
    match (result, stop_result) {
        (Err(e), _) => Err(e),
        (Ok(_), Err(e)) => Err(e),
        (Ok(_), Ok(_)) => Ok(()),
    }
}
