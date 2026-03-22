use std::{
    os::fd::AsRawFd,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use log::{info, warn};
use smithay_client_toolkit::{
    compositor::{self, CompositorHandler}, delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_registry, delegate_seat, delegate_shm, output::{OutputHandler, OutputState}, reexports::{
        client::{Connection, QueueHandle, delegate_dispatch, globals::registry_queue_init, protocol::{wl_pointer::WlPointer, wl_region, wl_shm}},
    }, registry::{ProvidesRegistryState, RegistryState}, registry_handlers, seat::{SeatHandler, SeatState, pointer::PointerHandler}, shell::{
        WaylandSurface,
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface},
    }, shm::{Shm, ShmHandler, slot::SlotPool}
};
use crate::wayland::{
    mouse_tracker::MouseTrackerLibinput,
    types::{MouseTrackRecordingInfo, MouseTracker},
};

#[derive(Debug, thiserror::Error)]
pub enum WaylandError {
    #[error("Failed to connect to Wayland")]
    ConnectionFailed,
    #[error("Failed to initialize Wayland registry")]
    RegistryInitializationFailed,
    #[error("Failed to bind compositor")]
    CompositorBindingFailed,
    #[error("Failed to bind layer shell")]
    LayerShellBindingFailed,
}

#[derive(Clone)]
pub struct TrackingControl {
    pub stop_requested: Arc<AtomicBool>,
    pub paused: Arc<AtomicBool>,
}

impl TrackingControl {
    pub fn new(stop_requested: Arc<AtomicBool>, paused: Arc<AtomicBool>) -> Self {
        Self {
            stop_requested,
            paused,
        }
    }

    pub fn idle() -> Self {
        Self {
            stop_requested: Arc::new(AtomicBool::new(false)),
            paused: Arc::new(AtomicBool::new(false)),
        }
    }
}

pub struct WaylandState {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    compositor_state: compositor::CompositorState,

    layer_surface: LayerSurface,
    shm: Shm,
    pool: SlotPool,

    width: u32,
    height: u32,

    cursor_x: f64,
    cursor_y: f64,

    pointer: Option<WlPointer>,
    mouse_tracker: MouseTrackerLibinput,
    waiting_for_anchor: bool,
    resync_probe_mode: bool,
    last_anchor_at: Option<Instant>,
    resync_period: Option<Duration>,
    resync_deadline: Option<Instant>,
    stop_capture: bool,
}

