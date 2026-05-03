use iced::widget::shader::Shader as ShaderWidget;
use iced::widget::{button, column, container, image, row, stack, text};
use iced::{Alignment, Color, Element, Length, Theme};

use crate::app::{App, Message};
use crate::model::UiPage;

impl App {
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
                        text("Recording…").size(28),
                        text(self.recording_elapsed()).size(34),
                        text("Preview disabled while recording"),
                    ]
                    .align_x(Alignment::Center)
                    .spacing(8),
                )
                .width(Length::FillPortion(4))
                .height(Length::Fill)
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .style(iced::widget::container::rounded_box)
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
                            text("Preview Disabled")
                                .size(24)
                                .color(Color::from_rgb(0.8, 0.8, 0.8))
                                .font(iced::Font {
                                    weight: iced::font::Weight::Bold,
                                    ..Default::default()
                                })
                        )
                        .padding(12)
                        .style(|_| iced::widget::container::Style {
                            background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.6).into()),
                            border: iced::border::rounded(8),
                            ..Default::default()
                        }),
                        text("Enable preview to inspect capture settings in realtime")
                            .color(Color::from_rgb(0.7, 0.7, 0.7)),
                        App::btn("Enable Preview").on_press(Message::TogglePreview),
                    ]
                    .align_x(Alignment::Center)
                    .spacing(8),
                )
                .padding(8)
                .width(Length::FillPortion(3))
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
            return container(
                column![
                    text("Preview Running").size(24),
                    text("Waiting for first frame…")
                ]
                .align_x(Alignment::Center)
                .spacing(8),
            )
            .width(Length::FillPortion(3))
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(iced::widget::container::rounded_box)
            .into();
        }

        if let Some(program) = &self.preview_program {
            return container(
                ShaderWidget::new(program.clone())
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .width(Length::FillPortion(4))
            .height(Length::Fill)
            .style(iced::widget::container::rounded_box)
            .into();
        }

        container(text("Preview unavailable"))
            .width(Length::FillPortion(4))
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(iced::widget::container::rounded_box)
            .into()
    }

    pub fn view(&self) -> Element<'_, Message> {
        let preview_label = if matches!(self.mode, crate::model::AppMode::Preview) {
            "Disable Preview"
        } else {
            "Enable Preview"
        };
        let nav = match self.page {
            UiPage::Configure => row![
                App::btn(preview_label)
                    .style(button::secondary)
                    .on_press(Message::TogglePreview),
                App::btn("Record")
                    .style(button::primary)
                    .on_press(Message::GoToRecordPage),
            ]
            .spacing(8),
            UiPage::Record => row![
                App::btn("Back").style(button::secondary).on_press_maybe(
                    (!matches!(self.mode, crate::model::AppMode::Recording))
                        .then_some(Message::GoToConfigurePage)
                ),
                if matches!(self.mode, crate::model::AppMode::Recording) {
                    App::btn(if self.paused {
                        "Resume Recording"
                    } else {
                        "Pause Recording"
                    })
                    .style(button::secondary)
                    .on_press(Message::TogglePauseRecording)
                } else {
                    App::btn("Start Recording")
                        .style(button::primary)
                        .on_press(Message::StartRecording)
                },
            ]
            .spacing(8),
        };

        let content: Element<'_, Message> = match self.page {
            UiPage::Configure => row![self.configure_controls_panel(), self.preview_panel()]
                .spacing(12)
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
            UiPage::Record => row![self.record_controls_panel()]
                .spacing(12)
                .width(Length::Fill)
                .height(Length::Fill)
                .into(),
        };

        container(
            column![
                row![
                    text("Framepipe").size(30),
                    text(" • ").size(30),
                    text(&self.status)
                        .size(18)
                        .color(Color::from_rgb(0.7, 0.7, 0.7)),
                ]
                .align_y(Alignment::Center),
                nav,
                content,
            ]
            .spacing(10)
            .padding(12),
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
