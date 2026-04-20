mod app;
mod model;
mod theme;
mod preview_shader;

use app::App;
use crate::theme::ThemeKind;

fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .theme(App::theme)
        .title("Framepipe")
        .subscription(App::subscription)
        .run()
}
