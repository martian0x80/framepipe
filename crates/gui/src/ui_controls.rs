use crate::app::{App, Message};
use crate::model::{
    BitrateModeChoice, CodecChoice, ColorRangeChoice, ColorimetryChoice, EncoderChoice,
    FrameRateModeChoice, ProfileChoice, QualityChoice, SourceChoice,
};
use crate::theme::get_all_themes;
use iced::widget::scrollable::Scrollbar;
use iced::widget::{
    button, column, container, pick_list, row, scrollable, slider, text, text_input, toggler,
};
use iced::{Alignment, Element, Length};

impl App {
    fn section_heading(title: &str) -> iced::widget::Text<'_> {
        text(title).size(22)
    }

    fn inset_divider() -> Element<'static, Message> {
        container(
            container("")
                .height(Length::Fixed(2.0))
                .width(Length::Fill)
                .style(|theme: &iced::Theme| {
                    let p = theme.extended_palette();
                    container::Style {
                        background: Some(p.secondary.strong.color.into()),
                        ..Default::default()
                    }
                }),
        )
        .padding([4, 6])
        .into()
    }

    fn action_buttons(&self) -> Element<'_, Message> {
        if matches!(self.mode, crate::model::AppMode::Recording) {
            let pause_label = if self.paused {
                "Resume Recording"
            } else {
                "Pause Recording"
            };
            return column![
                App::btn(text("Stop Recording").size(26))
                    .padding([16, 18])
                    .width(Length::Fill)
                    .style(button::danger)
                    .on_press(Message::StopRecording),
                App::btn(text(pause_label).size(18))
                    .padding([10, 14])
                    .width(Length::Fill)
                    .style(button::secondary)
                    .on_press(Message::TogglePauseRecording),
                text(format!("Recording {}", self.recording_elapsed())).size(18),
            ]
            .spacing(10)
            .into();
        }

        let preview_label = if matches!(self.mode, crate::model::AppMode::Preview) {
            "Disable Preview"
        } else {
            "Enable Preview"
        };

        let mut col = column![
            App::btn(text("Record").size(30))
                .padding([18, 20])
                .width(Length::Fill)
                .style(button::primary)
                .on_press(Message::StartRecording),
            App::btn(text(preview_label).size(18))
                .padding([12, 16])
                .width(Length::Fill)
                .style(button::secondary)
                .on_press(Message::TogglePreview),
        ]
        .spacing(8);

        if matches!(self.mode, crate::model::AppMode::Preview) {
            let pause_label = if self.paused {
                "Resume Preview"
            } else {
                "Pause Preview"
            };
            col = col.push(
                App::btn(text(pause_label).size(16))
                    .padding([8, 12])
                    .width(Length::Fill)
                    .style(button::secondary)
                    .on_press(Message::TogglePausePreview),
            );
        }

        col.into()
    }

    pub(super) fn controls_panel(&self) -> Element<'_, Message> {
        let disabled = matches!(self.mode, crate::model::AppMode::Recording);

        let mut controls = column![
            self.action_buttons(),
            Self::section_heading("Capture"),
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
                text(format!("Preview FPS {}", self.live.fps)),
                slider(1..=240, self.live.fps, Message::FpsChanged).width(Length::Fill),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            Self::inset_divider(),
            Self::section_heading("Encoding"),
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
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            row![
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
            row![
                text("Output"),
                container(
                    text(match &self.fixed.output_path {
                        Some(path) => Self::short_path(Some(path)),
                        None => {
                            match dirs::video_dir() {
                                Some(v) => format!("{}/framepipe_record_<time>.mp4", v.display()),
                                None => "framepipe_record_<time>.mp4".to_string(),
                            }
                        }
                    })
                    .size(13)
                )
                .width(Length::Fill),
                App::btn("Browse").on_press_maybe((!disabled).then_some(Message::PickOutputPath)),
                App::btn("Clear").on_press_maybe((!disabled).then_some(Message::OutputPathCleared)),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            row![
                toggler(self.live.background_enabled)
                    .label("Background")
                    .on_toggle_maybe((!disabled).then_some(Message::BackgroundToggled)),
                container(text(Self::short_path(self.live.background.as_ref())).size(13))
                    .width(Length::Fill),
                App::btn("Browse")
                    .on_press_maybe((!disabled).then_some(Message::PickBackgroundImage)),
                App::btn("Clear").on_press_maybe((!disabled).then_some(Message::BackgroundClear)),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        ]
        .spacing(10);

        // todo: insert below background toggle
        if self.live.background_enabled || self.live.background.is_some() {
            controls = controls.push(
                row![
                    text(format!("Background Zoom {:.2}", self.live.background_zoom)),
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

        controls = controls.push(Self::inset_divider());
        controls = controls.push(Self::section_heading("Cursor"));
        controls = controls.push(
            row![
                toggler(self.fixed.cursor_composition)
                    .label("Cursor composition")
                    .on_toggle_maybe((!disabled).then_some(Message::CursorCompositionToggled)),
            ]
            .spacing(8),
        );

        if self.fixed.cursor_composition {
            controls = controls.push(
                row![
                    text("Cursor Sprite"),
                    container(text(Self::short_path(self.live.cursor_sprite.as_ref())).size(13))
                        .width(Length::Fill),
                    App::btn("Browse")
                        .on_press_maybe((!disabled).then_some(Message::PickCursorSprite)),
                    App::btn("Default")
                        .on_press_maybe((!disabled).then_some(Message::CursorSpriteClear)),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
            controls = controls.push(
                row![
                    text("Hotspot"),
                    text_input("0", &self.fixed.cursor_hotspot_x)
                        .on_input_maybe((!disabled).then_some(Message::CursorHotspotXEdited))
                        .width(Length::Fixed(70.0)),
                    text(","),
                    text_input("0", &self.fixed.cursor_hotspot_y)
                        .on_input_maybe((!disabled).then_some(Message::CursorHotspotYEdited))
                        .width(Length::Fixed(70.0)),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
            controls = controls.push(
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
            );
            controls = controls.push(
                row![
                    text("Wayland sync (Hz)"),
                    text_input("0.5", &self.fixed.wayland_sync_frequency)
                        .on_input_maybe((!disabled).then_some(Message::WaylandSyncFrequencyEdited))
                        .width(Length::Fixed(90.0)),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
            controls = controls.push(Self::inset_divider());
            controls = controls.push(Self::section_heading("Live Effects"));
            controls = controls.push(
                row![
                    toggler(self.live.cursor_smooth)
                        .label("Cursor Smooth")
                        .on_toggle_maybe((!disabled).then_some(Message::CursorSmoothToggled)),
                    toggler(self.live.cursor_smear)
                        .label("Cursor Smear")
                        .on_toggle_maybe((!disabled).then_some(Message::CursorSmearToggled)),
                ]
                .spacing(8),
            );
            if self.live.cursor_smooth {
                controls = controls.push(
                    column![
                        row![
                            text("Smooth K"),
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
                        row![
                            text("Smooth D"),
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
                    ]
                    .spacing(8),
                );
            }

            if self.live.cursor_smear {
                controls = controls.push(
                    column![
                        row![
                            text("Smear speed"),
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
                        row![
                            text("Smear shutter"),
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
                        row![
                            text("Smear taps"),
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
                        row![
                            text("Stretch range"),
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
                    ]
                    .spacing(8),
                );
            }
        }

        if self.fixed_dirty && !disabled {
            controls = controls.push(
                row![
                    text("Fixed settings changed"),
                    App::btn("Restart Preview")
                        .style(button::success)
                        .on_press(Message::RestartPreview),
                ]
                .spacing(8),
            );
        }

        controls = controls.push(Self::inset_divider());
        controls = controls.push(
            App::btn(if self.show_advanced {
                "Hide Advanced"
            } else {
                "Show Advanced"
            })
            .on_press_maybe((!disabled).then_some(Message::ToggleAdvanced)),
        );

        if self.show_advanced {
            let mut advanced = column![
                Self::section_heading("Capture Advanced"),
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
                Self::inset_divider(),
                Self::section_heading("Debug"),
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
                Self::inset_divider(),
                Self::section_heading("Theme"),
                pick_list(get_all_themes(), self.theme.clone(), Message::ThemeChanged)
            ]
            .spacing(8);

            controls = controls.push(advanced);
        }

        container(
            scrollable::Scrollable::with_direction(
                controls,
                scrollable::Direction::Vertical(Scrollbar::hidden()),
            )
            .auto_scroll(true),
        )
        .padding(12)
        .style(iced::widget::container::rounded_box)
        .height(Length::Fill)
        .width(Length::FillPortion(2))
        .max_width(560)
        .into()
    }
}
