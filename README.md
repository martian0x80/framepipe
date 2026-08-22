
Framepipe is a (zero-copy) GPU-accelerated screen recorder for Linux (Wayland only).
It captures primary plane frames, composites cursor or effects optionally, and encodes
using GStreamer (VAAPI/QSV/CPU) in realtime. It also supports global mouse tracking on Wayland.

#### Framepipe is now available on the AUR for arch users: https://aur.archlinux.org/packages/framepipe-git

Framepipe is built around a pragmatic design philosophy: when high-level APIs impose limitations, it falls back to lower-level system interfaces to maintain functionality. This enables features such as global mouse tracking and precise capture behavior that are not currently exposed through standard Wayland mechanisms or might never will be.

Framepipe also includes a Iced-based GUI for configuration and preview, but it can also be used as a CLI tool for recording without the GUI.

<img width="1920" height="1440" alt="144_1x_shots_so" src="https://github.com/user-attachments/assets/2efad40a-8089-46e9-b46c-8106b9dd7a98" />

Two capture backends are available: DRM/KMS and XDG Portal (PipeWire). DRM/KMS is the more stable backend but needs privileged access. Framepipe can request that access through polkit or use a helper with file capabilities. The portal backend does not need privileged display access, although global input features still do.

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

### Privileged access

KMS capture and global input tracking need access to protected device files. Framepipe keeps the main application unprivileged and obtains only the required file descriptors through `framepipe-privd`.

Installed packages use `auto` mode by default. Framepipe first tries direct access and opens a polkit authentication prompt only if permission is denied:

```bash
framepipe record --privilege-mode auto
```

The available modes are:

- `auto`: try direct access, then fall back to polkit on permission errors.
- `polkit`: always request authorization through polkit.
- `direct`: use existing file capabilities, root privileges, or device permissions without a polkit prompt.

The mode can also be set through `FRAMEPIPE_PRIVILEGE_MODE`. A CLI argument overrides the environment variable:

```bash
FRAMEPIPE_PRIVILEGE_MODE=polkit framepipe record
framepipe record --privilege-mode direct
```

Polkit mode requires the packaged helper at `/usr/lib/framepipe/framepipe-privd` and the installed `dev.0x80.framepipe` policy (optional, but recommended).

For development builds, direct mode can be enabled by granting capabilities to the helper:

```bash
sudo setcap cap_sys_admin,cap_dac_override+ep target/debug/framepipe-privd
```

`cap_sys_admin` enables DRM/KMS capture. `cap_dac_override` enables access to input devices. The main `framepipe` binary does not receive either capability.

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
