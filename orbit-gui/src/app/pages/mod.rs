use gpui::{AnyElement, Context, IntoElement, Window};

use super::OrbitApp;
use crate::model::Page;

mod accounts;
pub(super) use accounts::{initials as account_initials, provider_label as account_provider_label};
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
    let (transition, content) = match app.preferences.page {
        Page::Home => (
            "page-home",
            home::render(app, window, cx).into_any_element(),
        ),
        Page::Library => (
            "page-library",
            library::render(app, window, cx).into_any_element(),
        ),
        Page::Discover => (
            "page-discover",
            discover::render(app, window, cx).into_any_element(),
        ),
        Page::Audit => (
            "page-audit",
            audit::render(app, window, cx).into_any_element(),
        ),
        Page::Runtime => (
            "page-runtime",
            runtime::render(app, window, cx).into_any_element(),
        ),
        Page::Accounts => (
            "page-accounts",
            accounts::render(app, window, cx).into_any_element(),
        ),
        Page::Server => (
            "page-server",
            server::render(app, window, cx).into_any_element(),
        ),
        Page::Settings => (
            "page-settings",
            settings::render(app, window, cx).into_any_element(),
        ),
    };
    crate::app::components::reveal(transition, content)
}
