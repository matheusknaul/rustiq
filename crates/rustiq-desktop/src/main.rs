mod app;
mod message;
mod theme;
mod ui;
mod workspace;

use app::App;
use theme::rustiq_theme;

fn main() -> iced::Result {
    iced::application(App::default, App::update, App::view)
        .theme(|_: &App| rustiq_theme())
        .title("RustiQ • API Workspace")
        .font(include_bytes!("../assets/fonts/DejaVuSansMono.ttf").as_slice())
        .font(include_bytes!("../assets/fonts/fa-solid-900.ttf").as_slice())
        .default_font(theme::MONO)
        .window(iced::window::Settings {
            size: iced::Size::new(1280., 820.),
            min_size: Some(iced::Size::new(1000., 640.)),
            ..Default::default()
        })
        .run()
}
