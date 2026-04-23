
WIP: ACTIVE DEVELOPMENT - EXPECT BREAKAGE

Framepipe is a (zero-copy) GPU-accelerated screen recorder for Linux (Wayland only).
It captures primary plane frames, composites cursor or effects optionally, and encodes
using GStreamer (VAAPI/QSV/CPU) in realtime. It also supports global mouse tracking on Wayland.

Framepipe is built around a pragmatic design philosophy: when high-level APIs impose limitations, it falls back to lower-level system interfaces to maintain functionality. This enables features such as global mouse tracking and precise capture behavior that are not currently exposed through standard Wayland mechanisms or might never will be.

Rather than waiting for upstream solutions, we focus on delivering working implementations today, even if that requires stepping outside conventional application boundaries.

Framepipe now includes a Iced-based GUI for configuration and preview, but it can also be used as a CLI tool for recording without the GUI.

<img width="2880" height="1800" alt="1776963398586833818(1)" src="https://github.com/user-attachments/assets/526ea792-d42c-4fb3-9e93-c21e75848508" />

Two capture backends are available: DRM/KMS and XDG Portal (PipeWire). The former is more stable and convenient but requires some capabilities (`setcap`) to be set, pipewire support is almost stable as well, but requires extra work.

This repo is still under work and not very stable. Interfaces and CLI flags might change.

[Demo Video #1](https://www.youtube.com/watch?v=bOC7lMbf2aY)

 [![demo](https://img.youtube.com/vi/bOC7lMbf2aY/maxresdefault.jpg)](https://www.youtube.com/watch?v=bOC7lMbf2aY)

[Demo Video #2](https://youtu.be/zAJ6gD-stM0)

 [![demo](https://img.youtube.com/vi/zAJ6gD-stM0/maxresdefault.jpg)](https://www.youtube.com/watch?v=zAJ6gD-stM0)


## Features (current)

- DRM/KMS capture
- xdg-desktop-portal/pipewire capture
- Optional cursor composition with custom sprite, scaling, cursor smoothing/smearing
- Optional mouse tracking using zwlr_layer_shell_v1 and libinput
- GStreamer gpu accelerated encoding (VAAPI/QSV/CPU) (raise an issue for additions)
- bt601, bt709, bt2020 colorimetery support
- partial hdr support (10 bit depth + bt2020)
- H264, H265 and AV1 codec support
- Multiple rate control methods (cbr, vbr, qvbr, vcm, cqp, icq, and more)
- Sane quality presets
- Custom backgrounds and zoom
- a broken gui

## Notes

- Using H.264 with VAAPI encoder backend may introduce some artifacts and there is no fix for that so far. Use QSV or use H265 or av1 codecs.

## Quick Start

Build:

```bash
cargo build --all-targets
```

Set caps on the priviledged service binary if you plan to use kms capture:

```bash
sudo setcap cap_sys_admin,cap_dac_override+ep target/debug/framepipe-privd
```

`cap_sys_admin` is required for DRM/KMS capture, and `cap_dac_override` is needed to read input devices for mouse tracking.

`sudo` is no longer required for the cli, all priviledged operations are handled by the `framepipe-privd` service.

Run capture (H.264, default backend):

```bash
cargo run -p framepipe -- record
```

Preview (no file output):

```bash
cargo run -p framepipe -- preview
```

Set Quality presets:

```bash
cargo run -p framepipe -- record --output output.mp4 -q ultra
```

Enable cursor composition with sprite, set fps, profile, codec, and rate control method:

```bash
GST_DEBUG="*:3" cargo run -p framepipe -- record --fps 120 -q high -v h265 -e qsv -r cqp --wayland-sync-frequency 0.1 --cursor-composition --cursor-sprite /home/martian/Downloads/cursor-weird.png --cursor-scale 0.1
```

## Notes

- Requires access to `/dev/dri/*` for kms capture.
- Some pipelines depend on installed GStreamer plugins.

If you are hiring, reach out at `hire@0x80.dev` or @martian0x80 on Twitter.
