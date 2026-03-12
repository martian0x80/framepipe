use std::collections::VecDeque;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use input::event::Event;
use input::event::pointer::PointerEvent;
use input::{Libinput, LibinputInterface};
use log::{debug, warn};

#[derive(Debug, Clone, Copy)]
pub struct MouseSample {
    pub x: f64,
    pub y: f64,
    pub anchored: bool,
}

#[derive(Default)]
struct MouseState {
    anchored: bool,
    x: f64,
    y: f64,
    max_x: Option<f64>,
    max_y: Option<f64>,
    history: VecDeque<(Instant, f64, f64)>,
}

struct LibinputIface;

impl LibinputInterface for LibinputIface {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<OwnedFd, i32> {
        let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
            .map_err(|_| libc::EINVAL)?;
        let fd = unsafe { libc::open(c_path.as_ptr(), flags) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EIO));
        }
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    fn close_restricted(&mut self, fd: OwnedFd) {
        let raw = fd.as_raw_fd();
        drop(fd);
        let _ = raw;
    }
}

pub struct MouseTracker {
    state: Arc<Mutex<MouseState>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MouseTracker {
    fn clamp_to_bounds(st: &mut MouseState) {
        if let Some(max_x) = st.max_x {
            st.x = st.x.clamp(0.0, max_x.max(0.0));
        }
        if let Some(max_y) = st.max_y {
            st.y = st.y.clamp(0.0, max_y.max(0.0));
        }
    }

    pub fn start() -> Result<Self, String> {
        let state = Arc::new(Mutex::new(MouseState::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let state_clone = Arc::clone(&state);
        let stop_clone = Arc::clone(&stop);

        let thread = thread::Builder::new()
            .name("mouse-tracker-libinput".to_string())
            .spawn(move || {
                let mut li = Libinput::new_with_udev(LibinputIface);
                if let Err(e) = li.udev_assign_seat("seat0") {
                    warn!("libinput: failed to assign seat0: {:?}", e);
                    return;
                }

                while !stop_clone.load(Ordering::Relaxed) {
                    if let Err(e) = li.dispatch() {
                        warn!("libinput dispatch error: {e}");
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        continue;
                    }

                    for ev in &mut li {
                        let Event::Pointer(ptr) = ev else {
                            continue;
                        };
                        let (dx, dy) = match ptr {
                            PointerEvent::Motion(m) => (m.dx(), m.dy()),
                            _ => continue,
                        };
                        let now = Instant::now();
                        let mut st = state_clone.lock().expect("mouse tracker mutex poisoned");
                        st.history.push_back((now, dx, dy));
                        while st.history.len() > 4096 {
                            st.history.pop_front();
                        }
                        if st.anchored {
                            st.x += dx;
                            st.y += dy;
                            Self::clamp_to_bounds(&mut st);
                            debug!(
                                "mouse tracker calc pos -> ({:.2}, {:.2}) [delta=({:.3}, {:.3})]",
                                st.x, st.y, dx, dy
                            );
                        }
                    }

                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            })
            .map_err(|e| format!("failed to spawn libinput thread: {e}"))?;

        Ok(Self {
            state,
            stop,
            thread: Some(thread),
        })
    }

    pub fn anchor_absolute(&self, x: f64, y: f64, at: Instant) {
        let mut st = self.state.lock().expect("mouse tracker mutex poisoned");
        let (dx_sum, dy_sum) = st
            .history
            .iter()
            .filter(|(t, _, _)| *t >= at)
            .fold((0.0_f64, 0.0_f64), |(sx, sy), (_, dx, dy)| {
                (sx + *dx, sy + *dy)
            });
        st.x = x;
        st.y = y;
        st.anchored = true;
        // Replay only deltas observed after anchor timestamp.
        st.x += dx_sum;
        st.y += dy_sum;
        Self::clamp_to_bounds(&mut st);
        debug!(
            "mouse tracker anchored at ({:.2}, {:.2}) with replay",
            st.x, st.y
        );
    }

    pub fn set_bounds(&self, width: f64, height: f64) {
        let mut st = self.state.lock().expect("mouse tracker mutex poisoned");
        st.max_x = Some(width);
        st.max_y = Some(height);
        Self::clamp_to_bounds(&mut st);
        debug!(
            "mouse tracker bounds set to {:.2}x{:.2}; current=({:.2}, {:.2})",
            width, height, st.x, st.y
        );
    }

    pub fn sample(&self) -> MouseSample {
        let st = self.state.lock().expect("mouse tracker mutex poisoned");
        MouseSample {
            x: st.x,
            y: st.y,
            anchored: st.anchored,
        }
    }
}

impl Drop for MouseTracker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
    }
}
