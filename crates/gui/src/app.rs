use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[path = "ui.rs"]
mod ui;

use framepipe::app::cli::CaptureArgs;
use framepipe::app::hotkeys::{HotkeyAction, HotkeyBinding};
use framepipe::drm_kms::types::{CaptureOutput, LiveSettings};
use framepipe::embedded_preview::EmbeddedPreviewSession;
use framepipe::utils::tray::{TrayCallbacks, TrayController, spawn_tray};
use iced::{Subscription, Task, Theme};

use crate::model::{
    AppMode, BitrateModeChoice, CodecChoice, ColorRangeChoice, ColorimetryChoice, EncoderChoice,
    FixedOptions, FrameRateModeChoice, OutputContainerChoice, PrivilegeModeChoice, ProfileChoice,
    QualityChoice, SourceChoice, UiPage,
};
use crate::preview_shader::PreviewProgram;

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    TogglePreview,
    RestartPreview,
    StartRecording,
    StartReplayBuffer,
    StopRecording,
    SaveReplayBuffer,

    ThemeChanged(Theme),

    SourceChanged(SourceChoice),
    PrivilegeModeChanged(PrivilegeModeChoice),
    CardEdited(String),
    ConnectorEdited(String),
    AllowFallbackConnectorToggled(bool),
    FpsChanged(u32),
    OutputWidthEdited(String),
    OutputHeightEdited(String),
    DumpFramesToggled(bool),
    DumpEveryEdited(String),
    PickDumpDir,
    DumpDirPicked(Option<rfd::FileHandle>),
    FrameRateModeChanged(FrameRateModeChoice),
    BitrateModeChanged(BitrateModeChoice),
    QualityChanged(QualityChoice),
    CodecChanged(CodecChoice),
    EncoderChanged(EncoderChoice),
    BitrateEdited(String),
    ColorRangeChanged(ColorRangeChoice),
    ColorimetryChanged(ColorimetryChoice),
    ProfileChanged(ProfileChoice),
    CursorCompositionToggled(bool),
    WaylandSyncFrequencyEdited(String),
    CursorHotspotXEdited(String),
    CursorHotspotYEdited(String),
    CursorScaleChanged(f32),
    KeyboardOverlayToggled(bool),
    KeyboardOverlayDurationEdited(String),
    KeyboardOverlayFadeEdited(String),
    KeyboardOverlayDebounceEdited(String),
    KeyboardOverlayShowSingleModifiersToggled(bool),
    PickKeyboardOverlayFont,
    KeyboardOverlayFontPicked(Option<rfd::FileHandle>),
    KeyboardOverlayFontClear,
    HotkeysEnabledToggled(bool),
    HotkeyStopEdited(String),
    HotkeyPauseEdited(String),
    HotkeyResumeEdited(String),
    HotkeyTogglePauseEdited(String),
    HotkeySaveReplayBufferEdited(String),
    ApplyHotkeyStop,
    ApplyHotkeyPause,
    ApplyHotkeyResume,
    ApplyHotkeyTogglePause,
    ApplyHotkeySaveReplayBuffer,

    OutputContainerChanged(OutputContainerChoice),
    ReplaySecondsEdited(String),
    PickOutputPath,
    OutputPathPicked(Option<rfd::FileHandle>),
    OutputPathCleared,
    PickCursorSprite,
    CursorSpritePicked(Option<rfd::FileHandle>),
    CursorSpriteClear,
    PickBackgroundImage,
    BackgroundPicked(Option<rfd::FileHandle>),
    BackgroundClear,
    BackgroundToggled(bool),
    BackgroundZoomChanged(f32),
    CursorSmoothToggled(bool),
    CursorSmearToggled(bool),

    CursorSpringKChanged(f32),
    CursorSpringDChanged(f32),
    CursorMaxSpeedChanged(f32),
    CursorSnapPxChanged(f32),
    CursorSmoothMsChanged(f32),
    CursorDeadzonePxChanged(f32),
    CursorSmearSpeedThresholdChanged(f32),
    CursorSmearShutterScaleChanged(f32),
    CursorSmearMinLenChanged(f32),
    CursorSmearMaxLenChanged(f32),
    CursorSmearTapsChanged(u32),
    CursorSmearAlphaExpChanged(f32),
    CursorSmearAlphaScaleChanged(f32),
    CursorSmearStretchThresholdChanged(f32),
    CursorSmearStretchRangeChanged(f32),
    CursorSmearMaxStretchChanged(f32),
    CursorSmearMaxSquashChanged(f32),

    #[allow(unused)]
    TogglePausePreview,
    TogglePauseRecording,
    GoToConfigurePage,
    GoToRecordPage,
    GoToAdvancedPage,
}

#[derive(Debug, Clone, Copy)]
enum TrayCommand {
    StartRecording,
    StopRecording,
    TogglePause,
}

pub struct App {
    mode: AppMode,
    page: UiPage,
    status: String,

    fixed: FixedOptions,
    live: LiveSettings,

    fixed_dirty: bool,

