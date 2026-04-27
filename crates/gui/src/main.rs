mod app;
mod model;
mod preview_shader;
mod theme;

use app::App;
use iced::window::icon;

fn main() -> iced::Result {
    let icon = image::load_from_memory_with_format(
        include_bytes!("../../../assets/icons/icon-512.png"),
        image::ImageFormat::Png,
    );
    iced::application(App::new, App::update, App::view)
        .window(iced::window::Settings {
            size: iced::Size {
                width: 1200.0,
                height: 600.0,
            },
            resizable: true,
            icon: Some(
                icon::from_rgba(icon.unwrap().to_rgba8().into_raw(), 512, 512).expect("valid icon"),
            ),
            ..Default::default()
        })
        .theme(App::theme)
        .title("Framepipe")
        .subscription(App::subscription)
        .run()
}
