use eyre::Result;

use crate::drm_kms::{egl, types::CaptureOptions};

pub trait CapturePipeline: Send + Sync {
    fn run(&self, options: CaptureOptions) -> Result<()>;
}

#[derive(Default)]
pub struct DrmEglPipeline;

impl CapturePipeline for DrmEglPipeline {
    fn run(&self, options: CaptureOptions) -> Result<()> {
        egl::egl_main(options).map_err(Into::into)
    }
}
