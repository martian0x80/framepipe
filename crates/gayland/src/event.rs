#[derive(Debug, Clone, Copy)]
pub enum StateEvent {
    Started,
    Paused,
    Resumed,
    Stopped,
}

#[derive(Debug, Clone, Copy)]
pub enum GaylandEvent {
    MouseDelta {
        dx: f64,
        dy: f64,
        t_ns: u64,
    },
    MousePosition {
        x: f64,
        y: f64,
        max_x: Option<f64>,
        max_y: Option<f64>,
        anchored: bool,
        t_ns: u64,
    },
    MouseButton {
        button: u32,
        pressed: bool,
        t_ns: u64,
    },
    KeyboardKey {
        keycode: u32,
        key_name: Option<&'static str>,
        is_modifier: bool,
        pressed: bool,
        t_ns: u64,
    },
    HotkeyTriggered {
        id: u64,
        t_ns: u64,
    },
    BoundsChanged {
        max_x: Option<f64>,
        max_y: Option<f64>,
    },
    AnchorChanged {
        x: f64,
        y: f64,
    },
    State(StateEvent),
}
