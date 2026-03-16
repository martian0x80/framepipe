use eyre::Result;

use crate::drm_kms::{recording_loop, types::CaptureOptions};

pub trait CapturePipeline: Send + Sync {
    fn run(&self, options: CaptureOptions) -> Result<()>;
}

#[derive(Default)]
pub struct DrmEglPipeline;

impl CapturePipeline for DrmEglPipeline {
    fn run(&self, options: CaptureOptions) -> Result<()> {
        recording_loop::run_capture_session(options).map_err(Into::into)
    }
}
