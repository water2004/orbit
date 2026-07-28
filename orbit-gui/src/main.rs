#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[macro_use]
extern crate orbit_i18n;

mod app;
mod assets;
mod http_images;
mod model;
mod process;
mod theme;
mod wire;

use gpui::{
    App, AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size,
};
use gpui_component::Root;
use std::sync::Arc;

fn main() {
    Application::new()
        .with_http_client(Arc::new(
            http_images::ImageHttpClient::new().expect("failed to initialize the image client"),
        ))
        .with_assets(assets::OrbitAssets)
        .run(|cx: &mut App| {
            gpui_component::init(cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1220.), px(780.)),
                    cx,
                ))),
                window_min_size: Some(size(px(940.), px(620.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Orbit".into()),
                    ..Default::default()
                }),
                app_id: Some("dev.orbit.gui".to_string()),
                ..Default::default()
            };

            cx.open_window(options, |window, cx| {
                let app = cx.new(|cx| app::OrbitApp::new(window, cx));
                cx.new(|cx| Root::new(app, window, cx))
            })
            .expect("failed to open Orbit window");
            cx.activate(true);
        });
}
