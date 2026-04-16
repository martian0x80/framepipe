use std::sync::{Arc, atomic::AtomicBool};

use signal_hook::consts::signal::{SIGINT, SIGTERM, SIGUSR1, SIGUSR2};
use signal_hook::flag as signal_flag;

#[derive(Clone)]
pub struct CaptureControl {
    pub stop_requested: Arc<AtomicBool>,
    pub pause_req: Arc<AtomicBool>,
    pub resume_req: Arc<AtomicBool>,
    pub paused: Arc<AtomicBool>,
}

impl CaptureControl {
    pub fn new_unregistered() -> Self {
        Self {
            stop_requested: Arc::new(AtomicBool::new(false)),
            pause_req: Arc::new(AtomicBool::new(false)),
            resume_req: Arc::new(AtomicBool::new(false)),
            paused: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn register() -> Result<Self, String> {
        let control = Self::new_unregistered();

        signal_flag::register(SIGINT, Arc::clone(&control.stop_requested))
            .map_err(|e| format!("failed to register SIGINT: {e}"))?;
        signal_flag::register(SIGTERM, Arc::clone(&control.stop_requested))
            .map_err(|e| format!("failed to register SIGTERM: {e}"))?;
        signal_flag::register(SIGUSR1, Arc::clone(&control.pause_req))
            .map_err(|e| format!("failed to register SIGUSR1: {e}"))?;
        signal_flag::register(SIGUSR2, Arc::clone(&control.resume_req))
            .map_err(|e| format!("failed to register SIGUSR2: {e}"))?;

        Ok(control)
    }
}
