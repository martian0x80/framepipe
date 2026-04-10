# Framepipe

WIP: ACTIVE DEVELOPMENT - EXPECT BREAKAGE

Framepipe is a (zero-copy) GPU-accelerated screen recorder for Linux (Wayland only).
It captures primary plane frames, composites cursor optionally, and encodes
using GStreamer backends (VAAPI/QSV/CPU). It also supports global mouse tracking on Wayland.

Two capture backends are available: DRM/KMS and XDG Portal (PipeWire). The former is more stable and convenient but requires some capabilities, pipwire support is experimental and mostly broken for now.

This repo is experimental and not stable. Interfaces and CLI flags will change.

[Demo Video](https://www.youtube.com/watch?v=bOC7lMbf2aY)

 [![demo](https://img.youtube.com/vi/bOC7lMbf2aY/maxresdefault.jpg)](https://www.youtube.com/watch?v=bOC7lMbf2aY)

## Features (current)

- DRM/KMS capture (primary plane)
- EGL import + GPU render pipeline
- Optional cursor composition with sprite and cursor smoothing
- Optional mouse tracking using zwlr_layer_shell_v1 and libinput
- GStreamer encoding (VAAPI/QSV/CPU)
- Postfx WIP (decode + re-encode)
- Mouse tracking stream (bitcode)
- bt601, bt709, bt2020 support

## Notes

- Using H.264 with VAAPI encoder backend may introduce some artifacts and there is no fix for that so far. Use QSV or use H265 or av1 codecs.

## Quick Start

Build:

```bash
cargo build --all-targets
```

Set caps on the priviledged service binary:

```bash
sudo setcap cap_sys_admin,cap_dac_override+ep target/debug/framepipe-privd
```

`cap_sys_admin` is required for DRM/KMS capture, and `cap_dac_override` is needed to read input devices for mouse tracking.

`sudo` is no longer required for the cli, all priviledged operations are handled by the `framepipe-privd` service.

Run capture (H.264, default backend):

```bash
cargo run -p framepipe -- record --output output.mp4
```

Preview (no file output):

```bash
cargo run -p framepipe -- preview
```

Set Quality presets:

```bash
cargo run -p framepipe -- record --output output.mp4 -q ultra
```

Enable cursor composition with sprite:

```bash
GST_DEBUG="*:3" cargo run -p framepipe -- record --fps 120 --output output.mp4 -q high -v h265 -e qsv -r cqp --wayland-sync-frequency 0.1 --cursor-composition --cursor-sprite /home/martian/Downloads/cursor-weird.png --cursor-scale 0.1
```

## Notes

- Requires access to `/dev/dri/*` (use `sudo -E` for now).
- Some pipelines depend on installed GStreamer plugins (VAAPI/QSV).
- WIP: quality/bitrate tuning, encoder stability, and postfx pipeline.
