## [Unreleased]
### Changed
- Moved wayland tracking to separate crate and published as `gayland`.
- Update gui size for proper dialog scaling.
- Add pages to gui for configuring capture and recording control.
- Add runtime libinput based hotkey support.
- Add key overlay support. Custom TTF + default bitmap. Modifier only overlay support.

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