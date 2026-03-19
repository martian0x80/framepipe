use eyre::Result;

use crate::drm_kms::{recording_loop, types::CaptureOptions};
use crate::app::signals::CaptureControl;

pub trait CapturePipeline: Send + Sync {
    fn run(&self, options: CaptureOptions, control: CaptureControl) -> Result<()>;
}

#[derive(Default)]
pub struct DrmEglPipeline;

impl CapturePipeline for DrmEglPipeline {
    fn run(&self, options: CaptureOptions, control: CaptureControl) -> Result<()> {
        recording_loop::run_capture_session(options, control).map_err(Into::into)
    }
}
