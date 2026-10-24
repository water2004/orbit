//! Native path prompts must finish outside GPUI's borrowed event callbacks.

use std::path::PathBuf;

use gpui::{App, Context, Entity, Window};
use gpui_component::input::InputState;
use rfd::AsyncFileDialog;

pub(super) struct PathDialog {
    dialog: AsyncFileDialog,
    kind: DialogKind,
}

enum DialogKind {
    OpenFile,
    SaveFile,
    Directory,
}

impl PathDialog {
    pub(super) fn open_file() -> Self {
        Self {
            dialog: AsyncFileDialog::new(),
            kind: DialogKind::OpenFile,
        }
    }

    pub(super) fn save_file(name: impl Into<String>) -> Self {
        Self {
            dialog: AsyncFileDialog::new().set_file_name(name),
            kind: DialogKind::SaveFile,
        }
    }

    pub(super) fn directory() -> Self {
        Self {
            dialog: AsyncFileDialog::new(),
            kind: DialogKind::Directory,
        }
    }

    pub(super) fn filter(mut self, name: &str, extensions: &[&str]) -> Self {
        self.dialog = self.dialog.add_filter(name, extensions);
        self
    }

    pub(super) fn show<T: 'static>(
        self,
        target: &Entity<T>,
        window: &mut Window,
        cx: &mut App,
        on_selected: impl FnOnce(&mut T, PathBuf, &mut Window, &mut Context<T>) + 'static,
    ) {
        let target = target.downgrade();
        let dialog = self.dialog.set_parent(window);
        // A synchronous native dialog pumps Windows messages while the click
        // handler still borrows App/Window/entity state. Await the native async
        // backend before borrowing any of that state again.
        window
            .spawn(cx, async move |cx| {
                let file = match self.kind {
                    DialogKind::OpenFile => dialog.pick_file().await,
                    DialogKind::SaveFile => dialog.save_file().await,
                    DialogKind::Directory => dialog.pick_folder().await,
                };
                if let Some(file) = file {
                    // Closing the owning window/entity while the prompt is
                    // pending cancels delivery. Cancelling the prompt is a no-op.
                    let _ = target.update_in(cx, |target, window, cx| {
                        on_selected(target, file.path().to_path_buf(), window, cx);
                        cx.notify();
                    });
                }
            })
            .detach();
    }

    pub(super) fn select_into(self, input: &Entity<InputState>, window: &mut Window, cx: &mut App) {
        self.show(input, window, cx, |input, path, window, cx| {
            input.set_value(path.display().to_string(), window, cx);
        });
    }
}