    preview_session: Option<EmbeddedPreviewSession>,
    preview_join_thread: Option<std::thread::JoinHandle<Result<(), String>>>,
    pending_preview_start: bool,
    preview_program: Option<PreviewProgram>,
    preview_mailbox: Option<common::types::PreviewMailbox>,
    live_mailbox: Option<framepipe::drm_kms::types::LiveSettingsMailbox>,

    record_started_at: Option<Instant>,
    signal_control: framepipe::app::signals::CaptureControl,
    record_control: Option<framepipe::app::signals::CaptureControl>,
    record_thread: Option<std::thread::JoinHandle<Result<(), String>>>,
    replay_recording: bool,
    preview_control: Option<framepipe::app::signals::CaptureControl>,
    paused: bool,
    total_paused_duration: std::time::Duration,
    pause_start: Option<Instant>,
    ui_tick: u64,
    tray_rx: Option<mpsc::Receiver<TrayCommand>>,
    tray: Option<TrayController>,
    theme: Option<Theme>,
    background_cache: Option<iced::widget::image::Handle>,
}

impl App {
    pub fn new() -> Self {
        framepipe::init_logging("info");
        let signal_control =
            framepipe::app::signals::CaptureControl::register().unwrap_or_else(|e| {
                log::warn!("failed to register capture control signals in GUI: {e}");
                framepipe::app::signals::CaptureControl::new_unregistered()
            });
        let fixed = FixedOptions {
            source: SourceChoice::MonitorKms,
            ..Default::default()
        };

        let live = LiveSettings {
            fps: 60,
            ..Default::default()
        };

        let mut app = Self {
            mode: AppMode::Idle,
            page: UiPage::Configure,
            status: "Idle. ".to_string(),
            fixed,
            live,

            fixed_dirty: false,
            preview_session: None,
            preview_join_thread: None,
            pending_preview_start: false,
            preview_program: None,
            preview_mailbox: None,
            live_mailbox: None,
            record_started_at: None,
            signal_control,
            record_control: None,
            record_thread: None,
            replay_recording: false,
            preview_control: None,
            paused: false,
            total_paused_duration: std::time::Duration::ZERO,
            pause_start: None,
            ui_tick: 0,
            tray_rx: None,
            tray: None,
            theme: Some(Theme::Moonfly),
            background_cache: Some(iced::widget::image::Handle::from_bytes(
                &std::include_bytes!("../../../assets/grainy_bg1.png")[..],
            )),
        };

        let (tx, rx) = mpsc::channel::<TrayCommand>();
        let callbacks = TrayCallbacks {
            start_recording: {
                let tx = tx.clone();
                std::sync::Arc::new(move || {
                    let _ = tx.send(TrayCommand::StartRecording);
                })
            },
            stop_recording: {
                let tx = tx.clone();
                std::sync::Arc::new(move || {
                    let _ = tx.send(TrayCommand::StopRecording);
                })
            },
            pause_recording: {
                let tx = tx.clone();
                std::sync::Arc::new(move || {
                    let _ = tx.send(TrayCommand::TogglePause);
                })
            },
            resume_recording: {
                let tx = tx.clone();
                std::sync::Arc::new(move || {
                    let _ = tx.send(TrayCommand::TogglePause);
                })
            },
            hide_window: std::sync::Arc::new(|| {}),
            show_window: std::sync::Arc::new(|| {}),
            quit: std::sync::Arc::new(|| std::process::exit(0)),
        };
        app.tray = spawn_tray(
            framepipe::utils::types::ProcessState::Stopped(None),
            callbacks,
        )
        .ok();
        app.tray_rx = Some(rx);

        app
    }