pub fn init_wayland(
    sync_frequency_hz: f64,
    file_path: &std::path::PathBuf,
    control: TrackingControl,
    recording: MouseTrackRecordingInfo,
) -> Result<(), WaylandError> {
    info!("Initializing Wayland connection and event loop");
    info!("Mouse tracking output file: {}", file_path.to_string_lossy());
    // Start libinput tracker first so we can replay deltas after first absolute anchor.
    let mouse_tracker = MouseTrackerLibinput::start(file_path, recording)
        .map_err(|_| WaylandError::ConnectionFailed)?;

    let conn = Connection::connect_to_env().map_err(|_| WaylandError::ConnectionFailed)?;
    let (globals, mut event_queue) =
        registry_queue_init(&conn).map_err(|_| WaylandError::RegistryInitializationFailed)?;
    let qh = event_queue.handle();
    let compositor = compositor::CompositorState::bind(&globals, &qh)
        .map_err(|_| WaylandError::CompositorBindingFailed)?;
    let layer_shell =
        LayerShell::bind(&globals, &qh).map_err(|_| WaylandError::LayerShellBindingFailed)?;
    let shm = Shm::bind(&globals, &qh).map_err(|_| WaylandError::LayerShellBindingFailed)?;
    let surface = compositor.create_surface(&qh);
    // let region = compositor.wl_compositor().create_region(&qh, ());
    // surface.set_input_region(Some(&region));
    // region.destroy();
    // surface.commit();
    let layer =
        layer_shell.create_layer_surface(&qh, surface, Layer::Overlay, Some("openstudio"), None);
    layer.set_anchor(Anchor::all());
    layer.set_size(0, 0);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.commit();
    let pool = SlotPool::new(4 * 1 * 1, &shm).unwrap();
    let mut state = WaylandState {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        compositor_state: compositor,
        layer_surface: layer,
        shm: shm,
        pool,
        width: 0,
        height: 0,
        cursor_x: 0.0,
        cursor_y: 0.0,
        pointer: None,
        mouse_tracker,
        waiting_for_anchor: true,
        resync_probe_mode: false,
        last_anchor_at: None,
        resync_period: if sync_frequency_hz > 0.0 {
            Some(Duration::from_secs_f64(1.0 / sync_frequency_hz))
        } else {
            None
        },
        resync_deadline: None,
        stop_capture: false,
    };
    loop {
        if control.stop_requested.load(Ordering::Relaxed) {
            info!("Mouse tracking stop requested");
            break;
        }
        state
            .mouse_tracker
            .set_paused(control.paused.load(Ordering::Relaxed));

        if let (Some(period), Some(last_anchor_at)) = (state.resync_period, state.last_anchor_at)
        {
            if !state.waiting_for_anchor && last_anchor_at.elapsed() >= period {
                state.waiting_for_anchor = true;
                state.resync_probe_mode = true;
                state.resync_deadline = Some(Instant::now() + Duration::from_millis(120));
                state.set_input_region_probe(&qh);
                info!(
                    "Periodic resync requested after {:?}; using probe input region",
                    period
                );
            }
        }

        if state.waiting_for_anchor
            && state.resync_probe_mode
            && state
                .resync_deadline
                .map(|deadline| Instant::now() >= deadline)
                .unwrap_or(false)
        {
            state.waiting_for_anchor = false;
            state.resync_probe_mode = false;
            state.resync_deadline = None;
            state.set_input_region_clickthrough(&qh);
            warn!("Resync probe timed out; restoring click-through input region");
        }

        let _ = event_queue.dispatch_pending(&mut state);
        let _ = event_queue.flush();

        // Read new Wayland events with bounded wait so periodic resync still runs.
        if let Some(read_guard) = event_queue.prepare_read() {
            let timeout_ms = if state.waiting_for_anchor {
                10
            } else {
                state
                    .resync_period
                    .map(|p| (p.as_millis().min(250)) as i32)
                    .unwrap_or(16)
            };

            let mut pfd = libc::pollfd {
                fd: read_guard.connection_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };

            let poll_rc = unsafe { libc::poll(&mut pfd as *mut libc::pollfd, 1, timeout_ms) };
            if poll_rc > 0 && (pfd.revents & libc::POLLIN) != 0 {
                let _ = read_guard.read();
                let _ = event_queue.dispatch_pending(&mut state);
            }
        }

        if state.stop_capture {
            info!("Stopping capture and exiting Wayland event loop");
            break;
        }
    }

    info!(
        "Mouse tracking event loop exiting, data written to {}",
        file_path.to_string_lossy()
    );
    Ok(())
}

impl CompositorHandler for WaylandState {
    fn scale_factor_changed(
        &mut self,
        conn: &Connection,
        qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>,
        surface: &smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface,
        new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        conn: &Connection,
        qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>,
        surface: &smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface,
        new_transform: smithay_client_toolkit::reexports::client::protocol::wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        conn: &Connection,
        qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>,
        surface: &smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface,
        time: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        conn: &Connection,
        qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>,
        surface: &smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface,
        output: &smithay_client_toolkit::reexports::client::protocol::wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        conn: &Connection,
        qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>,
        surface: &smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface,
        output: &smithay_client_toolkit::reexports::client::protocol::wl_output::WlOutput,
    ) {
    }
}

