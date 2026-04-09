use eyre::Result;

use super::signals::CaptureControl;
use crate::capture::{capture, types::CaptureBackendKind};
use crate::drm_kms::types::CaptureOptions;

pub struct RecordingSession {
    options: CaptureOptions,
    backend: CaptureBackendKind,
}

impl RecordingSession {
    pub fn new(options: CaptureOptions, backend: CaptureBackendKind) -> Result<Self> {
        Ok(Self { options, backend })
    }

    pub fn run(self, control: CaptureControl) -> Result<()> {
        capture::run_capture_session(self.options, control, self.backend).map_err(Into::into)
    }
}
