use eyre::Result;

use crate::drm_kms::types::{CaptureOptions, CaptureOutput};

use super::{
    pipeline::{CapturePipeline, DrmEglPipeline},
    signals::CaptureControl,
};

pub struct RecordingSession {
    options: CaptureOptions,
    pipeline: Box<dyn CapturePipeline>,
}

impl RecordingSession {
    pub fn new(options: CaptureOptions) -> Result<Self> {
        Ok(Self {
            options,
            pipeline: Box::<DrmEglPipeline>::default(),
        })
    }

    pub fn with_pipeline(options: CaptureOptions, pipeline: Box<dyn CapturePipeline>) -> Self {
        Self { options, pipeline }
    }

    pub fn run(self, control: CaptureControl) -> Result<()> {
        self.pipeline.run(self.options, control)
    }

    pub fn is_preview(&self) -> bool {
        matches!(self.options.output, CaptureOutput::Preview)
    }
}
