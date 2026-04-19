use iced::widget::shader::Shader as ShaderWidget;
use iced::widget::{button, column, container, row, text};
use iced::{Alignment, Color, Element, Length, Theme};

use crate::app::{App, Message};

impl App {
    fn preview_panel(&self) -> Element<'_, Message> {
        if matches!(self.mode, crate::model::AppMode::Recording) {
            return container(
                column![
                    text("Recording…").size(28),
                    text(self.recording_elapsed()).size(34),
                    text("Preview disabled while recording"),
                ]
                .align_x(Alignment::Center)
                .spacing(8),
            )
            .width(Length::FillPortion(65))
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(iced::widget::container::rounded_box)
            .into();
        }

        if !matches!(self.mode, crate::model::AppMode::Preview) {
            return container(
                column![
                    text("Preview Disabled").size(24),
                    text("Enable preview to inspect capture settings in realtime"),
                    button("Enable Preview").on_press(Message::TogglePreview),
                ]
                .align_x(Alignment::Center)
                .spacing(8),
            )
            .width(Length::FillPortion(65))
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(iced::widget::container::rounded_box)
            .into();
        }

        let has_frame = self
            .preview_mailbox
            .as_ref()
            .and_then(|mb| mb.get_frame())
            .is_some();

        if !has_frame {
            return container(
                column![text("Preview Running").size(24), text("Waiting for first frame…")]
                    .align_x(Alignment::Center)
                    .spacing(8),
            )
            .width(Length::FillPortion(65))
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
            .width(Length::FillPortion(65))
            .height(Length::Fill)
            .style(iced::widget::container::rounded_box)
            .into();
        }

        container(text("Preview unavailable"))
            .width(Length::FillPortion(65))
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(iced::widget::container::rounded_box)
            .into()
    }

    pub fn view(&self) -> Element<'_, Message> {
        container(
            column![
                text("Framepipe").size(30),
                text(&self.status).size(14),
                row![self.controls_panel(), self.preview_panel()]
                    .spacing(12)
                    .width(Length::Fill)
                    .height(Length::Fill),
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

