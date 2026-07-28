use std::borrow::Cow;

use anyhow::Result;
use gpui::{AssetSource, SharedString};
use gpui_component::IconNamed;

pub struct OrbitAssets;

impl AssetSource for OrbitAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let bytes: &'static [u8] = match path {
            "icons/activity.svg" => include_bytes!("../assets/icons/activity.svg"),
            "icons/account.svg" | "icons/user.svg" | "icons/circle-user.svg" => {
                include_bytes!("../assets/icons/account.svg")
            }
            "icons/audit.svg" | "icons/check.svg" | "icons/circle-check.svg" => {
                include_bytes!("../assets/icons/audit.svg")
            }
            "icons/browse.svg" | "icons/globe.svg" => {
                include_bytes!("../assets/icons/browse.svg")
            }
            "icons/chevron-down.svg" | "icons/chevrons-up-down.svg" => {
                include_bytes!("../assets/icons/chevron-down.svg")
            }
            "icons/close.svg" | "icons/circle-x.svg" | "icons/window-close.svg" => {
                include_bytes!("../assets/icons/close.svg")
            }
            "icons/download.svg" | "icons/arrow-down.svg" => {
                include_bytes!("../assets/icons/download.svg")
            }
            "icons/folder.svg" | "icons/folder-closed.svg" | "icons/folder-open.svg" => {
                include_bytes!("../assets/icons/folder.svg")
            }
            "icons/home.svg" | "icons/layout-dashboard.svg" => {
                include_bytes!("../assets/icons/home.svg")
            }
            "icons/inbox.svg" => include_bytes!("../assets/icons/inbox.svg"),
            "icons/java.svg" => include_bytes!("../assets/icons/java.svg"),
            "icons/loader.svg" | "icons/loader-circle.svg" => {
                include_bytes!("../assets/icons/loader.svg")
            }
            "icons/mods.svg" | "icons/gallery-vertical-end.svg" => {
                include_bytes!("../assets/icons/mods.svg")
            }
            "icons/orbit.svg" => include_bytes!("../../assets/orbit.svg"),
            "icons/play.svg" => include_bytes!("../assets/icons/play.svg"),
            "icons/plus.svg" => include_bytes!("../assets/icons/plus.svg"),
            "icons/refresh.svg" | "icons/redo.svg" | "icons/redo-2.svg" => {
                include_bytes!("../assets/icons/refresh.svg")
            }
            "icons/runtime.svg" | "icons/file.svg" => {
                include_bytes!("../assets/icons/runtime.svg")
            }
            "icons/search.svg" => include_bytes!("../assets/icons/search.svg"),
            "icons/server.svg" => include_bytes!("../assets/icons/server.svg"),
            "icons/settings.svg" | "icons/settings-2.svg" => {
                include_bytes!("../assets/icons/settings.svg")
            }
            "icons/terminal.svg" | "icons/square-terminal.svg" => {
                include_bytes!("../assets/icons/terminal.svg")
            }
            "icons/trash.svg" | "icons/delete.svg" => {
                include_bytes!("../assets/icons/trash.svg")
            }
            "icons/warning.svg" | "icons/triangle-alert.svg" => {
                include_bytes!("../assets/icons/warning.svg")
            }
            _ => return Ok(None),
        };
        Ok(Some(Cow::Borrowed(bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        if path == "icons" {
            Ok(OrbitIcon::ALL.iter().map(|icon| icon.path()).collect())
        } else {
            Ok(Vec::new())
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum OrbitIcon {
    Home,
    Mods,
    Browse,
    Audit,
    Runtime,
    Account,
    Server,
    Settings,
    Activity,
    Refresh,
    Download,
    Play,
    Plus,
    Search,
    Terminal,
    Java,
    Folder,
    Trash,
    Warning,
    Check,
    Close,
    Orbit,
}

impl OrbitIcon {
    const ALL: [Self; 22] = [
        Self::Home,
        Self::Mods,
        Self::Browse,
        Self::Audit,
        Self::Runtime,
        Self::Account,
        Self::Server,
        Self::Settings,
        Self::Activity,
        Self::Refresh,
        Self::Download,
        Self::Play,
        Self::Plus,
        Self::Search,
        Self::Terminal,
        Self::Java,
        Self::Folder,
        Self::Trash,
        Self::Warning,
        Self::Check,
        Self::Close,
        Self::Orbit,
    ];
}

impl IconNamed for OrbitIcon {
    fn path(self) -> SharedString {
        let name = match self {
            Self::Home => "home",
            Self::Mods => "mods",
            Self::Browse => "browse",
            Self::Audit => "audit",
            Self::Runtime => "runtime",
            Self::Account => "account",
            Self::Server => "server",
            Self::Settings => "settings",
            Self::Activity => "activity",
            Self::Refresh => "refresh",
            Self::Download => "download",
            Self::Play => "play",
            Self::Plus => "plus",
            Self::Search => "search",
            Self::Terminal => "terminal",
            Self::Java => "java",
            Self::Folder => "folder",
            Self::Trash => "trash",
            Self::Warning => "warning",
            Self::Check => "check",
            Self::Close => "close",
            Self::Orbit => "orbit",
        };
        format!("icons/{name}.svg").into()
    }
}
