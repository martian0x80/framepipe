use iced::widget::shader::Shader as ShaderWidget;
use iced::widget::{button, column, container, image, row, stack, text};
use iced::{Alignment, Color, Element, Length, Theme};

use crate::app::{App, Message};
use crate::model::UiPage;

impl App {
    fn nav_tab<'a>(label: &'a str, active: bool, msg: Option<Message>) -> Element<'a, Message> {
        let label_widget = text(label).size(13).font(iced::Font {
            weight: if active {
                iced::font::Weight::Bold
            } else {
                iced::font::Weight::Normal
            },
            ..Default::default()
        });

        let btn = button(container(label_widget).center_x(Length::Fill))
            .width(Length::Fill)
            .height(Length::Shrink)
            .padding([6, 14])
            .style(move |theme: &Theme, status| {
                let palette = theme.extended_palette();
                let mut style = if active {
                    let mut s = iced::widget::button::primary(theme, status);
                    s.background = Some(iced::Background::Color(palette.primary.strong.color));
                    s.text_color = Color::WHITE;
                    s
                } else {
                    let mut s = iced::widget::button::secondary(theme, status);
                    s.background = Some(iced::Background::Color(Color::TRANSPARENT));
                    s.text_color = Color::from_rgba(
                        palette.background.base.text.r,
                        palette.background.base.text.g,
                        palette.background.base.text.b,
                        0.55,
                    );
                    s
                };
                style.border.radius = 8.0.into();
                style
            });

        if let Some(m) = msg {
            btn.on_press(m).into()
        } else {
            btn.into()
        }
    }

    fn navbar(&self) -> Element<'_, Message> {
        let is_recording = matches!(self.mode, crate::model::AppMode::Recording);
        let current = self.page;

        let configure_msg = if is_recording {
            None
        } else {
            Some(Message::GoToConfigurePage)
        };
        let record_msg = Some(Message::GoToRecordPage);
        let advanced_msg = if is_recording {
            None
        } else {
            Some(Message::GoToAdvancedPage)
        };

        let nav_row = row![
            Self::nav_tab("Configure", current == UiPage::Configure, configure_msg),
            Self::nav_tab("Record", current == UiPage::Record, record_msg),
            Self::nav_tab("Advanced", current == UiPage::Advanced, advanced_msg),
        ]
        .spacing(2)
        .width(Length::Fill);

        container(nav_row)
            .padding([3, 3])
            .width(Length::Fill)
            .height(Length::Shrink)
            .style(|theme: &Theme| {
                let palette = theme.extended_palette();
                iced::widget::container::Style {
                    background: Some(
                        Color::from_rgba(
                            palette.background.weak.color.r,
                            palette.background.weak.color.g,
                            palette.background.weak.color.b,
                            0.4,
                        )
                        .into(),
                    ),
                    border: iced::border::rounded(10),
                    ..Default::default()
                }
            })
            .into()
    }

    fn preview_overlay_button(&self) -> Element<'_, Message> {
        let is_preview = matches!(self.mode, crate::model::AppMode::Preview);

        if !is_preview {
            return container("").width(0).height(0).into();
        }

        let btn = button(
            text(if self.fixed_dirty {
                "Restart"
            } else {
                "Stop Preview"
            })
            .size(11)
            .font(iced::Font {
                weight: iced::font::Weight::Bold,
                ..Default::default()
            }),
        )
        .padding([4, 10])
        .on_press(if self.fixed_dirty {
            Message::RestartPreview
        } else {
            Message::TogglePreview
        })
        .style(move |theme: &Theme, status| {
            let mut s = if is_preview {
                iced::widget::button::danger(theme, status)
            } else {
                iced::widget::button::success(theme, status)
            };
            s.background = Some(iced::Background::Color(Color::from_rgba(
                0.15, 0.15, 0.15, 0.85,
            )));
            s.text_color = Color::from_rgb(0.9, 0.9, 0.9);
            s.border.radius = 6.0.into();
            s
        });

        container(btn)
            .width(Length::Fill)
            .align_x(Alignment::End)
            .padding([6, 8])
            .into()
    }

    fn preview_panel(&self) -> Element<'_, Message> {
        if matches!(self.mode, crate::model::AppMode::Recording) {
            return stack![
                image::Image::new(self.background_cache.as_ref().unwrap_or(
                    &iced::widget::image::Handle::from_path("assets/grainy_bg.png")
                ))
                .width(Length::Fill)
                .height(Length::Fill)
                .content_fit(iced::ContentFit::Cover),
                container(
                    column![
                        container(text("REC").size(14).color(Color::WHITE).font(iced::Font {
                            weight: iced::font::Weight::Bold,
                            ..Default::default()
                        }))
                        .padding([4, 12])
                        .style(|_: &Theme| {
                            iced::widget::container::Style {
                                background: Some(Color::from_rgb(0.85, 0.15, 0.15).into()),
                                border: iced::border::rounded(6),
                                ..Default::default()
                            }
                        }),
                        text(self.recording_elapsed()).size(40).font(iced::Font {
                            weight: iced::font::Weight::Bold,
                            ..Default::default()
                        }),
                        text("Preview disabled while recording")
                            .size(13)
                            .color(Color::from_rgb(0.6, 0.6, 0.6)),
                    ]
                    .align_x(Alignment::Center)
                    .spacing(10),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .style(|_: &Theme| iced::widget::container::Style {
                    background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.5).into()),
                    border: iced::border::rounded(12),
                    ..Default::default()
                })
            ]
            .into();
        }

        if !matches!(self.mode, crate::model::AppMode::Preview) {
            return stack![
                image::Image::new(self.background_cache.as_ref().unwrap_or(
                    &iced::widget::image::Handle::from_path("assets/grainy_bg.png")
                ))
                .width(Length::Fill)
                .height(Length::Fill)
                .content_fit(iced::ContentFit::Cover)
                .border_radius(32),
                container(
                    column![
                        container(
                            text("No Preview")
                                .size(18)
                                .color(Color::from_rgb(0.75, 0.75, 0.75))
                                .font(iced::Font {
                                    weight: iced::font::Weight::Bold,
                                    ..Default::default()
                                })
                        )
                        .padding([8, 16])
                        .style(|_: &Theme| {
                            iced::widget::container::Style {
                                background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.55).into()),
                                border: iced::border::rounded(8),
                                ..Default::default()
                            }
                        }),
                        text("Start preview to inspect settings live")
                            .size(12)
                            .color(Color::from_rgb(0.55, 0.55, 0.55)),
                        button(text("Enable Preview").size(13).font(iced::Font {
                            weight: iced::font::Weight::Bold,
                            ..Default::default()
                        }))
                        .padding([6, 16])
                        .style(|theme: &Theme, status| {
                            let mut s = iced::widget::button::success(theme, status);
                            s.border.radius = 8.0.into();
                            s
                        })
                        .on_press(Message::TogglePreview),
                    ]
                    .align_x(Alignment::Center)
                    .spacing(8),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .style(iced::widget::container::transparent)
            ]
            .into();
        }

        let has_frame = self
            .preview_mailbox
            .as_ref()
            .and_then(|mb| mb.get_frame())
            .is_some();

        if !has_frame {
            return stack![
                container("")
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .style(|theme: &Theme| {
                        let p = theme.extended_palette();
                        iced::widget::container::Style {
                            background: Some(
                                Color::from_rgba(
                                    p.background.strong.color.r,
                                    p.background.strong.color.g,
                                    p.background.strong.color.b,
                                    0.4,
                                )
                                .into(),
                            ),
                            border: iced::border::rounded(12),
                            ..Default::default()
                        }
                    }),
                container(
                    column![
                        text("Preview Running").size(18).font(iced::Font {
                            weight: iced::font::Weight::Bold,
                            ..Default::default()
                        }),
                        text("Waiting for first frame…")
                            .size(12)
                            .color(Color::from_rgb(0.55, 0.55, 0.55)),
                    ]
                    .align_x(Alignment::Center)
                    .spacing(6),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x(Length::Fill)
                .center_y(Length::Fill),
                self.preview_overlay_button(),
            ]
            .into();
        }

        if let Some(program) = &self.preview_program {
            return stack![
                container(
                    ShaderWidget::new(program.clone())
                        .width(Length::Fill)
                        .height(Length::Fill),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .style(|_: &Theme| iced::widget::container::Style {
                    border: iced::border::rounded(12),
                    ..Default::default()
                }),
                self.preview_overlay_button(),
            ]
            .into();
        }

        container(
            text("Preview unavailable")
                .size(13)
                .color(Color::from_rgb(0.5, 0.5, 0.5)),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(iced::widget::container::rounded_box)
        .into()
    }

    fn header_bar(&self) -> Element<'_, Message> {
        let title = text("Framepipe").size(22).font(iced::Font {
            weight: iced::font::Weight::Bold,
            ..Default::default()
        });

        let status = text(&self.status)
            .size(12)
            .color(Color::from_rgb(0.45, 0.45, 0.45));

        let mode_pill = {
            let (label, color) = match self.mode {
                crate::model::AppMode::Idle => ("IDLE", Color::from_rgb(0.35, 0.35, 0.35)),
                crate::model::AppMode::Preview => ("LIVE", Color::from_rgb(0.15, 0.6, 0.35)),
                crate::model::AppMode::Recording => ("REC", Color::from_rgb(0.85, 0.15, 0.15)),
            };
            container(text(label).size(10).color(Color::WHITE).font(iced::Font {
                weight: iced::font::Weight::Bold,
                ..Default::default()
            }))
            .padding([2, 8])
            .style(move |_: &Theme| iced::widget::container::Style {
                background: Some(color.into()),
                border: iced::border::rounded(5),
                ..Default::default()
            })
        };

        row![
            title,
            mode_pill,
            iced::widget::Space::new().width(Length::Fill),
            status
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
    }

    pub fn view(&self) -> Element<'_, Message> {
        let content: Element<'_, Message> = match self.page {
            UiPage::Configure => row![self.configure_controls_panel(), self.preview_panel()]
                .spacing(10)
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
            UiPage::Record => row![self.record_controls_panel(), self.preview_panel()]
                .spacing(10)
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
            UiPage::Advanced => row![self.advanced_controls_panel(), self.preview_panel()]
                .spacing(10)
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
        };

        container(
            column![self.header_bar(), self.navbar(), content,]
                .spacing(6)
                .padding(10),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|theme: &Theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(theme.palette().background)),
            text_color: Some(theme.palette().text),
            ..iced::widget::container::Style::default()
        })
        .into()
    }
}