    pub fn btn<'a>(
        content: impl Into<iced::Element<'a, Message>>,
    ) -> iced::widget::Button<'a, Message> {
        iced::widget::button(content).style(|theme, status| {
            let mut style = iced::widget::button::primary(theme, status);
            style.border.radius = 8.0.into();
            style
        })
    }

    pub fn theme(&self) -> Option<Theme> {
        self.theme.clone()
    }

    fn parse_opt_u32(input: &str) -> Option<u32> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            None
        } else {
            trimmed.parse::<u32>().ok()
        }
    }

    fn parse_or<T: std::str::FromStr + Copy>(input: &str, fallback: T) -> T {
        input.trim().parse::<T>().ok().unwrap_or(fallback)
    }

    fn build_capture_args(&self) -> CaptureArgs {
        let hotkeys = if self.fixed.hotkeys_enabled {
            self.configured_hotkeys().unwrap_or_default()
        } else {
            Vec::new()
        };
        CaptureArgs {
            capture_backend: self.fixed.source.to_backend(),
            privilege_mode: self.fixed.privilege_mode.to_mode(),
            card: (!self.fixed.card.trim().is_empty()).then(|| self.fixed.card.trim().to_string()),
            connector: (!self.fixed.connector.trim().is_empty())
                .then(|| self.fixed.connector.trim().to_string()),
            allow_fallback_connector: self.fixed.allow_fallback_connector,
            fps: self.live.fps.max(1),
            hotkeys,
            disable_hotkeys: !self.fixed.hotkeys_enabled,
            keyboard_overlay: self.fixed.keyboard_overlay,
            keyboard_overlay_duration_ms: Self::parse_or(
                &self.fixed.keyboard_overlay_duration_ms,
                framepipe::capture::key_overlay::DEFAULT_DISPLAY_DURATION_MS,
            )
            .max(1),
            keyboard_overlay_fade_ms: Self::parse_or(
                &self.fixed.keyboard_overlay_fade_ms,
                framepipe::capture::key_overlay::DEFAULT_FADE_DURATION_MS,
            )
            .max(1),
            keyboard_overlay_debounce_ms: Self::parse_or(
                &self.fixed.keyboard_overlay_debounce_ms,
                framepipe::capture::key_overlay::DEFAULT_DEBOUNCE_MS,
            ),
            keyboard_overlay_show_single_modifiers: self
                .fixed
                .keyboard_overlay_show_single_modifiers,
            keyboard_overlay_font: self.fixed.keyboard_overlay_font.clone(),
            output_width: Self::parse_opt_u32(&self.fixed.output_width),
            output_height: Self::parse_opt_u32(&self.fixed.output_height),
            dump_frames: self.fixed.dump_frames,
            dump_dir: self.fixed.dump_dir.clone(),
            dump_every: Self::parse_or(&self.fixed.dump_every, 30_u32).max(1),
            output_container: self.fixed.output_container.to_container(),
            bitrate_kbps: Self::parse_or(&self.fixed.bitrate_input, 15000_u32).max(1),
            frame_rate_mode: self.fixed.frame_rate_mode.to_mode(),
            bitrate_mode: self.fixed.bitrate_mode.to_mode(),
            quality: self.fixed.quality.to_quality(),
            color_range: self.fixed.color_range.to_color_range(),
            colorimetry: self.fixed.colorimetry.to_colorimetry(),
            encoder_backend: self.fixed.encoder.to_encoder(),
            video_codec: self.fixed.codec.to_codec(),
            cursor_composition: self.fixed.cursor_composition,
            cursor_hotspot_x: Self::parse_or(&self.fixed.cursor_hotspot_x, 0_i32),
            cursor_hotspot_y: Self::parse_or(&self.fixed.cursor_hotspot_y, 0_i32),
            cursor_scale: self.live.cursor_scale.clamp(1.0, 100.0),
            cursor_smooth: self.live.cursor_smooth,
            cursor_smear: self.live.cursor_smear,
            cursor_spring_k: self.live.cursor_spring_k,
            cursor_spring_d: self.live.cursor_spring_d,
            cursor_max_speed: self.live.cursor_max_speed,
            cursor_snap_px: self.live.cursor_snap_px,
            cursor_smooth_ms: self.live.cursor_smooth_ms,
            cursor_deadzone_px: self.live.cursor_deadzone_px,
            cursor_smear_speed_threshold: self.live.cursor_smear_speed_threshold,
            cursor_smear_shutter_scale: self.live.cursor_smear_shutter_scale,
            cursor_smear_min_len: self.live.cursor_smear_min_len,
            cursor_smear_max_len: self.live.cursor_smear_max_len,
            cursor_smear_taps: self.live.cursor_smear_taps.clamp(1, 8),
            cursor_smear_alpha_exp: self.live.cursor_smear_alpha_exp,
            cursor_smear_alpha_scale: self.live.cursor_smear_alpha_scale,
            cursor_smear_stretch_threshold: self.live.cursor_smear_stretch_threshold,
            cursor_smear_stretch_range: self.live.cursor_smear_stretch_range,
            cursor_smear_max_stretch: self.live.cursor_smear_max_stretch,
            cursor_smear_max_squash: self.live.cursor_smear_max_squash,
            wayland_sync_frequency: Self::parse_or(&self.fixed.wayland_sync_frequency, 0.5_f64),
            profile: self.fixed.profile.to_profile(),
            cursor_sprite: self.live.cursor_sprite.clone(),
            background: self.live.background.clone(),
            background_zoom: self.live.background_zoom.clamp(1.0, 100.0),
        }
    }

    fn apply_live(&self) {
        if let Some(mb) = &self.live_mailbox {
            mb.update(self.live.clone());
        }
    }

    fn short_path(path: Option<&PathBuf>) -> String {
        match path {
            Some(p) => {
                let raw = p.to_string_lossy().into_owned();
                let max_chars = 64usize;
                if raw.chars().count() <= max_chars {
                    raw
                } else {
                    let head_len = 28usize;
                    let tail_len = 28usize;
                    let head: String = raw.chars().take(head_len).collect();
                    let tail: String = raw
                        .chars()
                        .rev()
                        .take(tail_len)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect();
                    format!("{head}…{tail}")
                }
            }
            None => "None".to_string(),
        }
    }

    fn ui_tick_period(&self) -> Duration {
        match self.mode {
            AppMode::Preview => {
                let fps = self.live.fps.clamp(1, 240) as u64;
                Duration::from_micros((1_000_000 / fps).max(4_000))
            }
            AppMode::Recording => Duration::from_millis(100),
            AppMode::Idle => Duration::from_millis(250),
        }
    }

    fn default_output_path(&self, prefix: &str) -> PathBuf {
        let fmt = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
        let filename = format!("{prefix}_{fmt}.{}", self.fixed.output_container.extension());
        if let Some(mut dir) = dirs::video_dir() {
            dir.push(filename);
            dir
        } else {
            PathBuf::from(filename)
        }
    }

    fn start_capture_output(&mut self, output: CaptureOutput, replay: bool) {
        self.page = UiPage::Record;
        self.stop_preview_session();
        if self.preview_join_thread.is_some() {
            self.status = "Waiting for preview teardown before recording…".to_string();
            return;
        }

        let output_path = match &output {
            CaptureOutput::File(path) | CaptureOutput::ReplayBuffer(path, _) => path.clone(),
            CaptureOutput::Preview | CaptureOutput::EmbeddedPreview => PathBuf::new(),
        };
        let session_control = framepipe::app::signals::CaptureControl::new_unregistered();
        let control = session_control.clone();

        let args = self.build_capture_args();
        let backend = args.capture_backend;

        match framepipe::app::config::build_capture_options(args, output) {
            Ok(options) => {
                self.mode = AppMode::Recording;
                self.replay_recording = replay;
                self.record_started_at = Some(Instant::now());
                self.total_paused_duration = std::time::Duration::ZERO;
                self.pause_start = None;
                self.status = if replay {
                    format!(
                        "Replay buffer running, save target {}",
                        output_path.display()
                    )
                } else {
                    format!("Recording to {}", output_path.display())
                };
                self.record_control = Some(control.clone());
                self.record_thread = Some(std::thread::spawn(move || {
                    framepipe::app::app::RecordingSession::new(options, backend)
                        .and_then(|s| s.run(control))
                        .map_err(|e| e.to_string())
                }));
            }
            Err(e) => {
                self.status = format!("Failed to start recording: {e}");
            }
        }
    }

    fn recording_elapsed(&self) -> String {
        if let Some(start) = self.record_started_at {
            let mut duration = start.elapsed();
            if let Some(ps) = self.pause_start {
                duration = duration.saturating_sub(ps.elapsed());
            }
            duration = duration.saturating_sub(self.total_paused_duration);
            let secs = duration.as_secs();
            let h = secs / 3600;
            let m = (secs % 3600) / 60;
            let s = secs % 60;
            format!("{h:02}:{m:02}:{s:02}")
        } else {
            "00:00:00".to_string()
        }
    }

    fn sync_recording_pause_state(&mut self) {
        let Some(control) = &self.record_control else {
            return;
        };
        let actual = control.paused.load(std::sync::atomic::Ordering::Relaxed);
        if actual == self.paused {
            return;
        }

        self.paused = actual;
        if actual {
            self.pause_start.get_or_insert_with(Instant::now);
            self.status = if self.replay_recording {
                "Replay buffer paused".to_string()
            } else {
                "Recording paused".to_string()
            };
        } else {
            if let Some(ps) = self.pause_start.take() {
                self.total_paused_duration += ps.elapsed();
            }
            self.status = if self.replay_recording {
                "Replay buffer resumed".to_string()
            } else {
                "Recording resumed".to_string()
            };
        }
    }

    fn stop_preview_session(&mut self) {
        if let Some(session) = self.preview_session.take() {
            session.stop();
            self.preview_join_thread = Some(std::thread::spawn(move || {
                session.join().map_err(|e| e.to_string())
            }));
            self.status = "Stopping preview…".to_string();
        }
        self.preview_program = None;
        self.preview_mailbox = None;
        self.live_mailbox = None;
        self.preview_control = None;
        self.paused = false;
        if matches!(self.mode, AppMode::Preview) {
            self.mode = AppMode::Idle;
        }
    }

    fn poll_background_threads(&mut self) {
        if let Some(handle) = self.preview_join_thread.as_ref()
            && handle.is_finished()
        {
            let handle = self
                .preview_join_thread
                .take()
                .expect("preview join thread exists");
            match handle.join() {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    self.status = format!("Preview stop failed: {e}");
                    log::warn!("preview stop failed: {e}");
                }
                Err(_) => {
                    self.status = "Preview stop thread panicked".to_string();
                    log::warn!("preview stop thread panicked");
                }
            }
            if self.pending_preview_start {
                self.pending_preview_start = false;
                self.start_preview_session();
            }
        }

        if let Some(handle) = self.record_thread.as_ref()
            && handle.is_finished()
        {
            let handle = self.record_thread.take().expect("record thread exists");
            let result = handle
                .join()
                .map_err(|_| "record thread panicked".to_string())
                .and_then(|v| v);
            self.record_started_at = None;
            self.record_control = None;
            self.replay_recording = false;
            self.paused = false;
            self.total_paused_duration = std::time::Duration::ZERO;
            self.pause_start = None;
            self.mode = AppMode::Idle;
            match result {
                Ok(()) => {
                    self.status = "Recording stopped".to_string();
                }
                Err(e) => {
                    self.status = format!("Recording failed: {e}");
                    log::warn!("recording failed: {e}");
                }
            }
        }
    }

    fn process_tray_commands(&mut self) {
        let mut pending = Vec::new();
        if let Some(rx) = &self.tray_rx {
            while let Ok(cmd) = rx.try_recv() {
                pending.push(cmd);
            }
        }
        for cmd in pending {
            match cmd {
                TrayCommand::StartRecording => {
                    if matches!(self.mode, AppMode::Idle) {
                        self.page = UiPage::Record;
                        let _ = self.update(Message::StartRecording);
                    }
                }
                TrayCommand::StopRecording => {
                    if matches!(self.mode, AppMode::Recording) {
                        let _ = self.update(Message::StopRecording);
                    }
                }
                TrayCommand::TogglePause => {
                    if matches!(self.mode, AppMode::Recording) {
                        let _ = self.update(Message::TogglePauseRecording);
                    }
                }
            }
        }
    }

    fn configured_hotkeys(&self) -> Result<Vec<HotkeyBinding>, String> {
        let mut bindings = Vec::new();
        push_hotkey_binding(&mut bindings, HotkeyAction::Stop, &self.fixed.hotkey_stop)?;
        push_hotkey_binding(&mut bindings, HotkeyAction::Pause, &self.fixed.hotkey_pause)?;
        push_hotkey_binding(
            &mut bindings,
            HotkeyAction::Resume,
            &self.fixed.hotkey_resume,
        )?;
        push_hotkey_binding(
            &mut bindings,
            HotkeyAction::TogglePause,
            &self.fixed.hotkey_toggle_pause,
        )?;
        push_hotkey_binding(
            &mut bindings,
            HotkeyAction::SaveReplayBuffer,
            &self.fixed.hotkey_save_replay_buffer,
        )?;
        Ok(bindings)
    }

    fn sync_tray_state(&self) {
        if let Some(tray) = &self.tray {
            let state = match self.mode {
                AppMode::Recording if self.paused => framepipe::utils::types::ProcessState::Paused,
                AppMode::Recording => framepipe::utils::types::ProcessState::Running,
                AppMode::Preview => framepipe::utils::types::ProcessState::Preview,
                AppMode::Idle => framepipe::utils::types::ProcessState::Stopped(None),
            };
            tray.set_process_state(state);
            let control = if matches!(self.mode, AppMode::Recording) {
                self.record_control.clone()
            } else if matches!(self.mode, AppMode::Preview) {
                self.preview_control.clone()
            } else {
                None
            };
            tray.set_capture_control(control);
        }
    }

    fn start_preview_session(&mut self) {
        self.page = UiPage::Configure;
        if self.preview_join_thread.is_some() {
            self.pending_preview_start = true;
            self.status = "Waiting for previous preview to stop…".to_string();
            return;
        }
        self.stop_preview_session();
        if self.preview_join_thread.is_some() {
            self.pending_preview_start = true;
            self.status = "Waiting for previous preview to stop…".to_string();
            return;
        }

        let session_control = framepipe::app::signals::CaptureControl::new_unregistered();
        match framepipe::embedded_preview::start_embedded_preview_with_control(
            self.build_capture_args(),
            session_control,
        ) {
            Ok(session) => {
                let preview_mailbox = session.mailbox();
                let live_mailbox = session.live_settings();
                live_mailbox.update(self.live.clone());
                // Expose the CaptureControl so we can pause/resume the preview.
                let control = session.control();
                self.preview_control = Some(control);

                self.preview_program = Some(PreviewProgram::new(preview_mailbox.clone()));
                self.preview_mailbox = Some(preview_mailbox);
                self.live_mailbox = Some(live_mailbox);
                self.preview_session = Some(session);
                self.mode = AppMode::Preview;
                self.fixed_dirty = false;
                self.paused = false;
                self.status = "Preview running".to_string();
            }
            Err(e) => {
                self.mode = AppMode::Idle;
                self.status = format!("Failed to start preview: {e}");
            }
        }
    }

    fn mark_fixed_changed(&mut self) {
        if matches!(self.mode, AppMode::Preview) {
            self.fixed_dirty = true;
            self.status = "Fixed settings changed. Restart preview to apply.".to_string();
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ThemeChanged(theme) => {
                self.theme = Some(theme);
                Task::none()
            }
            Message::Tick => {
                self.ui_tick = self.ui_tick.wrapping_add(1);
                self.poll_background_threads();
                self.process_tray_commands();
                self.sync_recording_pause_state();
                self.sync_tray_state();

                // Handle SIGINT/SIGTERM: gracefully shut down and exit the GUI.
                if self
                    .signal_control
                    .stop_requested
                    .load(std::sync::atomic::Ordering::Relaxed)
                {
                    log::info!("SIGINT/SIGTERM received, shutting down GUI");
                    self.stop_preview_session();
                    if let Some(control) = self.record_control.take() {
                        control.request_stop();
                    }
                    return iced::exit();
                }

                Task::none()
            }
            Message::TogglePreview => {
                match self.mode {
                    AppMode::Idle => self.start_preview_session(),
                    AppMode::Preview => {
                        self.stop_preview_session();
                        self.status = "Preview stopped".to_string();
                    }
                    AppMode::Recording => {}
                }
                Task::none()
            }
            Message::RestartPreview => {
                // self.stop_preview_session();
                if matches!(self.mode, AppMode::Preview) || matches!(self.mode, AppMode::Idle) {
                    self.start_preview_session();
                }
                Task::none()
            }
            Message::StartRecording => {
                let output = self
                    .fixed
                    .output_path
                    .clone()
                    .unwrap_or_else(|| self.default_output_path("framepipe_record"));
                self.start_capture_output(CaptureOutput::File(output), false);
                Task::none()
            }
            Message::StartReplayBuffer => {
                let output = self
                    .fixed
                    .output_path
                    .clone()
                    .unwrap_or_else(|| self.default_output_path("framepipe_replay"));
                let seconds = Self::parse_or(&self.fixed.replay_seconds, 30_u32).max(1);
                self.start_capture_output(CaptureOutput::ReplayBuffer(output, seconds), true);
                Task::none()
            }
            Message::StopRecording => {
                if let Some(control) = self.record_control.take() {
                    control.request_stop();
                }
                self.status = format!(
                    "Stopping recording. Output target: {}",
                    self.fixed
                        .output_path
                        .as_ref()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "Dynamic".to_string())
                );
                Task::none()
            }
            Message::SaveReplayBuffer => {
                if let Some(control) = &self.record_control {
                    control.request_save_replay_buffer();
                    self.status = "Replay save requested".to_string();
                }
                Task::none()
            }
            Message::GoToConfigurePage => {
                if matches!(self.mode, AppMode::Recording) {
                    return Task::none();
                }
                self.page = UiPage::Configure;
                Task::none()
            }
            Message::GoToRecordPage => {
                if matches!(self.mode, AppMode::Preview) {
                    self.stop_preview_session();
                }
                self.page = UiPage::Record;
                Task::none()
            }
            Message::GoToAdvancedPage => {
                if matches!(self.mode, AppMode::Recording) {
                    return Task::none();
                }
                self.page = UiPage::Advanced;
                Task::none()
            }

            Message::SourceChanged(v) => {
                self.fixed.source = v;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::PrivilegeModeChanged(v) => {
                self.fixed.privilege_mode = v;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::CardEdited(v) => {
                self.fixed.card = v;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::ConnectorEdited(v) => {
                self.fixed.connector = v;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::AllowFallbackConnectorToggled(v) => {
                self.fixed.allow_fallback_connector = v;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::FpsChanged(v) => {
                self.live.fps = v.max(1);
                self.apply_live();
                Task::none()
            }
            Message::OutputWidthEdited(v) => {
                self.fixed.output_width = v;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::OutputHeightEdited(v) => {
                self.fixed.output_height = v;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::DumpFramesToggled(v) => {
                self.fixed.dump_frames = v;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::DumpEveryEdited(v) => {
                self.fixed.dump_every = v;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::PickDumpDir => {
                Task::future(rfd::AsyncFileDialog::new().pick_folder()).map(Message::DumpDirPicked)
            }
            Message::DumpDirPicked(handle) => {
                if let Some(file) = handle {
                    self.fixed.dump_dir = file.path().to_path_buf();
                    self.mark_fixed_changed();
                }
                Task::none()
            }
            Message::FrameRateModeChanged(v) => {
                self.fixed.frame_rate_mode = v;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::BitrateModeChanged(v) => {
                self.fixed.bitrate_mode = v;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::QualityChanged(v) => {
                self.fixed.quality = v;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::CodecChanged(v) => {
                self.fixed.codec = v;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::EncoderChanged(v) => {
                self.fixed.encoder = v;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::BitrateEdited(v) => {
                self.fixed.bitrate_input = v;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::ColorRangeChanged(v) => {
                self.fixed.color_range = v;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::ColorimetryChanged(v) => {
                self.fixed.colorimetry = v;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::ProfileChanged(v) => {
                self.fixed.profile = v;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::CursorCompositionToggled(v) => {
                self.fixed.cursor_composition = v;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::WaylandSyncFrequencyEdited(v) => {
                self.fixed.wayland_sync_frequency = v;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::CursorHotspotXEdited(v) => {
                self.fixed.cursor_hotspot_x = v;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::CursorHotspotYEdited(v) => {
                self.fixed.cursor_hotspot_y = v;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::CursorScaleChanged(v) => {
                self.live.cursor_scale = v;
                self.live.cursor_sprite_version = self.live.cursor_sprite_version.wrapping_add(1);
                self.apply_live();
                Task::none()
            }
            Message::KeyboardOverlayToggled(v) => {
                self.fixed.keyboard_overlay = v;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::KeyboardOverlayDurationEdited(v) => {
                self.fixed.keyboard_overlay_duration_ms = v;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::KeyboardOverlayFadeEdited(v) => {
                self.fixed.keyboard_overlay_fade_ms = v;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::KeyboardOverlayDebounceEdited(v) => {
                self.fixed.keyboard_overlay_debounce_ms = v;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::KeyboardOverlayShowSingleModifiersToggled(v) => {
                self.fixed.keyboard_overlay_show_single_modifiers = v;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::PickKeyboardOverlayFont => Task::future(
                rfd::AsyncFileDialog::new()
                    .add_filter("Font", &["ttf", "otf"])
                    .pick_file(),
            )
            .map(Message::KeyboardOverlayFontPicked),
            Message::KeyboardOverlayFontPicked(handle) => {
                if let Some(file) = handle {
                    self.fixed.keyboard_overlay_font = Some(file.path().to_path_buf());
                    self.mark_fixed_changed();
                }
                Task::none()
            }
            Message::KeyboardOverlayFontClear => {
                self.fixed.keyboard_overlay_font = None;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::HotkeysEnabledToggled(enabled) => {
                self.fixed.hotkeys_enabled = enabled;
                self.status = if enabled {
                    "Recording-time hotkeys enabled".to_string()
                } else {
                    "Recording-time hotkeys disabled".to_string()
                };
                Task::none()
            }
            Message::HotkeyStopEdited(v) => {
                self.fixed.hotkey_stop = v;
                Task::none()
            }
            Message::HotkeyPauseEdited(v) => {
                self.fixed.hotkey_pause = v;
                Task::none()
            }
            Message::HotkeyResumeEdited(v) => {
                self.fixed.hotkey_resume = v;
                Task::none()
            }
            Message::HotkeyTogglePauseEdited(v) => {
                self.fixed.hotkey_toggle_pause = v;
                Task::none()
            }
            Message::HotkeySaveReplayBufferEdited(v) => {
                self.fixed.hotkey_save_replay_buffer = v;
                Task::none()
            }
            Message::ApplyHotkeyStop
            | Message::ApplyHotkeyPause
            | Message::ApplyHotkeyResume
            | Message::ApplyHotkeyTogglePause
            | Message::ApplyHotkeySaveReplayBuffer => {
                self.status = match self.configured_hotkeys() {
                    Ok(bindings) => format!("Saved {} recording-time hotkeys", bindings.len()),
                    Err(e) => format!("Invalid hotkey: {e}"),
                };
                Task::none()
            }
            Message::OutputContainerChanged(container) => {
                self.fixed.output_container = container;
                if let Some(path) = &mut self.fixed.output_path {
                    let ext = path
                        .extension()
                        .and_then(|v| v.to_str())
                        .map(str::to_ascii_lowercase);
                    if matches!(ext.as_deref(), Some("mp4" | "mkv" | "matroska")) {
                        path.set_extension(container.extension());
                    }
                }
                self.mark_fixed_changed();
                Task::none()
            }
            Message::ReplaySecondsEdited(v) => {
                self.fixed.replay_seconds = v;
                self.mark_fixed_changed();
                Task::none()
            }

            Message::PickOutputPath => Task::future(
                rfd::AsyncFileDialog::new()
                    .add_filter("Video", &["mp4", "mkv"])
                    .set_file_name(format!(
                        "output.{}",
                        self.fixed.output_container.extension()
                    ))
                    .save_file(),
            )
            .map(Message::OutputPathPicked),
            Message::OutputPathPicked(handle) => {
                if let Some(file) = handle {
                    self.fixed.output_path = Some(file.path().to_path_buf());
                    self.mark_fixed_changed();
                }
                Task::none()
            }
            Message::OutputPathCleared => {
                self.fixed.output_path = None;
                self.mark_fixed_changed();
                Task::none()
            }
            Message::PickCursorSprite => Task::future(
                rfd::AsyncFileDialog::new()
                    .add_filter("Image Formats", &["png", "jpg", "jpeg"])
                    .pick_file(),
            )
            .map(Message::CursorSpritePicked),
            Message::CursorSpritePicked(handle) => {
                if let Some(file) = handle {
                    self.live.cursor_sprite = Some(file.path().to_path_buf());
                    self.live.cursor_sprite_version =
                        self.live.cursor_sprite_version.wrapping_add(1);
                    self.apply_live();
                }
                Task::none()
            }
            Message::CursorSpriteClear => {
                self.live.cursor_sprite = None;
                self.live.cursor_sprite_version = self.live.cursor_sprite_version.wrapping_add(1);
                self.apply_live();
                Task::none()
            }
            Message::PickBackgroundImage => Task::future(
                rfd::AsyncFileDialog::new()
                    .add_filter("Image Formats", &["png", "jpg", "jpeg"])
                    .pick_file(),
            )
            .map(Message::BackgroundPicked),
            Message::BackgroundPicked(handle) => {
                if let Some(file) = handle {
                    self.live.background = Some(file.path().to_path_buf());
                    self.live.background_enabled = true;
                    self.live.background_version = self.live.background_version.wrapping_add(1);
                    self.apply_live();
                }
                Task::none()
            }
            Message::BackgroundClear => {
                self.live.background = None;
                self.live.background_enabled = false;
                self.live.background_version = self.live.background_version.wrapping_add(1);
                self.apply_live();
                Task::none()
            }
            Message::BackgroundToggled(v) => {
                self.live.background_enabled = v;
                self.apply_live();
                Task::none()
            }
            Message::BackgroundZoomChanged(v) => {
                self.live.background_zoom = v.clamp(1.0, 100.0);
                self.apply_live();
                Task::none()
            }
            Message::CursorSmoothToggled(v) => {
                self.live.cursor_smooth = v;
                self.apply_live();
                Task::none()
            }
            Message::CursorSmearToggled(v) => {
                self.live.cursor_smear = v;
                self.apply_live();
                Task::none()
            }
            Message::CursorSpringKChanged(v) => {
                self.live.cursor_spring_k = v;
                self.apply_live();
                Task::none()
            }
            Message::CursorSpringDChanged(v) => {
                self.live.cursor_spring_d = v;
                self.apply_live();
                Task::none()
            }
            Message::CursorMaxSpeedChanged(v) => {
                self.live.cursor_max_speed = v;
                self.apply_live();
                Task::none()
            }
            Message::CursorSnapPxChanged(v) => {
                self.live.cursor_snap_px = v;
                self.apply_live();
                Task::none()
            }
            Message::CursorSmoothMsChanged(v) => {
                self.live.cursor_smooth_ms = v;
                self.apply_live();
                Task::none()
            }
            Message::CursorDeadzonePxChanged(v) => {
                self.live.cursor_deadzone_px = v;
                self.apply_live();
                Task::none()
            }
            Message::CursorSmearSpeedThresholdChanged(v) => {
                self.live.cursor_smear_speed_threshold = v;
                self.apply_live();
                Task::none()
            }
            Message::CursorSmearShutterScaleChanged(v) => {
                self.live.cursor_smear_shutter_scale = v;
                self.apply_live();
                Task::none()
            }
            Message::CursorSmearMinLenChanged(v) => {
                self.live.cursor_smear_min_len = v;
                self.apply_live();
                Task::none()
            }
            Message::CursorSmearMaxLenChanged(v) => {
                self.live.cursor_smear_max_len = v;
                self.apply_live();
                Task::none()
            }
            Message::CursorSmearTapsChanged(v) => {
                self.live.cursor_smear_taps = v.clamp(1, 8);
                self.apply_live();
                Task::none()
            }
            Message::CursorSmearAlphaExpChanged(v) => {
                self.live.cursor_smear_alpha_exp = v;
                self.apply_live();
                Task::none()
            }
            Message::CursorSmearAlphaScaleChanged(v) => {
                self.live.cursor_smear_alpha_scale = v;
                self.apply_live();
                Task::none()
            }
            Message::CursorSmearStretchThresholdChanged(v) => {
                self.live.cursor_smear_stretch_threshold = v;
                self.apply_live();
                Task::none()
            }
            Message::CursorSmearStretchRangeChanged(v) => {
                self.live.cursor_smear_stretch_range = v;
                self.apply_live();
                Task::none()
            }
            Message::CursorSmearMaxStretchChanged(v) => {
                self.live.cursor_smear_max_stretch = v;
                self.apply_live();
                Task::none()
            }
            Message::CursorSmearMaxSquashChanged(v) => {
                self.live.cursor_smear_max_squash = v;
                self.apply_live();
                Task::none()
            }

            Message::TogglePausePreview => {
                if let Some(ctrl) = &self.preview_control {
                    if self.paused {
                        ctrl.request_resume();
                        ctrl.paused
                            .store(false, std::sync::atomic::Ordering::Relaxed);
                        self.paused = false;
                        self.status = "Preview resumed".to_string();
                    } else {
                        ctrl.request_pause();
                        ctrl.paused
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                        self.paused = true;
                        self.status = "Preview paused".to_string();
                    }
                }
                Task::none()
            }
            Message::TogglePauseRecording => {
                if let Some(ctrl) = &self.record_control {
                    if self.paused {
                        ctrl.request_resume();
                        ctrl.paused
                            .store(false, std::sync::atomic::Ordering::Relaxed);
                        self.paused = false;
                        if let Some(ps) = self.pause_start.take() {
                            self.total_paused_duration += ps.elapsed();
                        }
                        self.status = format!(
                            "Recording to {}",
                            self.fixed
                                .output_path
                                .as_ref()
                                .map(|p| p.to_string_lossy().into_owned())
                                .unwrap_or_else(|| "Dynamic".to_string())
                        );
                    } else {
                        ctrl.request_pause();
                        ctrl.paused
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                        self.paused = true;
                        self.pause_start = Some(Instant::now());
                        self.status = "Recording paused".to_string();
                    }
                }
                Task::none()
            }
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        iced::time::every(self.ui_tick_period()).map(|_| Message::Tick)
    }
}

fn push_hotkey_binding(
    bindings: &mut Vec<HotkeyBinding>,
    action: HotkeyAction,
    input: &str,
) -> Result<(), String> {
    let hotkey = input.trim();
    if hotkey.is_empty() {
        return Ok(());
    }
    bindings.push(HotkeyBinding {
        action,
        spec: hotkey.parse().map_err(|e| format!("{action:?}: {e}"))?,
    });
    Ok(())
}

impl Drop for App {
    fn drop(&mut self) {
        self.tray = None;
        self.stop_preview_session();
        if let Some(handle) = self.preview_join_thread.take() {
            let _ = handle.join();
        }
        if let Some(control) = self.record_control.take() {
            control.request_stop();
        }
        if let Some(handle) = self.record_thread.take() {
            let _ = handle.join();
        }
        self.replay_recording = false;
    }
}
