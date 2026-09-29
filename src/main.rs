#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod model;
mod ssh;
mod term;

fn main() -> iced::Result {
    app::run()
}
