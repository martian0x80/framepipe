use std::fs;
use std::io::{BufWriter, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use input::event::pointer::PointerEvent;
use input::event::Event;
use input::{Libinput, LibinputInterface};
use log::{debug, info, warn};

use crate::shared::mouse_ring::{MouseEvent, RingBuffer};
use crate::wayland::types::{
    MouseSample, MouseSampleRecord, MouseState, MouseTrackChunk, MouseTrackHeader,
    MouseTrackRecordingInfo, MouseTracker,
};

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

pub struct MouseTrackerLibinput {
    state: Arc<Mutex<MouseState>>,
    stop: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl MouseTrackerLibinput {
    pub fn clamp_to_bounds(st: &mut MouseState) {
        if let Some(max_x) = st.max_x {
            st.x = st.x.clamp(0.0, max_x.max(0.0));
        }
        if let Some(max_y) = st.max_y {
            st.y = st.y.clamp(0.0, max_y.max(0.0));
        }
    }

    pub fn anchor_absolute(&self, x: f64, y: f64, at: Instant) {
        let mut st = self.state.lock().expect("mouse tracker mutex poisoned");
        st.x = x;
        st.y = y;
        st.anchored = true;
        st.history.clear();
        st.anchor_epoch = st.anchor_epoch.wrapping_add(1);
        Self::clamp_to_bounds(&mut st);
        debug!(
            "mouse tracker anchored at ({:.2}, {:.2}) at {:?}; delta history reset (epoch={})",
            st.x, st.y, at, st.anchor_epoch
        );
    }

    fn write_framed<W: Write>(writer: &mut W, payload: &[u8]) -> Result<(), String> {
        let len = payload.len() as u32;
        writer
            .write_all(&len.to_le_bytes())
            .map_err(|e| format!("failed to write mouse track frame length: {e}"))?;
        writer
            .write_all(payload)
            .map_err(|e| format!("failed to write mouse track frame payload: {e}"))?;
        Ok(())
    }

    fn flush_chunk<W: Write>(
        writer: &mut W,
        pending: &mut Vec<MouseSampleRecord>,
    ) -> Result<(), String> {
        if pending.is_empty() {
            return Ok(());
        }
        let chunk = MouseTrackChunk {
            samples: std::mem::take(pending),
        };
        let bytes = bitcode::encode(&chunk);
        Self::write_framed(writer, &bytes)?;
        writer
            .flush()
            .map_err(|e| format!("failed to flush mouse track chunk: {e}"))?;
        Ok(())
    }
}

impl MouseTracker for MouseTrackerLibinput {
    fn start<T: AsRef<std::path::Path>>(
        file_path: T,
        recording: MouseTrackRecordingInfo,
        ring: Option<Arc<RingBuffer>>,
    ) -> Result<Self, String> {
        let state = Arc::new(Mutex::new(MouseState::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let paused = Arc::new(AtomicBool::new(false));
        let state_clone = Arc::clone(&state);
        let stop_clone = Arc::clone(&stop);
        let paused_clone = Arc::clone(&paused);
        let file_path = PathBuf::from(file_path.as_ref());

        let thread = thread::Builder::new()
            .name("mouse-tracker-libinput".to_string())
            .spawn(move || {
                let file = match fs::File::create(&file_path) {
                    Ok(file) => file,
                    Err(e) => {
                        warn!("failed to create mouse track file {}: {e}", file_path.display());
                        return;
                    }
                };
                let mut writer = BufWriter::new(file);
                let header = MouseTrackHeader {
                    version: 1,
                    recording,
                };
                let header_bytes = bitcode::encode(&header);
                if let Err(e) = Self::write_framed(&mut writer, &header_bytes) {
                    warn!("failed to write mouse track header: {e}");
                    return;
                }

                let mut li = Libinput::new_with_udev(LibinputIface);
                if let Err(e) = li.udev_assign_seat("seat0") {
                    warn!("libinput: failed to assign seat0: {:?}", e);
                    return;
                }

                let batch_window = Duration::from_millis(5);
                let mut batch_start = Instant::now();
                let tracker_start = batch_start;
                let mut batch_dx = 0.0_f64;
                let mut batch_dy = 0.0_f64;
                let mut pending: Vec<MouseSampleRecord> = Vec::with_capacity(256);
                let mut was_paused = false;
                let mut seen_anchor_epoch = 0_u64;

                while !stop_clone.load(Ordering::Relaxed) {
                    {
                        let st = state_clone.lock().expect("mouse tracker mutex poisoned");
                        if st.anchor_epoch != seen_anchor_epoch {
                            seen_anchor_epoch = st.anchor_epoch;
                            batch_dx = 0.0;
                            batch_dy = 0.0;
                            debug!(
                                "mouse tracker observed anchor epoch change -> {}, cleared pending batch deltas",
                                seen_anchor_epoch
                            );
                        }
                    }

                    let is_paused = paused_clone.load(Ordering::Relaxed);
                    if is_paused && !was_paused {
                        if let Err(e) = Self::flush_chunk(&mut writer, &mut pending) {
                            warn!("mouse tracker pause flush failed: {e}");
                        }
                        info!("mouse tracker paused");
                    } else if !is_paused && was_paused {
                        info!("mouse tracker resumed");
                    }
                    was_paused = is_paused;

                    let elapsed = batch_start.elapsed();
                    let timeout_ms = if elapsed >= batch_window {
                        0
                    } else {
                        (batch_window - elapsed).as_millis().max(1) as i32
                    };

                    let mut pfd = libc::pollfd {
                        fd: li.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    };

                    let poll_rc = unsafe { libc::poll(&mut pfd as *mut libc::pollfd, 1, timeout_ms) };
                    if poll_rc < 0 {
                        warn!("libinput poll error: {}", std::io::Error::last_os_error());
                        continue;
                    }

                    if poll_rc > 0 && (pfd.revents & libc::POLLIN) != 0 {
                        if let Err(e) = li.dispatch() {
                            warn!("libinput dispatch error: {e}");
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
                            {
                                let mut st = state_clone.lock().expect("mouse tracker mutex poisoned");
                                st.history.push_back((now, dx, dy));
                                while st.history.len() > 256 {
                                    st.history.pop_front();
                                }
                            }
                            batch_dx += dx;
                            batch_dy += dy;
                        }
                    }

                    if batch_start.elapsed() >= batch_window {
                        if batch_dx != 0.0 || batch_dy != 0.0 {
                            let now = Instant::now();
                            let mut st = state_clone.lock().expect("mouse tracker mutex poisoned");
                            if st.anchored {
                                st.x += batch_dx;
                                st.y += batch_dy;
                                // Clamp once after applying the whole batch.
                                Self::clamp_to_bounds(&mut st);
                                log::trace!(
                                    "mouse tracker calc pos -> ({:.2}, {:.2}) [batch_delta=({:.3}, {:.3}) window_ms={}]",
                                    st.x,
                                    st.y,
                                    batch_dx,
                                    batch_dy,
                                    batch_window.as_millis()
                                );

                                if !is_paused {
                                    let t_ns = now
                                        .duration_since(tracker_start)
                                        .as_nanos()
                                        .min(u64::MAX as u128)
                                        as u64;
                                    let record = MouseSampleRecord {
                                        t_ns,
                                        x: st.x,
                                        y: st.y,
                                        anchored: st.anchored,
                                    };
                                    pending.push(record);
                                    if pending.len() >= 512 {
                                        if let Err(e) = Self::flush_chunk(&mut writer, &mut pending)
                                        {
                                            warn!("mouse tracker chunk flush failed: {e}");
                                        }
                                    }
                                    if let Some(ref ring) = ring {
                                        ring.push(MouseEvent {
                                            t_ns,
                                            x: st.x,
                                            y: st.y,
                                        });
                                    }
                                }
                            }
                        }
                        batch_dx = 0.0;
                        batch_dy = 0.0;
                        batch_start = Instant::now();
                    }
                }

                if let Err(e) = Self::flush_chunk(&mut writer, &mut pending) {
                    warn!("mouse tracker final chunk flush failed: {e}");
                }
                if let Err(e) = writer.flush() {
                    warn!("mouse tracker final writer flush failed: {e}");
                } else {
                    info!("mouse tracker bitcode written to {}", file_path.display());
                }
                debug!("mouse tracker thread exiting");
            })
            .map_err(|e| format!("failed to spawn libinput thread: {e}"))?;

        Ok(Self {
            state,
            stop,
            paused,
            thread: Some(thread),
        })
    }

    fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    fn set_bounds(&self, width: f64, height: f64) {
        let mut st = self.state.lock().expect("mouse tracker mutex poisoned");
        st.max_x = Some(width);
        st.max_y = Some(height);
        Self::clamp_to_bounds(&mut st);
        debug!(
            "mouse tracker bounds set to {:.2}x{:.2}; current=({:.2}, {:.2})",
            width, height, st.x, st.y
        );
    }

    fn sample(&self) -> MouseSample {
        let st = self.state.lock().expect("mouse tracker mutex poisoned");
        MouseSample {
            x: st.x,
            y: st.y,
            anchored: st.anchored,
            timestamp: Instant::now(),
        }
    }
}

impl Drop for MouseTrackerLibinput {
    fn drop(&mut self) {
        info!("mouse tracker stop requested");
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.thread.take() {
            match h.join() {
                Ok(()) => debug!("mouse tracker thread joined"),
                Err(_) => warn!("mouse tracker thread join failed"),
            }
        }
    }
}
