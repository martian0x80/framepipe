use std::collections::{BTreeSet, HashMap};
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use crate::keyboard::{HotkeyParseError, parse_hotkey};

#[derive(Debug, Clone)]
pub struct GaylandConfig {
    pub seat: String,
    pub batch_window: Duration,
    pub enable_mouse: bool,
    pub enable_keyboard: bool,
    pub hotkeys: HashMap<u64, HotkeySpec>,
}

impl Default for GaylandConfig {
    fn default() -> Self {
        Self {
            seat: "seat0".to_string(),
            batch_window: Duration::from_millis(5),
            enable_mouse: true,
            enable_keyboard: true,
            hotkeys: HashMap::new(),
        }
    }
}

pub enum InputSource {
    Preopened {
        fds_by_path: HashMap<PathBuf, OwnedFd>,
    },
    DirectOpen,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HotkeySpec {
    pub keys: BTreeSet<u32>,
}

impl HotkeySpec {
    pub fn new(keys: impl IntoIterator<Item = u32>) -> Self {
        Self {
            keys: keys.into_iter().collect(),
        }
    }

    pub fn single(key: u32) -> Self {
        Self::new([key])
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

impl FromStr for HotkeySpec {
    type Err = HotkeyParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_hotkey(s)
    }
}
