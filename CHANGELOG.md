## [Unreleased]
### Added
- Native KMS cursor-plane composition, with CLI and GUI controls.
- Capture privilege modes (`auto`, `polkit`, and `direct`) and a polkit helper policy.

### Changed
- CPU encoding imports non-linear KMS DMABufs through GStreamer GL before downloading linear frames for software encoders.

### Fixed
- Prevent a mouse-resynchronization freeze during capture.

## [0.1.4] - 2026-07-05
### Added
- Replay buffer support.
- Hotkey support for replay buffer saves.

## [0.1.3] - 2026-06-04
### Added
- Add pages to gui for configuring capture and recording control.
- Add runtime libinput based hotkey support.
- Add key overlay support. Custom TTF + default bitmap. Modifier only overlay support.

### Changed
- Moved wayland tracking to separate crate and published as `gayland`.
- Update gui size for proper dialog scaling.

## [0.1.2] - 2026-04-27
### Changed
- Add tray icon support with KDE StatusNotifierItem (ksni).
- Fix for missing assets, and embed assets at compile time.
- Filter some noisy log messages from dependencies.
- Add icons.

## [0.1.1] - 2026-04-25
### Changed
- Re-enabled cursor composition for embedded previews. Oops.

## [0.1.0] - 2026-04-25
### Added
- Initial release of Framepipe, a zero-copy gpu accelerated screenrecorder for linux (wayland).
- Added support for xdg-desktop-portal Screencast API/Pipewire capture.
- Added support for DRM-KMS capture.
- Added support for background composition capture.
- Added support for cursor composition capture.
- Added support for ICed based GUI.
