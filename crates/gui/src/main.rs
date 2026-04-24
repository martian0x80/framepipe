mod app;
mod model;
mod preview_shader;
mod theme;

use app::App;

fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .window(iced::window::Settings {
            size: iced::Size {
                width: 1200.0,
                height: 600.0,
            },
            resizable: true,
            ..Default::default()
        })
        .theme(App::theme)
        .title("Framepipe")
        .subscription(App::subscription)
        .run()
}
