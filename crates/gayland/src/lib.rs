//! `gayland` provides a libinput runtime plus optional layer-shell anchoring.
//!
//! Public entrypoints:
//! - [`start_runtime`]: start libinput processing and receive [`GaylandEvent`] over a channel.
//! - [`start_tracker`]: start the public tracker session, optionally with layer-shell anchoring.

pub mod config;
pub mod event;
pub mod keyboard;
#[cfg(feature = "recording")]
pub mod recording;
pub mod runtime;
pub mod session;

#[cfg(feature = "layer-shell")]
pub mod layer_shell;
#[cfg(feature = "layer-shell")]
pub use layer_shell::{LayerShellConfig, WaylandError};

pub use runtime::{GaylandController, GaylandHandle, start as start_runtime};
#[cfg(feature = "layer-shell")]
pub use session::start_layer_shell;
pub use session::{TrackerConfig, TrackerError, TrackerSession, start_tracker};
pub use {
    config::GaylandConfig,
    config::HotkeySpec,
    config::InputSource,
    event::GaylandEvent,
    keyboard::{HotkeyParseError, KeyInfo, key_info, key_name, keycode_from_name, parse_hotkey},
};
