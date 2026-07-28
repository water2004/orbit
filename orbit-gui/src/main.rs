#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[macro_use]
extern crate orbit_i18n;

mod app;
mod icon;
mod model;
mod process;
mod theme;
mod wire;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Orbit")
            .with_icon(icon::app_icon())
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([960.0, 640.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Orbit",
        options,
        Box::new(|creation| {
            egui_extras::install_image_loaders(&creation.egui_ctx);
            Ok(Box::new(app::OrbitApp::new(creation)))
        }),
    )
}
