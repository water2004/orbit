use crate::{
    discovery::{self, CleanupLevel, Plan, Roots},
    transaction::Result,
};
use std::path::PathBuf;
use windows_sys::Win32::System::ApplicationInstallationAndServicing::*;

const SUCCESS: u32 = 0;
const CANCELLED: u32 = 1602;
const FAILED: u32 = 1603;

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn property(session: MSIHANDLE, name: &str) -> Result<String> {
    let name = wide(name);
    let mut length = 0;
    // SAFETY: valid terminated strings, session provided by Windows Installer.
    let status =
        unsafe { MsiGetPropertyW(session, name.as_ptr(), std::ptr::null_mut(), &mut length) };
    if status != SUCCESS && status != 234 {
        return Err(format!("cannot read MSI property: {status}").into());
    }
    if length > 4 * 1024 * 1024 {
        return Err("MSI cleanup property exceeds its size bound".into());
    }
    let mut buffer = vec![0; length as usize + 1];
    length += 1;
    // SAFETY: buffer includes room for terminator; length is capacity in UTF-16 units.
    let status =
        unsafe { MsiGetPropertyW(session, name.as_ptr(), buffer.as_mut_ptr(), &mut length) };
    if status != SUCCESS {
        return Err(format!("cannot read MSI property: {status}").into());
    }
    Ok(String::from_utf16(&buffer[..length as usize])?)
}

fn set_property(session: MSIHANDLE, name: &str, value: &str) -> Result<()> {
    if value.contains('\0') {
        return Err("MSI property contains NUL".into());
    }
    let name = wide(name);
    let value = wide(value);
    // SAFETY: the strings outlive the call and are NUL terminated.
    let status = unsafe { MsiSetPropertyW(session, name.as_ptr(), value.as_ptr()) };
    if status == SUCCESS {
        Ok(())
    } else {
        Err(format!("cannot set MSI property: {status}").into())
    }
}

fn roots(session: MSIHANDLE) -> Result<Roots> {
    let roaming = dunce::canonicalize(PathBuf::from(property(session, "AppDataFolder")?))?;
    let local = dunce::canonicalize(PathBuf::from(property(session, "LocalAppDataFolder")?))?;
    let mut forbidden = Vec::new();
    for name in [
        "WindowsFolder",
        "SystemFolder",
        "ProgramFiles64Folder",
        "CommonAppDataFolder",
        "PersonalFolder",
        "DesktopFolder",
        "CommonFiles64Folder",
        "TempFolder",
    ] {
        let value = property(session, name)?;
        if !value.is_empty() {
            forbidden.push(PathBuf::from(value));
        }
    }
    if let Some(profile) = local.parent().and_then(|p| p.parent()) {
        forbidden.push(profile.to_path_buf());
    }
    let gui = directories::ProjectDirs::from("dev", "Orbit", "Orbit GUI")
        .ok_or("cannot resolve GUI project storage")?;
    let project = gui.project_path();
    Ok(Roots {
        config: roaming.clone(),
        data: local.clone(),
        cache: local.clone(),
        gui_config: roaming.join(project).join("config"),
        gui_data: local.join(project).join("data"),
        gui_cache: local.join(project).join("cache"),
        gui_webview: local.join(env!("ORBIT_GUI_IDENTIFIER")),
        forbidden,
    })
}

fn prepare(session: MSIHANDLE) -> Result<u32> {
    set_property(session, "ORBIT_DATA_CLEANUP_READY", "")?;
    // Defense in depth: never clean on upgrade, repair or feature/profile changes.
    if property(session, "REMOVE")? != "ALL"
        || !property(session, "UPGRADINGPRODUCTCODE")?.is_empty()
    {
        return Ok(SUCCESS);
    }
    let interactive = property(session, "UILevel")?.parse::<u32>()? >= 3;
    let mut level: CleanupLevel = property(session, "UNINSTALL_DATA_LEVEL")?.parse()?;
    // Settings/Control Panel can request basic UI, bypassing authored MSI dialogs.
    if interactive && property(session, "ORBIT_CLEANUP_OPTIONS_SEEN")? != "1" {
        let Some(selected) = crate::controls::choose_level()? else {
            return Ok(CANCELLED);
        };
        level = selected;
    }
    let plan = discovery::discover(roots(session)?, level)?;
    if plan.game_entries.is_empty() && plan.data_entries.is_empty() {
        return Ok(SUCCESS);
    }
    if property(session, "RollbackDisabled")? == "1" {
        return Err("data cleanup requires Windows Installer rollback support".into());
    }
    if interactive && !crate::controls::confirm(&discovery::preview(&plan))? {
        return Ok(CANCELLED);
    }
    let encoded = serde_json::to_string(&plan)?;
    for action in [
        "OrbitRollbackCleanup",
        "OrbitStageCleanup",
        "OrbitCommitCleanup",
    ] {
        set_property(session, action, &encoded)?;
    }
    set_property(session, "ORBIT_DATA_CLEANUP_READY", "1")?;
    Ok(SUCCESS)
}

fn data(session: MSIHANDLE) -> Result<Plan> {
    Ok(serde_json::from_str(&property(
        session,
        "CustomActionData",
    )?)?)
}

fn stage(session: MSIHANDLE) -> Result<u32> {
    let plan = data(session)?;
    crate::execution::stage(&plan)?;
    Ok(SUCCESS)
}
fn rollback(session: MSIHANDLE) -> Result<u32> {
    crate::execution::rollback(&data(session)?)?;
    Ok(SUCCESS)
}
fn commit(session: MSIHANDLE) -> Result<u32> {
    let plan = data(session)?;
    if let Err(error) = crate::execution::commit(&plan) {
        report(
            session,
            INSTALLMESSAGE_WARNING,
            &format!(
                "Orbit was uninstalled, but data cleanup is incomplete. Recovery folders were retained: {error}"
            ),
        );
    }
    Ok(SUCCESS)
}

fn entry(session: MSIHANDLE, action: fn(MSIHANDLE) -> Result<u32>) -> u32 {
    let result = std::panic::catch_unwind(|| action(session));
    let error = match result {
        Ok(Ok(status)) => return status,
        Ok(Err(error)) => error.to_string(),
        Err(_) => {
            "uninstall cleanup panicked; retained recovery directories must not be deleted".into()
        }
    };
    report(
        session,
        INSTALLMESSAGE_ERROR,
        &format!("Orbit data cleanup failed: {error}"),
    );
    FAILED
}

fn report(session: MSIHANDLE, kind: INSTALLMESSAGE, text: &str) {
    let message = wide(text);
    // SAFETY: MSI owns the session; the created record is closed in this block.
    unsafe {
        let record = MsiCreateRecord(0);
        if record != 0 {
            MsiRecordSetStringW(record, 0, message.as_ptr());
            MsiProcessMessage(session, kind, record);
            MsiCloseHandle(record);
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn OrbitPrepareCleanup(session: MSIHANDLE) -> u32 {
    entry(session, prepare)
}
#[unsafe(no_mangle)]
pub extern "system" fn OrbitStageCleanup(session: MSIHANDLE) -> u32 {
    entry(session, stage)
}
#[unsafe(no_mangle)]
pub extern "system" fn OrbitRollbackCleanup(session: MSIHANDLE) -> u32 {
    entry(session, rollback)
}
#[unsafe(no_mangle)]
pub extern "system" fn OrbitCommitCleanup(session: MSIHANDLE) -> u32 {
    entry(session, commit)
}
