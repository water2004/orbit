//! Native Windows task dialogs for basic-UI uninstall paths. No GUI process.
use crate::{discovery::CleanupLevel, transaction::Result};
use windows_sys::Win32::{
    Foundation::{FreeLibrary, HANDLE, HMODULE, INVALID_HANDLE_VALUE},
    System::{ApplicationInstallationAndServicing::*, LibraryLoader::*},
    UI::{Controls::*, WindowsAndMessaging::*},
};

type TaskDialog =
    unsafe extern "system" fn(*const TASKDIALOGCONFIG, *mut i32, *mut i32, *mut i32) -> i32;
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

struct Controls {
    context: HANDLE,
    cookie: usize,
    library: HMODULE,
    task_dialog: TaskDialog,
    _manifest: tempfile::TempDir,
}
impl Controls {
    fn load() -> Result<Self> {
        let manifest = tempfile::tempdir()?;
        let path = manifest.path().join("controls.manifest");
        std::fs::write(&path, include_bytes!("common-controls.manifest"))?;
        let source = wide(
            path.to_str()
                .ok_or("non-Unicode activation manifest path")?,
        );
        let config = ACTCTXW {
            cbSize: std::mem::size_of::<ACTCTXW>() as u32,
            lpSource: source.as_ptr(),
            ..Default::default()
        };
        // SAFETY: config and its terminated path live through CreateActCtxW.
        let context = unsafe { CreateActCtxW(&config) };
        if context == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut cookie = 0;
        // SAFETY: context is a live activation-context handle.
        if unsafe { ActivateActCtx(context, &mut cookie) } == 0 {
            unsafe {
                ReleaseActCtx(context);
            }
            return Err(std::io::Error::last_os_error().into());
        }
        let name = wide("comctl32.dll");
        // Bind under the v6 context, from the system search path only. A direct
        // import could bind to MSI's older Common Controls before activation.
        let library = unsafe {
            LoadLibraryExW(
                name.as_ptr(),
                std::ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        };
        let function = if library.is_null() {
            None
        } else {
            unsafe { GetProcAddress(library, c"TaskDialogIndirect".as_ptr().cast()) }
        };
        let Some(function) = function else {
            unsafe {
                if !library.is_null() {
                    FreeLibrary(library);
                }
                DeactivateActCtx(0, cookie);
                ReleaseActCtx(context);
            }
            return Err("Windows Common Controls v6 task dialog is unavailable".into());
        };
        // SAFETY: TaskDialogIndirect's documented ABI exactly matches TaskDialog.
        let task_dialog = unsafe {
            std::mem::transmute::<unsafe extern "system" fn() -> isize, TaskDialog>(function)
        };
        Ok(Self {
            context,
            cookie,
            library,
            task_dialog,
            _manifest: manifest,
        })
    }
    fn show(&self, config: &TASKDIALOGCONFIG) -> Result<(i32, i32)> {
        let mut button = 0;
        let mut radio = 0;
        // SAFETY: all text/array buffers outlive this synchronous dialog call.
        let status =
            unsafe { (self.task_dialog)(config, &mut button, &mut radio, std::ptr::null_mut()) };
        if status < 0 {
            Err(format!("Windows task dialog failed: {status:#x}").into())
        } else {
            Ok((button, radio))
        }
    }
}
impl Drop for Controls {
    fn drop(&mut self) {
        // SAFETY: handles/cookie are uniquely owned, and activation is balanced
        // on the same synchronous MSI callback thread.
        unsafe {
            FreeLibrary(self.library);
            DeactivateActCtx(0, self.cookie);
            ReleaseActCtx(self.context);
        }
    }
}
pub fn choose_level() -> Result<Option<CleanupLevel>> {
    let controls = Controls::load()?;
    let title = wide("Uninstall Orbit");
    let heading = wide("Choose what to remove");
    let content = wide(
        "All three choices remove the program files and every disposable cache. Game deletion is permanent.",
    );
    let labels = [
        wide("Program files and all caches — keep settings and instances"),
        wide("Also delete settings — keep all game instances"),
        wide("Delete everything — including all instances and worlds"),
    ];
    let buttons: Vec<_> = labels
        .iter()
        .enumerate()
        .map(|(index, text)| TASKDIALOG_BUTTON {
            nButtonID: 100 + index as i32,
            pszButtonText: text.as_ptr(),
        })
        .collect();
    let config = TASKDIALOGCONFIG {
        cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
        pszWindowTitle: title.as_ptr(),
        pszMainInstruction: heading.as_ptr(),
        pszContent: content.as_ptr(),
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION | TDF_SIZE_TO_CONTENT,
        dwCommonButtons: TDCBF_OK_BUTTON | TDCBF_CANCEL_BUTTON,
        cRadioButtons: buttons.len() as u32,
        pRadioButtons: buttons.as_ptr(),
        nDefaultRadioButton: 100,
        nDefaultButton: IDCANCEL,
        ..Default::default()
    };
    let (button, radio) = controls.show(&config)?;
    if button != IDOK {
        return Ok(None);
    }
    match radio {
        100 => Ok(Some(CleanupLevel::Cache)),
        101 => Ok(Some(CleanupLevel::Settings)),
        102 => Ok(Some(CleanupLevel::All)),
        _ => Err("invalid cleanup selection".into()),
    }
}
pub fn confirm(details: &str) -> Result<bool> {
    let controls = Controls::load()?;
    let title = wide("Uninstall Orbit");
    let heading = wide("Confirm permanent cleanup");
    let content =
        wide("Review the exact paths below. Cancel stops uninstall without deleting any data.");
    let details = wide(details);
    let config = TASKDIALOGCONFIG {
        cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
        pszWindowTitle: title.as_ptr(),
        pszMainInstruction: heading.as_ptr(),
        pszContent: content.as_ptr(),
        pszExpandedInformation: details.as_ptr(),
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION | TDF_EXPANDED_BY_DEFAULT,
        dwCommonButtons: TDCBF_OK_BUTTON | TDCBF_CANCEL_BUTTON,
        nDefaultButton: IDCANCEL,
        cxWidth: 440,
        ..Default::default()
    };
    Ok(controls.show(&config)?.0 == IDOK)
}

#[cfg(test)]
mod tests {
    #[test]
    fn system_common_controls_can_be_bound_without_displaying_a_dialog() {
        let _controls = super::Controls::load().unwrap();
    }
}
