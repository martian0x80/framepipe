use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyState {
    Pressed,
    Released,
}

#[derive(Debug, Clone, Copy)]
pub struct KeyEvent {
    pub keycode: u32,
    pub state: KeyState,
    pub time_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub keycode: u32,
}

#[derive(Default)]
pub struct KeyboardState {
    pressed: HashSet<u32>,
}

impl KeyboardState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply(&mut self, event: KeyEvent) {
        match event.state {
            KeyState::Pressed => {
                self.pressed.insert(event.keycode);
            }
            KeyState::Released => {
                self.pressed.remove(&event.keycode);
            }
        }
    }

    pub fn is_pressed(&self, keycode: u32) -> bool {
        self.pressed.contains(&keycode)
    }

    pub fn pressed_count(&self) -> usize {
        self.pressed.len()
    }
}

pub trait KeyboardEventSink: Send + Sync {
    fn on_key_event(&self, _event: KeyEvent) {}
}
