//! Manifest shell boot (composition root).
//!
//! `main.rs` declares the feature modules and boots the Iced
//! application with the Manifest theme. All state, messages, and views
//! live in the modules below; nothing is built here.

mod app;
mod composer;
mod kit;
mod sessions;
mod theme;
mod transcript;

use app::App;
use theme::manifest_theme;

/// Boots the shell and asks the runtime for the latest window id so
/// the title-bar controls know which window they drive.
fn boot() -> (App, iced::Task<app::Message>) {
    (
        App::boot(),
        iced::window::latest().map(app::Message::WindowIdentified),
    )
}

fn main() -> iced::Result {
    iced::application(boot, App::update, App::view)
        .title("Manifest")
        .theme(manifest_theme())
        .window(iced::window::Settings {
            decorations: false,
            ..iced::window::Settings::default()
        })
        .run()
}
