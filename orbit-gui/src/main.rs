#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[macro_use]
extern crate orbit_i18n;
use tauri::Manager;

mod desktop;
mod diagnostics;
mod model;
mod preferences;
mod process;
mod string_rule;
mod wire;

fn main() {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--install-cli-bindings"))
    {
        if std::env::args_os().count() != 2 {
            std::process::exit(2);
        }
        if let Err(error) = preferences::install_cli_bindings() {
            eprintln!("Cannot update installed GUI CLI bindings: {error}");
            std::process::exit(1);
        }
        return;
    }
    diagnostics::install_panic_reporter();
    if let Err(error) = preferences::refresh_installation() {
        eprintln!("Cannot update installed GUI CLI bindings: {error}");
        std::process::exit(1);
    }
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
