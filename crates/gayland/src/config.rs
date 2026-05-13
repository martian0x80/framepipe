use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct GaylandConfig {
    pub seat: String,
    pub batch_window: Duration,
    pub enable_mouse: bool,
    pub enable_keyboard: bool,
}

impl Default for GaylandConfig {
    fn default() -> Self {
        Self {
            seat: "seat0".to_string(),
            batch_window: Duration::from_millis(5),
            enable_mouse: true,
            enable_keyboard: true,
        }
    }
}

pub enum InputSource {
    Preopened {
        fds_by_path: HashMap<PathBuf, OwnedFd>,
    },
    DirectOpen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HotkeySpec {
    pub keycode: u32,
    pub mods_mask: u32,
}
