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

fn main() -> iced::Result {
    iced::application(App::default, App::update, App::view)
        .title("Manifest")
        .theme(manifest_theme())
        .run()
}
