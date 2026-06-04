use std::{
    collections::HashMap,
    io::{BufWriter, Write},
    os::fd::OwnedFd,
    path::{Path, PathBuf},
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use bitcode::{Decode, Encode};
use gayland::{
    GaylandConfig, GaylandEvent, InputSource, LayerShellConfig, TrackerConfig, WaylandError,
    start_tracker,
};

use crate::shared::mouse_ring::{MouseEvent, RingBuffer};
use crate::{
    app::{
        hotkeys::{HotkeyBinding, apply_action},
        signals::CaptureControl,
    },
    shared::keyboard_ring::{KeyOverlayEvent, KeyboardOverlayRingBuffer},
};

#[derive(Clone)]
pub struct TrackingControl {
    pub capture: CaptureControl,
}

impl TrackingControl {
    pub fn new(capture: CaptureControl) -> Self {
        Self { capture }
    }
}

#[derive(Debug, Clone, Encode, Decode, Default)]
pub struct MouseTrackRecordingInfo {
    pub started_unix_ms: u64,
    pub output_path: Option<String>,
    pub card_path: Option<String>,
    pub connector: Option<String>,
    pub fps: Option<u32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub encoder_backend: Option<String>,
    pub video_codec: Option<String>,
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct MouseSampleRecord {
    pub t_ns: u64,
    pub x: f64,
    pub y: f64,
    pub anchored: bool,
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct MouseTrackHeader {
    pub version: u32,
    pub recording: MouseTrackRecordingInfo,
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct MouseTrackChunk {
    pub samples: Vec<MouseSampleRecord>,
}

pub fn init_wayland(
    sync_frequency_hz: f64,
    file_path: &Path,
    control: TrackingControl,
    recording: MouseTrackRecordingInfo,
    ring: Option<Arc<RingBuffer>>,
    keyboard_ring: Option<Arc<KeyboardOverlayRingBuffer>>,
    input_fds: Option<HashMap<PathBuf, OwnedFd>>,
    hotkeys: Vec<HotkeyBinding>,
    enable_layer_shell: bool,
) -> Result<(), WaylandError> {
    log::info!("Initializing Wayland connection and event loop");
    if enable_layer_shell {
        log::info!(
            "Mouse tracking output file: {}",
            file_path.to_string_lossy()
        );
    }

    let input_source = input_fds
        .map(|fds_by_path| InputSource::Preopened { fds_by_path })
        .ok_or_else(|| {
            WaylandError::RuntimeStartFailed(
                "preopened input fds are required for wayland tracking".to_string(),
            )
        })?;

    let enable_keyboard = keyboard_ring.is_some() || !hotkeys.is_empty();
    let mut runtime_config = GaylandConfig {
        enable_mouse: enable_layer_shell,
        enable_keyboard,
        ..Default::default()
    };
    let mut hotkey_actions = HashMap::with_capacity(hotkeys.len());
    for (idx, binding) in hotkeys.into_iter().enumerate() {
        let id = idx as u64 + 1;
        runtime_config.hotkeys.insert(id, binding.spec);
        hotkey_actions.insert(id, binding.action);
    }

    let config = TrackerConfig::new(input_source).with_runtime(runtime_config);
    let config = if enable_layer_shell {
        config.with_layer_shell(LayerShellConfig {
            namespace: "openstudio".to_string(),
            sync_frequency_hz,
        })
    } else {
        config
    };
    let tracker = start_tracker(config).map_err(|e| match e {
        gayland::TrackerError::LayerShell(e) => e,
        other => WaylandError::RuntimeStartFailed(other.to_string()),
    })?;

    let mut sink = ring
        .is_some()
        .then(|| FramepipeMouseSink::new(file_path, recording, ring))
        .transpose()
        .map_err(|_| WaylandError::ConnectionFailed)?;
    let mut paused = control.capture.paused.load(Ordering::Relaxed);

    loop {
        if control.capture.stop_requested.load(Ordering::Relaxed) {
            log::info!("Input tracking stop requested");
            tracker.stop();
            break;
        }

        let now_paused = control.capture.paused.load(Ordering::Relaxed);
        if now_paused != paused {
            if now_paused {
                tracker.pause();
                if let Some(sink) = &mut sink {
                    sink.set_paused(true);
                }
            } else {
                tracker.resume();
                if let Some(sink) = &mut sink {
                    sink.set_paused(false);
                }
            }
            paused = now_paused;
        }

        match tracker.events.recv_timeout(Duration::from_millis(10)) {
            Ok(event) => match event {
                GaylandEvent::HotkeyTriggered { id, .. } => {
                    if let Some(action) = hotkey_actions.get(&id).copied() {
                        log::debug!("recording hotkey triggered: id={id} action={action}");
                        apply_action(action, &control.capture);
                    }
                }
                GaylandEvent::KeyboardKey {
                    keycode,
                    key_name,
                    is_modifier,
                    pressed,
                    t_ns,
                } => {
                    if let Some(keyboard_ring) = &keyboard_ring {
                        keyboard_ring.push(KeyOverlayEvent {
                            t_ns,
                            keycode,
                            key_name: key_name.unwrap_or("unknown"),
                            is_modifier,
                            pressed,
                        });
                    }
                }
                // mouse pos sink, probably should move type out
                event => {
                    if let Some(sink) = &mut sink {
                        sink.handle_event(event);
                    }
                }
            },
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    if let Some(sink) = &mut sink {
        sink.flush();
    }
    tracker.join().map_err(|e| match e {
        gayland::TrackerError::LayerShell(e) => e,
        other => WaylandError::RuntimeStartFailed(other.to_string()),
    })?;
    if sink.is_some() {
        log::info!(
            "Mouse tracking event loop exiting, data written to {}",
            file_path.display()
        );
    } else {
        log::info!("Input tracking event loop exiting");
    }
    Ok(())
}

struct FramepipeMouseSink {
    ring: Option<Arc<RingBuffer>>,
    writer: BufWriter<std::fs::File>,
    pending: Vec<MouseSampleRecord>,
    paused: bool,
}

impl FramepipeMouseSink {
    fn new(
        path: &Path,
        recording: MouseTrackRecordingInfo,
        ring: Option<Arc<RingBuffer>>,
    ) -> Result<Self, std::io::Error> {
        let file = std::fs::File::create(path)?;
        let mut writer = BufWriter::new(file);
        let header = MouseTrackHeader {
            version: 1,
            recording,
        };
        write_framed(&mut writer, &bitcode::encode(&header))?;
        Ok(Self {
            ring,
            writer,
            pending: Vec::with_capacity(512),
            paused: false,
        })
    }

    fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        if paused {
            self.flush();
        }
    }

    fn handle_event(&mut self, event: GaylandEvent) {
        if let GaylandEvent::MousePosition {
            x,
            y,
            max_x,
            max_y,
            anchored,
            t_ns,
        } = event
        {
            if let Some(ring) = &self.ring {
                ring.push(MouseEvent {
                    t_ns,
                    x,
                    y,
                    max_x,
                    max_y,
                });
            }
            if !self.paused {
                self.pending.push(MouseSampleRecord {
                    t_ns,
                    x,
                    y,
                    anchored,
                });
                if self.pending.len() >= 512 {
                    self.flush();
                }
            }
        }
    }

    fn flush(&mut self) {
        if !self.pending.is_empty() {
            let chunk = MouseTrackChunk {
                samples: std::mem::take(&mut self.pending),
            };
            let _ = write_framed(&mut self.writer, &bitcode::encode(&chunk));
        }
        let _ = self.writer.flush();
    }
}

impl Drop for FramepipeMouseSink {
    fn drop(&mut self) {
        self.flush();
    }
}

fn write_framed<W: Write>(writer: &mut W, payload: &[u8]) -> Result<(), std::io::Error> {
    let len = payload.len() as u32;
    writer.write_all(&len.to_le_bytes())?;
    writer.write_all(payload)
}
