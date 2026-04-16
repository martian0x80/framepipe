use std::sync::atomic::Ordering;
use std::thread;

use eyre::Result;

use crate::{
    app::{cli::CaptureArgs, config, signals::CaptureControl},
    capture,
    drm_kms::types::CaptureOutput,
};

pub struct EmbeddedPreviewSession {
    mailbox: common::types::PreviewMailbox,
    control: CaptureControl,
    worker: Option<thread::JoinHandle<Result<()>>>,
}

impl EmbeddedPreviewSession {
    pub fn mailbox(&self) -> common::types::PreviewMailbox {
        self.mailbox.clone()
    }

    pub fn stop(&self) {
        self.control.stop_requested.store(true, Ordering::Relaxed);
    }

    pub fn join(mut self) -> Result<()> {
        self.stop();
        if let Some(worker) = self.worker.take() {
            return worker
                .join()
                .map_err(|_| eyre::eyre!("embedded preview thread panicked"))?;
        }
        Ok(())
    }
}

impl Drop for EmbeddedPreviewSession {
    fn drop(&mut self) {
        self.stop();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub fn start_embedded_preview(capture: CaptureArgs) -> Result<EmbeddedPreviewSession> {
    let backend = capture.capture_backend;
    let mut options = config::build_capture_options(capture, CaptureOutput::EmbeddedPreview)?;
    let mailbox = common::types::PreviewMailbox::new();
    options.preview_mailbox = Some(mailbox.clone());

    let control = CaptureControl::new_unregistered();
    let worker_control = control.clone();
    let worker = thread::spawn(move || {
        capture::capture::run_capture_session(options, worker_control, backend)
            .map_err(|e| eyre::eyre!(e.to_string()))
    });

    Ok(EmbeddedPreviewSession {
        mailbox,
        control,
        worker: Some(worker),
    })
}

pub fn start_embedded_preview_default() -> Result<EmbeddedPreviewSession> {
    let mut args = CaptureArgs::default();
    // args.fps = 20;
    // args.output_width = Some(1280);
    // args.output_height = Some(720);
    start_embedded_preview(args)
}
