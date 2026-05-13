# gayland

`gayland` is a Linux input tracking crate for wayland tools that need global-ish mouse and keyboard events through libinput, with optional Wayland layer-shell anchoring for cursor position.

The crate is intentionally small:

- libinput mouse and keyboard event tracking
- direct input device opening for local tools
- pre-opened input FD support for privileged-helper or sandboxed integrations
- optional Wayland layer-shell anchoring for absolute cursor coordinates
<!-- - edge-triggered hotkey scaffolding -->
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
- Layer-shell anchoring is not perfect and may have edge cases where the cursor can get out of sync. The `sync_frequency_hz` config option can help mitigate this by periodically re-anchoring, but it may not be suitable for all use cases.
- Periodically re-anchoring requries stealing pointer focus (can be configured with `sync_frequency_hz`) which may interfere with user interactions. Setting `sync_frequency_hz` to 0 will anchor once and then rely on libinput deltas, but this may lead to drift over time and not recommended.

## Status

This crate is currently developed as part of Framepipe and is still evolving. API breakage is possible before a stable release.