impl PointerHandler for WaylandState {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>,
        _pointer: &smithay_client_toolkit::reexports::client::protocol::wl_pointer::WlPointer,
        events: &[smithay_client_toolkit::seat::pointer::PointerEvent],
    ) {
        use smithay_client_toolkit::seat::pointer::PointerEventKind as Kind;
        for event in events {
            match event.kind {
                _ => {
                    self.cursor_x = event.position.0;
                    self.cursor_y = event.position.1;
                    if self.waiting_for_anchor {
                        // First absolute point from layer-surface pointer focus.
                        let now = Instant::now();
                        self.mouse_tracker
                            .anchor_absolute(self.cursor_x, self.cursor_y, now);
                        self.waiting_for_anchor = false;
                        self.resync_probe_mode = false;
                        self.resync_deadline = None;
                        self.last_anchor_at = Some(now);
                        self.set_input_region_clickthrough(qh);
                        info!(
                            "Anchored absolute mouse position at ({:.2}, {:.2}) at t={:?}; switched layer input region to click-through",
                            self.cursor_x, self.cursor_y, now
                        );
                    }
                }
                _ => {}
            }
        }
    }
}

impl ShmHandler for WaylandState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl LayerShellHandler for WaylandState {
    fn closed(
        &mut self,
        conn: &Connection,
        qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>,
        layer: &smithay_client_toolkit::shell::wlr_layer::LayerSurface,
    ) {
        self.stop_capture = true;
    }

    fn configure(
        &mut self,
        conn: &Connection,
        qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>,
        layer: &smithay_client_toolkit::shell::wlr_layer::LayerSurface,
        configure: smithay_client_toolkit::shell::wlr_layer::LayerSurfaceConfigure,
        serial: u32,
    ) {
        info!("Layer surface configured with serial {}", serial);
        self.width = configure.new_size.0;
        self.height = configure.new_size.1;
        self.mouse_tracker
            .set_bounds(self.width as f64 - 1f64, self.height as f64 - 1f64);
        self.draw(qh);
        if self.waiting_for_anchor {
            if self.resync_probe_mode {
                self.set_input_region_probe(qh);
            } else {
                self.set_input_region_full(qh);
            }
        } else {
            self.set_input_region_clickthrough(qh);
        }
    }
}

impl OutputHandler for WaylandState {
    fn output_state(&mut self) -> &mut smithay_client_toolkit::output::OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        conn: &Connection,
        qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>,
        output: smithay_client_toolkit::reexports::client::protocol::wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        conn: &Connection,
        qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>,
        output: smithay_client_toolkit::reexports::client::protocol::wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        conn: &Connection,
        qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>,
        output: smithay_client_toolkit::reexports::client::protocol::wl_output::WlOutput,
    ) {
    }
}

impl SeatHandler for WaylandState {
    fn seat_state(&mut self) -> &mut smithay_client_toolkit::seat::SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, conn: &Connection, qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>, seat: smithay_client_toolkit::reexports::client::protocol::wl_seat::WlSeat) {
    }

    fn new_capability(
        &mut self,
        conn: &Connection,
        qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>,
        seat: smithay_client_toolkit::reexports::client::protocol::wl_seat::WlSeat,
        capability: smithay_client_toolkit::seat::Capability,
    ) {
       if capability == smithay_client_toolkit::seat::Capability::Pointer {
            info!("Pointer capability added to seat");
            let pointer = self.seat_state.get_pointer(&qh, &seat).unwrap();
            self.pointer = Some(pointer);
        }
    }

    fn remove_capability(
        &mut self,
        conn: &Connection,
        qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>,
        seat: smithay_client_toolkit::reexports::client::protocol::wl_seat::WlSeat,
        capability: smithay_client_toolkit::seat::Capability,
    ) {
        if capability == smithay_client_toolkit::seat::Capability::Pointer {
            info!("Pointer capability removed from seat");
            self.pointer = None;
        }
    }

    fn remove_seat(&mut self, conn: &Connection, qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>, seat: smithay_client_toolkit::reexports::client::protocol::wl_seat::WlSeat) {
        if let Some(pointer) = self.pointer.take() {
            pointer.release();
        }
    }
}

