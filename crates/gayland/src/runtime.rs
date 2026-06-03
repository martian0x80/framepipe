use std::collections::{HashMap, HashSet};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use input::event::Event;
use input::event::keyboard::{KeyState, KeyboardEventTrait};
use input::event::pointer::PointerEvent;
use input::{Libinput, LibinputInterface};
use log::{debug, info, warn};

use crate::config::{GaylandConfig, HotkeySpec, InputSource};
use crate::event::{GaylandEvent, StateEvent};
use crate::keyboard::key_info;

type Subscribers = Arc<Mutex<Vec<Sender<GaylandEvent>>>>;

#[derive(Default)]
struct RuntimeState {
    anchored: bool,
    anchor_edge_pending: bool,
    x: f64,
    y: f64,
    max_x: Option<f64>,
    max_y: Option<f64>,
    anchor_epoch: u64,
    pressed_keys: HashSet<u32>,
    active_hotkeys: HashSet<u64>,
    hotkeys: HashMap<u64, HotkeySpec>,
}

pub(crate) enum Command {
    Pause(bool),
    Stop,
    SetBounds { width: f64, height: f64 },
    SetAnchor { x: f64, y: f64 },
    ClearAnchor,
    RegisterHotkey { id: u64, spec: HotkeySpec },
    UnregisterHotkey { id: u64 },
}

pub struct GaylandHandle {
    controller: GaylandController,
    subscribers: Subscribers,
    finished: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

#[derive(Clone)]
pub struct GaylandController {
    tx: Sender<Command>,
}

impl GaylandController {
    pub fn pause(&self) {
        let _ = self.tx.send(Command::Pause(true));
    }

    pub fn resume(&self) {
        let _ = self.tx.send(Command::Pause(false));
    }

    pub fn stop(&self) {
        let _ = self.tx.send(Command::Stop);
    }

    pub fn set_bounds(&self, width: f64, height: f64) {
        let _ = self.tx.send(Command::SetBounds { width, height });
    }

    pub fn set_anchor(&self, x: f64, y: f64) {
        let _ = self.tx.send(Command::SetAnchor { x, y });
    }

    pub fn clear_anchor(&self) {
        let _ = self.tx.send(Command::ClearAnchor);
    }

    pub fn register_hotkey(&self, id: u64, spec: HotkeySpec) {
        let _ = self.tx.send(Command::RegisterHotkey { id, spec });
    }

    pub fn unregister_hotkey(&self, id: u64) {
        let _ = self.tx.send(Command::UnregisterHotkey { id });
    }
}

impl GaylandHandle {
    pub fn start(config: GaylandConfig, source: InputSource) -> Result<Self, String> {
        let (handle, _events) = start(config, source)?;
        Ok(handle)
    }

    pub fn controller(&self) -> GaylandController {
        self.controller.clone()
    }

    pub fn subscribe(&self) -> Receiver<GaylandEvent> {
        let (tx, rx) = mpsc::channel();
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.push(tx);
        }
        rx
    }

    pub fn pause(&self) {
        self.controller.pause();
    }

    pub fn resume(&self) {
        self.controller.resume();
    }

    pub fn stop(&self) {
        self.controller.stop();
    }

    pub fn set_bounds(&self, width: f64, height: f64) {
        self.controller.set_bounds(width, height);
    }

    pub fn set_anchor(&self, x: f64, y: f64) {
        self.controller.set_anchor(x, y);
    }

    pub fn clear_anchor(&self) {
        self.controller.clear_anchor();
    }

    pub fn register_hotkey(&self, id: u64, spec: HotkeySpec) {
        self.controller.register_hotkey(id, spec);
    }

    pub fn unregister_hotkey(&self, id: u64) {
        self.controller.unregister_hotkey(id);
    }

    pub fn join(mut self) {
        self.stop();
        if let Some(h) = self.join.take() {
            let _ = h.join();
        }
    }
}

impl Drop for GaylandHandle {
    fn drop(&mut self) {
        self.stop();
        if let Some(h) = self.join.take() {
            let deadline = Instant::now() + Duration::from_millis(750);
            while !self.finished.load(Ordering::Acquire) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            if self.finished.load(Ordering::Acquire) {
                let _ = h.join();
            }
        }
    }
}

