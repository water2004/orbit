use gpui::{AnyElement, Context, IntoElement, Window};

use super::OrbitApp;
use crate::model::Page;

mod accounts;
pub(super) mod activity;
mod audit;
mod discover;
mod home;
mod library;
mod runtime;
mod server;
mod settings;

pub(super) fn render(
    app: &mut OrbitApp,
    window: &mut Window,
    cx: &mut Context<OrbitApp>,
) -> AnyElement {
    match app.preferences.page {
        Page::Home => home::render(app, window, cx).into_any_element(),
        Page::Library => library::render(app, window, cx).into_any_element(),
        Page::Discover => discover::render(app, window, cx).into_any_element(),
        Page::Audit => audit::render(app, window, cx).into_any_element(),
        Page::Runtime => runtime::render(app, window, cx).into_any_element(),
        Page::Accounts => accounts::render(app, window, cx).into_any_element(),
        Page::Server => server::render(app, window, cx).into_any_element(),
        Page::Settings => settings::render(app, window, cx).into_any_element(),
    }
}
