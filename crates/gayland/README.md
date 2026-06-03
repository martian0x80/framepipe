# gayland

`gayland` is a Linux input tracking crate for Wayland tools that need global-ish mouse and keyboard events through libinput, with optional Wayland layer-shell anchoring for cursor position.

The crate is intentionally small:

- libinput mouse and keyboard event tracking
- strict multi-key hotkey detection
- direct input device opening for local tools
- pre-opened input FD support for privileged-helper or sandboxed integrations
- optional Wayland layer-shell anchoring for absolute cursor coordinates
- optional generic bitcode-framed recording helpers

## Quick Start

```rust,no_run
use gayland::{InputSource, LayerShellConfig, TrackerConfig, start_tracker};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tracker = start_tracker(
        TrackerConfig::new(InputSource::DirectOpen)
            .with_layer_shell(LayerShellConfig::default()),
    )?;

    while let Ok(event) = tracker.events.recv() {
        println!("{event:?}");
    }

    Ok(())
}
```

## Hotkeys

Hotkeys are registered before the tracker starts:

```rust,no_run
use gayland::{InputSource, TrackerConfig, start_tracker};

const HOTKEY_CTRL_SHIFT_R: u64 = 1;
let tracker = start_tracker(
    TrackerConfig::new(InputSource::DirectOpen)
        .with_hotkey_str(HOTKEY_CTRL_SHIFT_R, "Ctrl+Shift+R")?,
)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Matching is strict: a hotkey fires only when the currently pressed key set exactly matches the hotkey's key set. Repeats are edge-triggered, so holding the chord does not emit repeated hotkey events.

Key names are converted to evdev keycodes before matching. This keeps hotkeys physical and layout-independent. Keyboard events also include a best-effort evdev key name for overlays/logging; full layout-aware text conversion is not currently supported, but feel free to open an issue if that's a feature you'd like to see.

## Input Sources

`InputSource::DirectOpen` lets `gayland` open input devices directly through libinput. This usually requires appropriate permissions.

`InputSource::Preopened` accepts already-opened device file descriptors, which is useful when a privileged helper opens `/dev/input/event*` and passes descriptors to an unprivileged process.

## Layer-Shell Anchoring

With the default `layer-shell` feature, `gayland` can create a transparent layer-shell surface and use pointer focus to establish an absolute cursor anchor. Libinput deltas are then applied relative to that anchor.

This is useful for capture/overlay tools that need cursor position without depending on compositor-specific private APIs.

## Features

- `layer-shell` enabled by default: Wayland layer-shell anchor helper.
- `mouse` enabled by default: mouse event tracking.
- `keyboard` enabled by default: keyboard event tracking.
- `recording`: generic bitcode-framed recording utilities.

## Caveats

- Layer-shell anchoring can drift if libinput deltas and compositor cursor state diverge.
- `sync_frequency_hz` can periodically re-anchor, but it briefly changes pointer focus and may interfere with interactions.
- Setting `sync_frequency_hz` to `0.0` anchors once and then relies only on libinput deltas.

## Status

This crate is currently developed as part of Framepipe and is still evolving. API breakage is possible before a stable release.