enum LibinputIface {
    Preopened {
        fds_by_path: HashMap<PathBuf, OwnedFd>,
    },
    Direct,
}

impl LibinputInterface for LibinputIface {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<OwnedFd, i32> {
        match self {
            LibinputIface::Preopened { fds_by_path } => {
                let source = fds_by_path.get(path).ok_or(libc::ENOENT)?;
                let dup_fd = unsafe { libc::fcntl(source.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
                if dup_fd < 0 {
                    return Err(std::io::Error::last_os_error()
                        .raw_os_error()
                        .unwrap_or(libc::EIO));
                }
                let status = flags & (libc::O_NONBLOCK | libc::O_APPEND | libc::O_ASYNC);
                if unsafe { libc::fcntl(dup_fd, libc::F_SETFL, status) } < 0 {
                    let e = std::io::Error::last_os_error();
                    unsafe { libc::close(dup_fd) };
                    return Err(e.raw_os_error().unwrap_or(libc::EIO));
                }
                Ok(unsafe { OwnedFd::from_raw_fd(dup_fd) })
            }
            LibinputIface::Direct => {
                let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
                    .map_err(|_| libc::EINVAL)?;
                let fd = unsafe { libc::open(c_path.as_ptr(), flags) };
                if fd < 0 {
                    let err = std::io::Error::last_os_error();
                    warn!(
                        "gayland: failed to open input device {}: {err}",
                        path.display()
                    );
                    return Err(err.raw_os_error().unwrap_or(libc::EIO));
                }
                Ok(unsafe { OwnedFd::from_raw_fd(fd) })
            }
        }
    }

    fn close_restricted(&mut self, fd: OwnedFd) {
        drop(fd);
    }
}

/// Start the libinput runtime.
///
/// Event flow:
/// - libinput pointer/keyboard events are read in this thread and emitted as [`GaylandEvent`].
/// - absolute cursor anchoring enters through [`GaylandHandle::set_anchor`], typically from the
///   optional layer-shell helper.
/// - anchored absolute state and libinput deltas are merged here into `MousePosition` events.
pub fn start(
    config: GaylandConfig,
    source: InputSource,
) -> Result<(GaylandHandle, Receiver<GaylandEvent>), String> {
    let (event_tx, event_rx) = mpsc::channel();
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let subscribers = Arc::new(Mutex::new(vec![event_tx]));
    let thread_subscribers = subscribers.clone();
    let finished = Arc::new(AtomicBool::new(false));
    let finished_clone = finished.clone();

    let thread = thread::Builder::new()
        .name("gayland-runtime".into())
        .spawn(move || {
            let mut state = RuntimeState {
                hotkeys: config.hotkeys.clone(),
                ..Default::default()
            };
            let iface = match source {
                InputSource::Preopened { fds_by_path } => {
                    info!("gayland: using {} preopened fds", fds_by_path.len());
                    LibinputIface::Preopened { fds_by_path }
                }
                InputSource::DirectOpen => LibinputIface::Direct,
            };

            let mut li = Libinput::new_with_udev(iface);
            if let Err(e) = li.udev_assign_seat(&config.seat) {
                warn!("gayland: udev_assign_seat failed: {e:?}");
                finished_clone.store(true, Ordering::Release);
                return;
            }

            info!(
                "gayland runtime started: seat={} mouse={} keyboard={} hotkeys={} batch_window={:?}",
                config.seat,
                config.enable_mouse,
                config.enable_keyboard,
                state.hotkeys.len(),
                config.batch_window
            );
            emit_event(
                &thread_subscribers,
                GaylandEvent::State(StateEvent::Started),
            );

            let start = Instant::now();
            let mut paused = false;
            let mut batch_start = Instant::now();
            let mut batch_dx = 0.0;
            let mut batch_dy = 0.0;
            let mut local_stop = false;
            let mut seen_anchor_epoch = 0_u64;
            let mut drain_pending_events = false;

            while !local_stop {
                while let Ok(cmd) = cmd_rx.try_recv() {
                    match cmd {
                        Command::Pause(p) => {
                            paused = p;
                            emit_event(
                                &thread_subscribers,
                                GaylandEvent::State(if p {
                                    StateEvent::Paused
                                } else {
                                    StateEvent::Resumed
                                }),
                            );
                        }
                        Command::Stop => {
                            local_stop = true;
                            break;
                        }
                        Command::SetBounds { width, height } => {
                            state.max_x = Some(width);
                            state.max_y = Some(height);
                            clamp_bounds(&mut state);
                            emit_event(
                                &thread_subscribers,
                                GaylandEvent::BoundsChanged {
                                    max_x: state.max_x,
                                    max_y: state.max_y,
                                },
                            );
                        }
                        Command::SetAnchor { x, y } => {
                            state.x = x;
                            state.y = y;
                            state.anchored = true;
                            state.anchor_edge_pending = true;
                            state.anchor_epoch = state.anchor_epoch.wrapping_add(1);
                            clamp_bounds(&mut state);
                            emit_event(
                                &thread_subscribers,
                                GaylandEvent::AnchorChanged {
                                    x: state.x,
                                    y: state.y,
                                },
                            );
                        }
                        Command::ClearAnchor => {
                            state.anchored = false;
                            state.anchor_edge_pending = false;
                        }
                        Command::RegisterHotkey { id, spec } => {
                            debug!(
                                "gayland hotkey registered at runtime: id={id} keys={:?}",
                                spec.keys
                            );
                            state.hotkeys.insert(id, spec);
                        }
                        Command::UnregisterHotkey { id } => {
                            debug!("gayland hotkey unregistered at runtime: id={id}");
                            state.hotkeys.remove(&id);
                            state.active_hotkeys.remove(&id);
                        }
                    }
                }

                if local_stop {
                    break;
                }

                {
                    if state.anchor_epoch != seen_anchor_epoch {
                        seen_anchor_epoch = state.anchor_epoch;
                        batch_dx = 0.0;
                        batch_dy = 0.0;
                        let t_ns = start.elapsed().as_nanos().min(u64::MAX as u128) as u64;
                        let anchor_edge = state.anchor_edge_pending;
                        state.anchor_edge_pending = false;
                        emit_event(
                            &thread_subscribers,
                            GaylandEvent::MousePosition {
                                x: state.x,
                                y: state.y,
                                max_x: state.max_x,
                                max_y: state.max_y,
                                anchored: anchor_edge,
                                t_ns,
                            },
                        );
                    }
                }

                let elapsed = batch_start.elapsed();
                let timeout_ms = if drain_pending_events || elapsed >= config.batch_window {
                    0
                } else {
                    (config.batch_window - elapsed).as_millis().max(1) as i32
                };

                let mut pfd = libc::pollfd {
                    fd: li.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                };
                let poll_rc = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
                if poll_rc < 0 {
                    warn!("gayland: poll error: {}", std::io::Error::last_os_error());
                    continue;
                }

                let fd_readable = poll_rc > 0 && (pfd.revents & libc::POLLIN) != 0;
                if fd_readable || drain_pending_events {
                    if fd_readable && let Err(e) = li.dispatch() {
                        warn!("gayland: dispatch error: {e}");
                        continue;
                    }
                    const MAX_EVENTS_PER_TICK: usize = 256;
                    let mut processed_events = 0_usize;
                    for ev in (&mut li).take(MAX_EVENTS_PER_TICK) {
                        processed_events += 1;
                        match ev {
                            Event::Pointer(ref ptr) if config.enable_mouse => match ptr {
                                PointerEvent::Motion(m) => {
                                    let dx = m.dx();
                                    let dy = m.dy();
                                    batch_dx += dx;
                                    batch_dy += dy;
                                    let t_ns =
                                        start.elapsed().as_nanos().min(u64::MAX as u128) as u64;
                                    emit_event(
                                        &thread_subscribers,
                                        GaylandEvent::MouseDelta { dx, dy, t_ns },
                                    );
                                }
                                PointerEvent::Button(b) => {
                                    let t_ns =
                                        start.elapsed().as_nanos().min(u64::MAX as u128) as u64;
                                    emit_event(
                                        &thread_subscribers,
                                        GaylandEvent::MouseButton {
                                            button: b.button(),
                                            pressed: b.button_state()
                                                == input::event::pointer::ButtonState::Pressed,
                                            t_ns,
                                        },
                                    );
                                }
                                _ => {}
                            },
                            Event::Keyboard(ref key) if config.enable_keyboard => {
                                let keycode = key.key();
                                let pressed = key.key_state() == KeyState::Pressed;
                                let t_ns = start.elapsed().as_nanos().min(u64::MAX as u128) as u64;
                                let info = key_info(keycode);
                                let changed = if pressed {
                                    state.pressed_keys.insert(keycode)
                                } else {
                                    state.pressed_keys.remove(&keycode)
                                };
                                if changed {
                                    emit_event(
                                        &thread_subscribers,
                                        GaylandEvent::KeyboardKey {
                                            keycode,
                                            key_name: info.name,
                                            is_modifier: info.is_modifier,
                                            pressed,
                                            t_ns,
                                        },
                                    );
                                    evaluate_hotkeys(
                                        &mut state,
                                        t_ns,
                                        &thread_subscribers,
                                    );
                                }
                            }
                            _ => {}
                        }
                    }
                    drain_pending_events = processed_events == MAX_EVENTS_PER_TICK;
                }

                if batch_start.elapsed() >= config.batch_window {
                    if (batch_dx != 0.0 || batch_dy != 0.0) && state.anchored && !paused {
                        state.x += batch_dx;
                        state.y += batch_dy;
                        clamp_bounds(&mut state);
                        let t_ns = start.elapsed().as_nanos().min(u64::MAX as u128) as u64;
                        emit_event(
                            &thread_subscribers,
                            GaylandEvent::MousePosition {
                                x: state.x,
                                y: state.y,
                                max_x: state.max_x,
                                max_y: state.max_y,
                                anchored: false,
                                t_ns,
                            },
                        );
                    }
                    batch_dx = 0.0;
                    batch_dy = 0.0;
                    batch_start = Instant::now();
                }
            }

            emit_event(
                &thread_subscribers,
                GaylandEvent::State(StateEvent::Stopped),
            );
            info!("gayland runtime stopped");
            finished_clone.store(true, Ordering::Release);
        })
        .map_err(|e| format!("failed to spawn gayland runtime: {e}"))?;

    Ok((
        GaylandHandle {
            controller: GaylandController { tx: cmd_tx },
            subscribers,
            finished,
            join: Some(thread),
        },
        event_rx,
    ))
}

fn emit_event(subscribers: &Subscribers, event: GaylandEvent) {
    if let Ok(mut subscribers) = subscribers.lock() {
        subscribers.retain(|subscriber| subscriber.send(event).is_ok());
    }
}

fn is_hotkey_exact_match(st: &RuntimeState, spec: &HotkeySpec) -> bool {
    !spec.is_empty()
        && st.pressed_keys.len() == spec.keys.len()
        && spec.keys.iter().all(|k| st.pressed_keys.contains(k))
}

fn is_hotkey_held(st: &RuntimeState, spec: &HotkeySpec) -> bool {
    !spec.is_empty() && spec.keys.iter().all(|k| st.pressed_keys.contains(k))
}

fn evaluate_hotkeys(st: &mut RuntimeState, t_ns: u64, subscribers: &Subscribers) {
    let mut now_active = HashSet::new();
    for (id, spec) in &st.hotkeys {
        if is_hotkey_held(st, spec) {
            now_active.insert(*id);
            if is_hotkey_exact_match(st, spec) && !st.active_hotkeys.contains(id) {
                debug!(
                    "gayland hotkey exact match: id={} keys={:?} pressed={:?}",
                    id, spec.keys, st.pressed_keys
                );
                emit_event(subscribers, GaylandEvent::HotkeyTriggered { id: *id, t_ns });
            }
        }
    }
    st.active_hotkeys = now_active;
}

fn clamp_bounds(st: &mut RuntimeState) {
    if let Some(max_x) = st.max_x {
        st.x = st.x.clamp(0.0, max_x.max(0.0));
    }
    if let Some(max_y) = st.max_y {
        st.y = st.y.clamp(0.0, max_y.max(0.0));
    }
}
