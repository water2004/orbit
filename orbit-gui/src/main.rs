#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[macro_use]
extern crate orbit_i18n;
use tauri::Manager;

mod desktop;
mod diagnostics;
mod model;
mod process;
mod string_rule;
mod wire;

fn main() {
    diagnostics::install_panic_reporter();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(desktop::setup)
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                desktop::shutdown(window.app_handle());
            }
        })
        .invoke_handler(tauri::generate_handler![
            desktop::open_directory,
            desktop::preferences,
            desktop::save_preferences,
            desktop::run_cli,
            desktop::cancel_cli,
            desktop::answer_cli,
            desktop::parse_string_rule,
            desktop::render_string_rule
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Orbit desktop");
}
