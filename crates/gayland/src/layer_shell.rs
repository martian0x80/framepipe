use std::{
    os::fd::AsRawFd,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::Receiver,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::{GaylandEvent, runtime::GaylandController};
use log::{info, warn};
use smithay_client_toolkit::{
    compositor::{self, CompositorHandler},
    delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm,
    output::{OutputHandler, OutputState},
    reexports::client::{
        Connection, QueueHandle, delegate_dispatch,
        globals::registry_queue_init,
        protocol::{wl_pointer::WlPointer, wl_region, wl_shm},
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{SeatHandler, SeatState, pointer::PointerHandler},
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
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
    #[error("Failed to start gayland runtime: {0}")]
    RuntimeStartFailed(String),
    #[error("Layer shell thread failed to start: {0}")]
    ThreadStartFailed(String),
}

#[derive(Clone)]
pub(crate) struct LayerShellControl {
    pub stop_requested: Arc<AtomicBool>,
    pub paused: Arc<AtomicBool>,
}

impl LayerShellControl {
    pub fn idle() -> Self {
        Self {
            stop_requested: Arc::new(AtomicBool::new(false)),
            paused: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// Configuration for the layer-shell anchor helper.
///
/// This starts a transparent overlay surface, uses pointer focus to establish an
/// absolute cursor anchor, and feeds that anchor into the libinput runtime.
pub struct LayerShellConfig {
    pub namespace: String,
    pub sync_frequency_hz: f64,
}

impl Default for LayerShellConfig {
    fn default() -> Self {
        Self {
            namespace: "gayland".to_string(),
            sync_frequency_hz: 0.0,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct CursorSample {
    anchored: bool,
    x: f64,
    y: f64,
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
    runtime: GaylandController,
    current_sample: CursorSample,
    waiting_for_anchor: bool,
    resync_probe_mode: bool,
    last_anchor_at: Option<Instant>,
    resync_period: Option<Duration>,
    resync_deadline: Option<Instant>,
    stop_capture: bool,
}

pub(crate) fn spawn_layer_shell(
    config: LayerShellConfig,
    control: LayerShellControl,
    runtime: GaylandController,
    runtime_events: Receiver<GaylandEvent>,
    pause_runtime: bool,
) -> Result<JoinHandle<Result<(), WaylandError>>, WaylandError> {
    thread::Builder::new()
        .name("gayland-layer-shell".to_string())
        .spawn(move || {
            run_layer_shell_loop(LayerShellRun {
                namespace: config.namespace,
                sync_frequency_hz: config.sync_frequency_hz,
                control,
                runtime,
                runtime_events,
                pause_runtime,
            })
        })
        .map_err(|e| WaylandError::ThreadStartFailed(e.to_string()))
}

struct LayerShellRun {
    namespace: String,
    sync_frequency_hz: f64,
    control: LayerShellControl,
    runtime: GaylandController,
    runtime_events: Receiver<GaylandEvent>,
    pause_runtime: bool,
}

fn run_layer_shell_loop(run: LayerShellRun) -> Result<(), WaylandError> {
    let LayerShellRun {
        namespace,
        sync_frequency_hz,
        control,
        runtime,
        runtime_events,
        pause_runtime,
    } = run;

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
    let layer =
        layer_shell.create_layer_surface(&qh, surface, Layer::Overlay, Some(namespace), None);
    layer.set_anchor(Anchor::all());
    layer.set_size(0, 0);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.commit();
    let pool = SlotPool::new(4, &shm).unwrap();

    let mut state = WaylandState {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        compositor_state: compositor,
        layer_surface: layer,
        shm,
        pool,
        width: 0,
        height: 0,
        cursor_x: 0.0,
        cursor_y: 0.0,
        pointer: None,
        runtime,
        current_sample: CursorSample::default(),
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

    let mut last_paused = false;
    loop {
        if control.stop_requested.load(Ordering::Relaxed) {
            info!("Mouse tracking stop requested");
            break;
        }

        let paused = control.paused.load(Ordering::Relaxed);
        if paused != last_paused {
            if pause_runtime {
                if paused {
                    state.runtime.pause();
                } else {
                    state.runtime.resume();
                }
            }
            last_paused = paused;
        }

        // todo: don't depend on waiting for events, use the last cached sample if available and only block on the event queue when idle
        while let Ok(event) = runtime_events.try_recv() {
            if let GaylandEvent::MousePosition { x, y, anchored, .. } = event {
                state.current_sample = CursorSample { anchored, x, y };
            }
        }

        if let (Some(period), Some(last_anchor_at)) = (state.resync_period, state.last_anchor_at)
            && !state.waiting_for_anchor
            && last_anchor_at.elapsed() >= period
        {
            state.waiting_for_anchor = true;
            state.runtime.clear_anchor();
            state.resync_probe_mode = true;
            state.resync_deadline = Some(Instant::now() + Duration::from_millis(50));
            state.set_input_region_probe(&qh);
            log::trace!(
                "Periodic resync requested after {:?}; using probe input region",
                period
            );
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
            state.last_anchor_at = Some(Instant::now());
            state.set_input_region_clickthrough(&qh);
            log::debug!("Resync probe timed out; restoring click-through input region");
        }

        let _ = event_queue.dispatch_pending(&mut state);
        let _ = event_queue.flush();

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

    info!("Layer-shell tracking event loop exiting");
    Ok(())
}

impl CompositorHandler for WaylandState {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>,
        _surface: &smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>,
        _surface: &smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface,
        _new_transform: smithay_client_toolkit::reexports::client::protocol::wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>,
        _surface: &smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface,
        _time: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>,
        _surface: &smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface,
        _output: &smithay_client_toolkit::reexports::client::protocol::wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>,
        _surface: &smithay_client_toolkit::reexports::client::protocol::wl_surface::WlSurface,
        _output: &smithay_client_toolkit::reexports::client::protocol::wl_output::WlOutput,
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
        for event in events {
            self.cursor_x = event.position.0;
            self.cursor_y = event.position.1;
            if self.waiting_for_anchor {
                let now = Instant::now();
                self.runtime.set_anchor(self.cursor_x, self.cursor_y);
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
        _conn: &Connection,
        _qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>,
        _layer: &smithay_client_toolkit::shell::wlr_layer::LayerSurface,
    ) {
        self.stop_capture = true;
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>,
        _layer: &smithay_client_toolkit::shell::wlr_layer::LayerSurface,
        configure: smithay_client_toolkit::shell::wlr_layer::LayerSurfaceConfigure,
        serial: u32,
    ) {
        info!("Layer surface configured with serial {}", serial);
        self.width = configure.new_size.0;
        self.height = configure.new_size.1;
        self.runtime
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
        _conn: &Connection,
        _qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>,
        _output: smithay_client_toolkit::reexports::client::protocol::wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>,
        _output: smithay_client_toolkit::reexports::client::protocol::wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>,
        _output: smithay_client_toolkit::reexports::client::protocol::wl_output::WlOutput,
    ) {
    }
}

impl SeatHandler for WaylandState {
    fn seat_state(&mut self) -> &mut smithay_client_toolkit::seat::SeatState {
        &mut self.seat_state
    }

    fn new_seat(
        &mut self,
        _conn: &Connection,
        _qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>,
        _seat: smithay_client_toolkit::reexports::client::protocol::wl_seat::WlSeat,
    ) {
    }

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>,
        seat: smithay_client_toolkit::reexports::client::protocol::wl_seat::WlSeat,
        capability: smithay_client_toolkit::seat::Capability,
    ) {
        if capability == smithay_client_toolkit::seat::Capability::Pointer {
            info!("Pointer capability added to seat");
            let pointer = self.seat_state.get_pointer(qh, &seat).unwrap();
            self.pointer = Some(pointer);
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>,
        _seat: smithay_client_toolkit::reexports::client::protocol::wl_seat::WlSeat,
        capability: smithay_client_toolkit::seat::Capability,
    ) {
        if capability == smithay_client_toolkit::seat::Capability::Pointer {
            info!("Pointer capability removed from seat");
            self.pointer = None;
        }
    }

    fn remove_seat(
        &mut self,
        _conn: &Connection,
        _qh: &smithay_client_toolkit::reexports::client::QueueHandle<Self>,
        _seat: smithay_client_toolkit::reexports::client::protocol::wl_seat::WlSeat,
    ) {
        if let Some(pointer) = self.pointer.take() {
            pointer.release();
        }
    }
}

impl WaylandState {
    fn set_input_region_full(&self, qh: &QueueHandle<Self>) {
        let region = self.compositor_state.wl_compositor().create_region(qh, ());
        region.add(0, 0, self.width as i32, self.height as i32);
        self.layer_surface
            .wl_surface()
            .set_input_region(Some(&region));
        region.destroy();
        self.layer_surface.commit();
    }

    fn set_input_region_clickthrough(&self, qh: &QueueHandle<Self>) {
        let region = self.compositor_state.wl_compositor().create_region(qh, ());
        self.layer_surface
            .wl_surface()
            .set_input_region(Some(&region));
        region.destroy();
        self.layer_surface.commit();
    }

    fn set_input_region_probe(&self, qh: &QueueHandle<Self>) {
        let region = self.compositor_state.wl_compositor().create_region(qh, ());
        let sample = self.current_sample;
        let box_size = 512_i32;
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
        self.layer_surface
            .wl_surface()
            .set_input_region(Some(&region));
        region.destroy();
        self.layer_surface.commit();
    }

    pub fn draw(&mut self, qh: &QueueHandle<Self>) {
        let width = self.width;
        let height = self.height;
        let stride = width as i32 * 4;

        let (buffer, canvas) = self
            .pool
            .create_buffer(
                width as i32,
                height as i32,
                stride,
                wl_shm::Format::Argb8888,
            )
            .expect("create buffer");

        for chunk in canvas.chunks_exact_mut(4) {
            chunk.copy_from_slice(&[0, 0, 0, 0]);
        }

        self.layer_surface
            .wl_surface()
            .damage_buffer(0, 0, width as i32, height as i32);

        self.layer_surface
            .wl_surface()
            .frame(qh, self.layer_surface.wl_surface().clone());

        buffer
            .attach_to(self.layer_surface.wl_surface())
            .expect("buffer attach");

        self.layer_surface.commit();

        let mut sample = self.current_sample;
        if self.width > 0 && self.height > 0 {
            sample.x = sample.x.clamp(0.0, self.width as f64);
            sample.y = sample.y.clamp(0.0, self.height as f64);
        }
        if !self.waiting_for_anchor {
            info!(
                "Mouse tracker sample: ({:.2}, {:.2}) anchored_edge={} waiting_for_anchor={} size={}x{}",
                sample.x,
                sample.y,
                sample.anchored,
                self.waiting_for_anchor,
                self.width,
                self.height
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
