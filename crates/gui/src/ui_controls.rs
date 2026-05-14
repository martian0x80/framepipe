use crate::app::{App, Message};
use crate::model::{
    AppMode, BitrateModeChoice, CodecChoice, ColorRangeChoice, ColorimetryChoice, EncoderChoice,
    FrameRateModeChoice, ProfileChoice, QualityChoice, SourceChoice,
};
use crate::theme::get_all_themes;
use iced::widget::scrollable::Scrollbar;
use iced::widget::{
    button, column, container, pick_list, row, scrollable, slider, text, text_input, toggler,
};
use iced::{Alignment, Color, Element, Length, Theme};

impl App {
    fn section_heading(title: &str) -> iced::widget::Text<'_> {
        text(title).size(20).font(iced::Font {
            weight: iced::font::Weight::Bold,
            ..Default::default()
        })
    }

    fn subsection_label(title: &str) -> iced::widget::Text<'_> {
        text(title)
            .size(13)
            .color(Color::from_rgb(0.5, 0.5, 0.5))
            .font(iced::Font {
                weight: iced::font::Weight::Bold,
                ..Default::default()
            })
    }

    fn inset_divider() -> Element<'static, Message> {
        container(
            container("")
                .height(Length::Fixed(1.0))
                .width(Length::Fill)
                .style(|theme: &iced::Theme| {
                    let p = theme.extended_palette();
                    container::Style {
                        background: Some(
                            Color::from_rgba(
                                p.secondary.strong.color.r,
                                p.secondary.strong.color.g,
                                p.secondary.strong.color.b,
                                0.4,
                            )
                            .into(),
                        ),
                        ..Default::default()
                    }
                }),
        )
        .padding([6, 0])
        .into()
    }

    /// Styled card container for grouping related controls
    fn card<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
        container(content)
            .padding(12)
            .width(Length::Fill)
            .style(|theme: &Theme| {
                let palette = theme.extended_palette();
                iced::widget::container::Style {
                    background: Some(
                        Color::from_rgba(
                            palette.background.strong.color.r,
                            palette.background.strong.color.g,
                            palette.background.strong.color.b,
                            0.35,
                        )
                        .into(),
                    ),
                    border: iced::border::rounded(10),
                    ..Default::default()
                }
            })
            .into()
    }

    fn output_path_row(&self, disabled: bool) -> iced::widget::Row<'_, Message> {
        row![
            text("Output"),
            container(
                text(match &self.fixed.output_path {
                    Some(path) => Self::short_path(Some(path)),
                    None => match dirs::video_dir() {
                        Some(v) => format!("{}/framepipe_record_<time>.mp4", v.display()),
                        None => "framepipe_record_<time>.mp4".to_string(),
                    },
                })
                .size(13)
            )
            .width(Length::Fill),
            App::btn("Browse").on_press_maybe((!disabled).then_some(Message::PickOutputPath)),
            App::btn("Clear").on_press_maybe((!disabled).then_some(Message::OutputPathCleared)),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
    }

    pub(super) fn configure_controls_panel(&self) -> Element<'_, Message> {
        let disabled = matches!(self.mode, crate::model::AppMode::Recording);

        let capture_card = Self::card(
            column![
                Self::subsection_label("CAPTURE"),
                row![
                    text("Source"),
                    pick_list(
                        &SourceChoice::ALL[..],
                        Some(self.fixed.source),
                        Message::SourceChanged
                    )
                    .width(Length::Fixed(160.0))
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                row![
                    text("Connector"),
                    text_input("eDP-1", &self.fixed.connector)
                        .on_input_maybe((!disabled).then_some(Message::ConnectorEdited))
                        .width(Length::Fill),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                row![
                    text(format!("FPS {}", self.live.fps)),
                    slider(1..=240, self.live.fps, Message::FpsChanged).width(Length::Fill),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .spacing(8),
        );

        let encoding_card = Self::card({
            let mut col = column![
                Self::subsection_label("ENCODING"),
                row![
                    text("Quality"),
                    pick_list(
                        &QualityChoice::ALL[..],
                        Some(self.fixed.quality),
                        Message::QualityChanged
                    )
                    .width(Length::Fixed(120.0))
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                row![
                    text("Codec"),
                    pick_list(
                        &CodecChoice::ALL[..],
                        Some(self.fixed.codec),
                        Message::CodecChanged
                    )
                    .width(Length::Fixed(110.0)),
                    text("Encoder"),
                    pick_list(
                        &EncoderChoice::ALL[..],
                        Some(self.fixed.encoder),
                        Message::EncoderChanged
                    )
                    .width(Length::Fixed(110.0))
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                row![
                    text("Color"),
                    pick_list(
                        &ColorRangeChoice::ALL[..],
                        Some(self.fixed.color_range),
                        Message::ColorRangeChanged
                    )
                    .width(Length::Fixed(120.0)),
                    text("Profile"),
                    pick_list(
                        &ProfileChoice::ALL[..],
                        Some(self.fixed.profile),
                        Message::ProfileChanged
                    )
                    .width(Length::Fixed(120.0)),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                self.output_path_row(disabled),
                row![
                    toggler(self.live.background_enabled)
                        .label("Background")
                        .on_toggle_maybe((!disabled).then_some(Message::BackgroundToggled)),
                    container(text(Self::short_path(self.live.background.as_ref())).size(12))
                        .width(Length::Fill),
                    App::btn("Browse")
                        .on_press_maybe((!disabled).then_some(Message::PickBackgroundImage)),
                    App::btn("Clear")
                        .on_press_maybe((!disabled).then_some(Message::BackgroundClear)),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .spacing(8);

            if self.live.background_enabled || self.live.background.is_some() {
                col = col.push(
                    row![
                        text(format!("Zoom {:.1}", self.live.background_zoom)),
                        slider(
                            1.0..=100.0,
                            self.live.background_zoom,
                            Message::BackgroundZoomChanged
                        )
                        .width(Length::Fill),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                );
            }
            col
        });

        let cursor_card = Self::card({
            let mut col = column![
                Self::subsection_label("CURSOR"),
                toggler(self.fixed.cursor_composition)
                    .label("Cursor composition")
                    .on_toggle_maybe((!disabled).then_some(Message::CursorCompositionToggled)),
            ]
            .spacing(8);

            if self.fixed.cursor_composition {
                col = col
                    .push(
                        row![
                            text("Sprite"),
                            container(
                                text(Self::short_path(self.live.cursor_sprite.as_ref())).size(12)
                            )
                            .width(Length::Fill),
                            App::btn("Browse")
                                .on_press_maybe((!disabled).then_some(Message::PickCursorSprite)),
                            App::btn("Default")
                                .on_press_maybe((!disabled).then_some(Message::CursorSpriteClear)),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    )
                    .push(
                        row![
                            text("Hotspot"),
                            text_input("0", &self.fixed.cursor_hotspot_x)
                                .on_input_maybe(
                                    (!disabled).then_some(Message::CursorHotspotXEdited)
                                )
                                .width(Length::Fixed(70.0)),
                            text(","),
                            text_input("0", &self.fixed.cursor_hotspot_y)
                                .on_input_maybe(
                                    (!disabled).then_some(Message::CursorHotspotYEdited)
                                )
                                .width(Length::Fixed(70.0)),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    )
                    .push(
                        row![
                            text(format!("Scale {:.2}", self.live.cursor_scale)),
                            slider(
                                0.0..=100.0,
                                self.live.cursor_scale,
                                Message::CursorScaleChanged
                            )
                            .width(Length::Fill),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    )
                    .push(
                        row![
                            text("Wayland sync"),
                            text_input("0.5", &self.fixed.wayland_sync_frequency)
                                .on_input_maybe(
                                    (!disabled).then_some(Message::WaylandSyncFrequencyEdited)
                                )
                                .width(Length::Fixed(80.0)),
                            text("Hz").size(12).color(Color::from_rgb(0.5, 0.5, 0.5)),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    );
            }
            col
        });

        let mut controls = column![capture_card, encoding_card, cursor_card,].spacing(8);

        if self.fixed_dirty && !disabled {
            controls = controls.push(
                container(
                    row![
                        text("Settings changed")
                            .size(12)
                            .color(Color::from_rgb(0.9, 0.7, 0.2)),
                        App::btn("Restart Preview")
                            .style(button::success)
                            .on_press(Message::RestartPreview),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                )
                .padding([6, 10])
                .style(|theme: &Theme| {
                    let p = theme.extended_palette();
                    iced::widget::container::Style {
                        background: Some(
                            Color::from_rgba(
                                p.background.strong.color.r,
                                p.background.strong.color.g,
                                p.background.strong.color.b,
                                0.5,
                            )
                            .into(),
                        ),
                        border: iced::border::rounded(8),
                        ..Default::default()
                    }
                }),
            );
        }

        container(
            scrollable::Scrollable::with_direction(
                controls,
                scrollable::Direction::Vertical(Scrollbar::hidden()),
            )
            .auto_scroll(true),
        )
        .padding(8)
        .style(iced::widget::container::rounded_box)
        .height(Length::Fill)
        .width(Length::FillPortion(2))
        .max_width(560)
        .into()
    }

    pub(super) fn record_controls_panel(&self) -> Element<'_, Message> {
        let recording = matches!(self.mode, AppMode::Recording);
        let paused = self.paused;

        let action_card = Self::card(if recording {
            column![
                button(
                    container(text("Stop Recording").size(22).font(iced::Font {
                        weight: iced::font::Weight::Bold,
                        ..Default::default()
                    }),)
                    .center_x(Length::Fill),
                )
                .padding([14, 18])
                .width(Length::Fill)
                .style(|theme, status| {
                    let mut s = button::danger(theme, status);
                    s.border.radius = 10.0.into();
                    s
                })
                .on_press(Message::StopRecording),
                button(
                    container(
                        text(if paused {
                            "Resume Recording"
                        } else {
                            "Pause Recording"
                        })
                        .size(14)
                        .font(iced::Font {
                            weight: iced::font::Weight::Bold,
                            ..Default::default()
                        }),
                    )
                    .center_x(Length::Fill),
                )
                .padding([10, 16])
                .width(Length::Fill)
                .style(|theme, status| {
                    let mut s = button::secondary(theme, status);
                    s.border.radius = 10.0.into();
                    s
                })
                .on_press(Message::TogglePauseRecording),
            ]
            .spacing(8)
        } else {
            column![
                button(
                    container(text("Start Recording").size(24).font(iced::Font {
                        weight: iced::font::Weight::Bold,
                        ..Default::default()
                    }),)
                    .center_x(Length::Fill),
                )
                .padding([16, 20])
                .width(Length::Fill)
                .style(|theme, status| {
                    let mut s = button::primary(theme, status);
                    s.border.radius = 10.0.into();
                    s
                })
                .on_press(Message::StartRecording),
            ]
            .spacing(8)
        });

        let output_card = Self::card(
            column![
                Self::subsection_label("OUTPUT"),
                self.output_path_row(recording),
            ]
            .spacing(8),
        );

        let panel = column![action_card, output_card,].spacing(8);

        container(panel)
            .padding(8)
            .style(iced::widget::container::rounded_box)
            .height(Length::Fill)
            .width(Length::FillPortion(2))
            .max_width(640)
            .into()
    }

    // ── Advanced controls page ───────────────────────────────────────────
    pub(super) fn advanced_controls_panel(&self) -> Element<'_, Message> {
        let disabled = matches!(self.mode, crate::model::AppMode::Recording);

        // ── Capture advanced card ────────────────────────────────────────
        let capture_card = Self::card(
            column![
                Self::subsection_label("CAPTURE"),
                row![
                    text("Card"),
                    text_input("/dev/dri/card1", &self.fixed.card)
                        .on_input_maybe((!disabled).then_some(Message::CardEdited))
                        .width(Length::Fill),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                toggler(self.fixed.allow_fallback_connector)
                    .label("Allow fallback connector")
                    .on_toggle_maybe((!disabled).then_some(Message::AllowFallbackConnectorToggled)),
                row![
                    text("Output WxH"),
                    text_input("auto", &self.fixed.output_width)
                        .on_input_maybe((!disabled).then_some(Message::OutputWidthEdited))
                        .width(Length::Fixed(90.0)),
                    text("x"),
                    text_input("auto", &self.fixed.output_height)
                        .on_input_maybe((!disabled).then_some(Message::OutputHeightEdited))
                        .width(Length::Fixed(90.0)),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .spacing(8),
        );

        // ── Encoding advanced card ───────────────────────────────────────
        let encoding_card = Self::card(
            column![
                Self::subsection_label("ENCODING"),
                row![
                    text("Frame rate mode"),
                    pick_list(
                        &FrameRateModeChoice::ALL[..],
                        Some(self.fixed.frame_rate_mode),
                        Message::FrameRateModeChanged
                    )
                    .width(Length::Fixed(140.0)),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                row![
                    text("Rate control"),
                    pick_list(
                        &BitrateModeChoice::ALL[..],
                        Some(self.fixed.bitrate_mode),
                        Message::BitrateModeChanged
                    )
                    .width(Length::Fixed(140.0)),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                row![
                    text("Bitrate"),
                    text_input("15000", &self.fixed.bitrate_input)
                        .on_input_maybe((!disabled).then_some(Message::BitrateEdited))
                        .width(Length::Fixed(110.0)),
                    text("kbps"),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                row![
                    text("Colorimetry"),
                    pick_list(
                        &ColorimetryChoice::ALL[..],
                        Some(self.fixed.colorimetry),
                        Message::ColorimetryChanged
                    )
                    .width(Length::Fixed(140.0)),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .spacing(8),
        );

        // ── Debug card ───────────────────────────────────────────────────
        let debug_card = Self::card(
            column![
                Self::subsection_label("DEBUG"),
                toggler(self.fixed.dump_frames)
                    .label("Dump frames")
                    .on_toggle_maybe((!disabled).then_some(Message::DumpFramesToggled)),
                row![
                    text("Dump dir"),
                    container(text(Self::short_path(Some(&self.fixed.dump_dir))).size(13))
                        .width(Length::Fill),
                    App::btn("Browse").on_press_maybe((!disabled).then_some(Message::PickDumpDir)),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                row![
                    text("Dump every"),
                    text_input("30", &self.fixed.dump_every)
                        .on_input_maybe((!disabled).then_some(Message::DumpEveryEdited))
                        .width(Length::Fixed(90.0)),
                    text("frames"),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .spacing(8),
        );

        // ── Theme card ───────────────────────────────────────────────────
        let theme_card = Self::card(
            column![
                Self::subsection_label("THEME"),
                pick_list(get_all_themes(), self.theme.clone(), Message::ThemeChanged),
            ]
            .spacing(8),
        );

        // ── Cursor effects card (smooth) ─────────────────────────────────
        let smooth_card = Self::card({
            let mut col = column![
                Self::subsection_label("CURSOR SMOOTH"),
                toggler(self.live.cursor_smooth)
                    .label("Enable smooth cursor")
                    .on_toggle_maybe((!disabled).then_some(Message::CursorSmoothToggled)),
            ]
            .spacing(8);

            if self.live.cursor_smooth {
                col = col
                    .push(
                        row![
                            text("Spring K"),
                            slider(
                                1.0..=800.0,
                                self.live.cursor_spring_k,
                                Message::CursorSpringKChanged
                            )
                            .width(Length::Fill),
                            text(format!("{:.1}", self.live.cursor_spring_k))
                                .width(Length::Fixed(56.0)),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    )
                    .push(
                        row![
                            text("Spring D"),
                            slider(
                                1.0..=100.0,
                                self.live.cursor_spring_d,
                                Message::CursorSpringDChanged
                            )
                            .width(Length::Fill),
                            text(format!("{:.1}", self.live.cursor_spring_d))
                                .width(Length::Fixed(56.0)),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    )
                    .push(
                        row![
                            text("Max speed"),
                            slider(
                                100.0..=10000.0,
                                self.live.cursor_max_speed,
                                Message::CursorMaxSpeedChanged
                            )
                            .width(Length::Fill),
                            text(format!("{:.0}", self.live.cursor_max_speed))
                                .width(Length::Fixed(56.0)),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    )
                    .push(
                        row![
                            text("Snap px"),
                            slider(
                                0.0..=200.0,
                                self.live.cursor_snap_px,
                                Message::CursorSnapPxChanged
                            )
                            .width(Length::Fill),
                            text(format!("{:.1}", self.live.cursor_snap_px))
                                .width(Length::Fixed(56.0)),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    )
                    .push(
                        row![
                            text("Smooth ms"),
                            slider(
                                1.0..=60.0,
                                self.live.cursor_smooth_ms,
                                Message::CursorSmoothMsChanged
                            )
                            .width(Length::Fill),
                            text(format!("{:.1}", self.live.cursor_smooth_ms))
                                .width(Length::Fixed(56.0)),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    )
                    .push(
                        row![
                            text("Deadzone"),
                            slider(
                                0.0..=10.0,
                                self.live.cursor_deadzone_px,
                                Message::CursorDeadzonePxChanged
                            )
                            .width(Length::Fill),
                            text(format!("{:.2}", self.live.cursor_deadzone_px))
                                .width(Length::Fixed(56.0)),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    );
            }
            col
        });

        // ── Cursor effects card (smear) ──────────────────────────────────
        let smear_card = Self::card({
            let mut col = column![
                Self::subsection_label("CURSOR SMEAR"),
                toggler(self.live.cursor_smear)
                    .label("Enable smear effect")
                    .on_toggle_maybe((!disabled).then_some(Message::CursorSmearToggled)),
            ]
            .spacing(8);

            if self.live.cursor_smear {
                col = col
                    .push(
                        row![
                            text("Speed thr"),
                            slider(
                                0.0..=3000.0,
                                self.live.cursor_smear_speed_threshold,
                                Message::CursorSmearSpeedThresholdChanged
                            )
                            .width(Length::Fill),
                            text(format!("{:.0}", self.live.cursor_smear_speed_threshold))
                                .width(Length::Fixed(56.0)),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    )
                    .push(
                        row![
                            text("Shutter"),
                            slider(
                                0.1..=10.0,
                                self.live.cursor_smear_shutter_scale,
                                Message::CursorSmearShutterScaleChanged
                            )
                            .step(0.1)
                            .width(Length::Fill),
                            text(format!("{:.2}", self.live.cursor_smear_shutter_scale))
                                .width(Length::Fixed(56.0)),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    )
                    .push(
                        row![
                            text("Smear len"),
                            slider(
                                0.0..=400.0,
                                self.live.cursor_smear_min_len,
                                Message::CursorSmearMinLenChanged
                            )
                            .width(Length::Fill),
                            slider(
                                1.0..=500.0,
                                self.live.cursor_smear_max_len,
                                Message::CursorSmearMaxLenChanged
                            )
                            .width(Length::Fill),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    )
                    .push(
                        row![
                            text("Taps"),
                            slider(
                                1..=8,
                                self.live.cursor_smear_taps,
                                Message::CursorSmearTapsChanged
                            )
                            .width(Length::Fill),
                            text(format!("{}", self.live.cursor_smear_taps))
                                .width(Length::Fixed(36.0)),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    )
                    .push(
                        row![
                            text("Alpha exp"),
                            slider(
                                0.1..=4.0,
                                self.live.cursor_smear_alpha_exp,
                                Message::CursorSmearAlphaExpChanged
                            )
                            .step(0.1)
                            .width(Length::Fill),
                            text(format!("{:.2}", self.live.cursor_smear_alpha_exp))
                                .width(Length::Fixed(56.0)),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    )
                    .push(
                        row![
                            text("Alpha scale"),
                            slider(
                                0.01..=2.0,
                                self.live.cursor_smear_alpha_scale,
                                Message::CursorSmearAlphaScaleChanged
                            )
                            .step(0.01)
                            .width(Length::Fill),
                            text(format!("{:.2}", self.live.cursor_smear_alpha_scale))
                                .width(Length::Fixed(56.0)),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    )
                    .push(
                        row![
                            text("Stretch thr"),
                            slider(
                                0.0..=3000.0,
                                self.live.cursor_smear_stretch_threshold,
                                Message::CursorSmearStretchThresholdChanged
                            )
                            .width(Length::Fill),
                            text(format!("{:.0}", self.live.cursor_smear_stretch_threshold))
                                .width(Length::Fixed(56.0)),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    )
                    .push(
                        row![
                            text("Stretch rng"),
                            slider(
                                1.0..=4000.0,
                                self.live.cursor_smear_stretch_range,
                                Message::CursorSmearStretchRangeChanged
                            )
                            .width(Length::Fill),
                            text(format!("{:.0}", self.live.cursor_smear_stretch_range))
                                .width(Length::Fixed(56.0)),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    )
                    .push(
                        row![
                            text("Max stretch"),
                            slider(
                                0.0..=5.0,
                                self.live.cursor_smear_max_stretch,
                                Message::CursorSmearMaxStretchChanged
                            )
                            .width(Length::Fill),
                            text(format!("{:.2}", self.live.cursor_smear_max_stretch))
                                .width(Length::Fixed(56.0)),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    )
                    .push(
                        row![
                            text("Max squash"),
                            slider(
                                0.0..=0.95,
                                self.live.cursor_smear_max_squash,
                                Message::CursorSmearMaxSquashChanged
                            )
                            .step(0.01)
                            .width(Length::Fill),
                            text(format!("{:.2}", self.live.cursor_smear_max_squash))
                                .width(Length::Fixed(56.0)),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    );
            }
            col
        });

        let grid = column![
            Self::section_heading("Advanced Settings"),
            row![capture_card, encoding_card,]
                .spacing(10)
                .width(Length::Fill),
            row![debug_card, theme_card,]
                .spacing(10)
                .width(Length::Fill),
            Self::inset_divider(),
            Self::section_heading("Live Effects"),
            row![smooth_card, smear_card,]
                .spacing(10)
                .width(Length::Fill),
        ]
        .spacing(10);

        container(
            scrollable::Scrollable::with_direction(
                grid,
                scrollable::Direction::Vertical(Scrollbar::hidden()),
            )
            .auto_scroll(true),
        )
        .padding(12)
        .style(iced::widget::container::rounded_box)
        .height(Length::Fill)
        .width(Length::FillPortion(2))
        .into()
    }
}
