#[cfg(feature = "layer-shell")]
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver};
#[cfg(feature = "layer-shell")]
use std::thread::JoinHandle;

use crate::config::{GaylandConfig, InputSource};
use crate::event::GaylandEvent;
use crate::runtime::{self, GaylandHandle};

#[cfg(feature = "layer-shell")]
use crate::layer_shell::{LayerShellConfig, LayerShellControl, WaylandError, spawn_layer_shell};

#[derive(Debug, thiserror::Error)]
pub enum TrackerError {
    #[error("failed to start input runtime: {0}")]
    Runtime(String),
    #[cfg(feature = "layer-shell")]
    #[error(transparent)]
    LayerShell(#[from] WaylandError),
    #[error("tracker thread panicked")]
    ThreadPanic,
}

pub struct TrackerConfig {
    pub runtime: GaylandConfig,
    pub input_source: InputSource,
    #[cfg(feature = "layer-shell")]
    pub layer_shell: Option<LayerShellConfig>,
}

impl TrackerConfig {
    pub fn new(input_source: InputSource) -> Self {
        Self {
            runtime: GaylandConfig::default(),
            input_source,
            #[cfg(feature = "layer-shell")]
            layer_shell: None,
        }
    }

    #[cfg(feature = "layer-shell")]
    pub fn with_layer_shell(mut self, layer_shell: LayerShellConfig) -> Self {
        self.layer_shell = Some(layer_shell);
        self
    }
}

pub struct TrackerSession {
    pub events: Receiver<GaylandEvent>,
    runtime: Option<GaylandHandle>,
    #[cfg(feature = "layer-shell")]
    layer_control: Option<LayerShellControl>,
    #[cfg(feature = "layer-shell")]
    layer_thread: Option<JoinHandle<Result<(), WaylandError>>>,
}

impl TrackerSession {
    pub fn start(config: TrackerConfig) -> Result<Self, TrackerError> {
        let (event_tx, event_rx) = mpsc::channel();
        let sink = Box::new(move |event: &GaylandEvent| {
            let _ = event_tx.send(*event);
        });
        let (runtime, runtime_events) =
            runtime::start_with_sink(config.runtime, config.input_source, Some(sink))
                .map_err(TrackerError::Runtime)?;

        #[cfg(feature = "layer-shell")]
        {
            let mut layer_control = None;
            let mut layer_thread = None;
            if let Some(layer_shell) = config.layer_shell {
                let control = LayerShellControl::idle();
                let thread = spawn_layer_shell(
                    layer_shell,
                    control.clone(),
                    runtime.controller(),
                    runtime_events,
                )?;
                layer_control = Some(control);
                layer_thread = Some(thread);
            } else {
                drop(runtime_events);
            }

            Ok(Self {
                events: event_rx,
                runtime: Some(runtime),
                layer_control,
                layer_thread,
            })
        }

        #[cfg(not(feature = "layer-shell"))]
        {
            drop(runtime_events);
            Ok(Self {
                events: event_rx,
                runtime: Some(runtime),
            })
        }
    }

    pub fn pause(&self) {
        #[cfg(feature = "layer-shell")]
        if let Some(control) = &self.layer_control {
            control.paused.store(true, Ordering::Relaxed);
        }
        if let Some(runtime) = &self.runtime {
            runtime.pause();
        }
    }

    pub fn resume(&self) {
        #[cfg(feature = "layer-shell")]
        if let Some(control) = &self.layer_control {
            control.paused.store(false, Ordering::Relaxed);
        }
        if let Some(runtime) = &self.runtime {
            runtime.resume();
        }
    }

    pub fn stop(&self) {
        #[cfg(feature = "layer-shell")]
        if let Some(control) = &self.layer_control {
            control.stop_requested.store(true, Ordering::Relaxed);
        }
        if let Some(runtime) = &self.runtime {
            runtime.stop();
        }
    }

    pub fn join(mut self) -> Result<(), TrackerError> {
        self.stop();
        #[cfg(feature = "layer-shell")]
        if let Some(thread) = self.layer_thread.take() {
            thread.join().map_err(|_| TrackerError::ThreadPanic)??;
        }
        if let Some(runtime) = self.runtime.take() {
            runtime.join();
        }
        Ok(())
    }
}

impl Drop for TrackerSession {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn start_tracker(config: TrackerConfig) -> Result<TrackerSession, TrackerError> {
    TrackerSession::start(config)
}

#[cfg(feature = "layer-shell")]
pub fn start_layer_shell(
    input_source: InputSource,
    layer_shell: LayerShellConfig,
) -> Result<TrackerSession, TrackerError> {
    start_tracker(TrackerConfig::new(input_source).with_layer_shell(layer_shell))
}
