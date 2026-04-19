mod app;
mod model;
mod preview_shader;

use app::App;

fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title("Framepipe")
        .subscription(App::subscription)
        .run()
}
