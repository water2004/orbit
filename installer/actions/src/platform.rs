//! Native path discovery belongs to the packaging host, never to cleanup logic.
use crate::{discovery::Roots, transaction::Result};
use orbit_launcher_core::{NativeRuntimeEnvironment, RuntimeEnvironment};

pub fn current_user() -> Result<Roots> {
    #[cfg(target_os = "linux")]
    if std::env::var_os("SUDO_USER").is_some() || std::env::var_os("PKEXEC_UID").is_some() {
        return Err("run orbit-maintenance without sudo/pkexec, as the owner of the data".into());
    }
    let environment = NativeRuntimeEnvironment;
    let config = environment.config_root()?;
    let data = environment.data_root()?;
    let cache = environment.cache_root()?;
    let gui = directories::ProjectDirs::from("dev", "Orbit", "Orbit GUI")
        .ok_or("cannot locate GUI storage")?;
    let base = directories::BaseDirs::new().ok_or("cannot locate user storage")?;
    let mut forbidden = vec![
        base.home_dir().into(),
        config.clone(),
        data.clone(),
        cache.clone(),
    ];
    forbidden.push(recovery_directory()?);
    #[cfg(unix)]
    forbidden
        .extend(["/usr", "/etc", "/var", "/tmp", "/home", "/opt"].map(std::path::PathBuf::from));
    #[cfg(windows)]
    forbidden.extend(
        ["SystemRoot", "ProgramFiles", "ProgramData", "TEMP"]
            .into_iter()
            .filter_map(std::env::var_os)
            .map(Into::into),
    );
    Ok(Roots {
        config,
        gui_webview: data.join(env!("ORBIT_GUI_IDENTIFIER")),
        data,
        cache,
        gui_config: gui.config_dir().into(),
        gui_data: gui.data_dir().into(),
        gui_cache: gui.cache_dir().into(),
        forbidden,
    })
}

pub fn recovery_directory() -> Result<std::path::PathBuf> {
    let base = directories::BaseDirs::new().ok_or("cannot locate user storage")?;
    Ok(base.data_local_dir().join("orbit-maintenance"))
}