impl WaylandState {
    fn set_input_region_full(&self, qh: &QueueHandle<Self>) {
        let region = self.compositor_state.wl_compositor().create_region(qh, ());
        region.add(0, 0, self.width as i32, self.height as i32);
        self.layer_surface.wl_surface().set_input_region(Some(&region));
        region.destroy();
        self.layer_surface.commit();
    }

    fn set_input_region_clickthrough(&self, qh: &QueueHandle<Self>) {
        let region = self.compositor_state.wl_compositor().create_region(qh, ());
        // Empty input region => click-through.
        self.layer_surface.wl_surface().set_input_region(Some(&region));
        region.destroy();
        self.layer_surface.commit();
    }

    fn set_input_region_probe(&self, qh: &QueueHandle<Self>) {
        let region = self.compositor_state.wl_compositor().create_region(qh, ());
        let sample = self.mouse_tracker.sample();
        let box_size = 96_i32;
        let half = box_size / 2;
        let max_x = self.width.saturating_sub(1) as i32;
        let max_y = self.height.saturating_sub(1) as i32;
        let cx = sample.x.round() as i32;
        let cy = sample.y.round() as i32;
        let x = (cx - half).clamp(0, max_x.max(0));
        let y = (cy - half).clamp(0, max_y.max(0));
        let w = box_size.min(self.width as i32 - x).max(1);
        let h = box_size.min(self.height as i32 - y).max(1);
        region.add(x, y, w, h);
        self.layer_surface.wl_surface().set_input_region(Some(&region));
        region.destroy();
        self.layer_surface.commit();
    }

    pub fn draw(&mut self, qh: &QueueHandle<Self>) {
    let width = self.width;
    let height = self.height;
    let stride = width as i32 * 4;

    let (buffer, canvas) = self.pool
        .create_buffer(
            width as i32,
            height as i32,
            stride,
            wl_shm::Format::Argb8888,
        )
        .expect("create buffer");

    // Fill buffer (transparent pixels)
    for chunk in canvas.chunks_exact_mut(4) {
        chunk.copy_from_slice(&[0, 0, 0, 0]);
    }

    // Mark the whole surface as damaged
    self.layer_surface
        .wl_surface()
        .damage_buffer(0, 0, width as i32, height as i32);

    // Request next frame (optional but common)
    self.layer_surface
        .wl_surface()
        .frame(qh, self.layer_surface.wl_surface().clone());

    // Attach buffer and commit
    buffer
        .attach_to(self.layer_surface.wl_surface())
        .expect("buffer attach");

    self.layer_surface.commit();

    let mut sample = self.mouse_tracker.sample();
    if sample.anchored && self.width > 0 && self.height > 0 {
        // Keep reported cursor bounded to layer logical size.
        sample.x = sample.x.clamp(0.0, self.width as f64);
        sample.y = sample.y.clamp(0.0, self.height as f64);
    }
    if sample.anchored {
        info!(
            "Mouse tracker sample: ({:.2}, {:.2}) waiting_for_anchor={} size={}x{}",
            sample.x, sample.y, self.waiting_for_anchor, self.width, self.height
        );
    } else {
        warn!("Mouse tracker waiting for absolute anchor from layer pointer event");
    }
}
}

delegate_compositor!(WaylandState);
delegate_layer!(WaylandState);
delegate_shm!(WaylandState);
delegate_seat!(WaylandState);
delegate_pointer!(WaylandState);
delegate_output!(WaylandState);
delegate_registry!(WaylandState);
delegate_dispatch!(WaylandState: [wl_region::WlRegion: ()] => WaylandState);

impl ProvidesRegistryState for WaylandState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState, SeatState];
}
