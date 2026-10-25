//! GUI-owned presentation storage. MSI calls the same writer in a headless
//! process instead of interpreting or patching the preference format itself.
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Serialize, Deserialize)]
pub struct Preferences {
    #[serde(default)]
    pub close_after_launch: bool,
    pub orbit_binary: PathBuf,
    pub launcher_binary: PathBuf,
    pub selected_instance: Option<String>,
    #[serde(default)]
    pub language: orbit_i18n::LanguageMode,
    pub theme_mode: String,
    pub accent_theme: String,
    #[serde(default)]
    installation_binding: Option<InstallationBinding>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct InstallationBinding {
    executable: PathBuf,
    size: u64,
    modified_nanos: u128,
}

impl InstallationBinding {
    fn current() -> Result<Self, String> {
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        Self::for_executable(executable)
    }
    fn for_executable(executable: PathBuf) -> Result<Self, String> {
        let metadata = std::fs::metadata(&executable).map_err(|error| error.to_string())?;
        Ok(Self {
            executable,
            size: metadata.len(),
            modified_nanos: metadata
                .modified()
                .map_err(|error| error.to_string())?
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|error| error.to_string())?
                .as_nanos(),
        })
    }
}

impl Preferences {
    fn path() -> Result<PathBuf, String> {
        ProjectDirs::from("dev", "Orbit", "Orbit GUI")
            .map(|dirs| dirs.config_dir().join("preferences.json"))
            .ok_or_else(|| "Cannot locate GUI preferences directory".into())
    }
    pub fn load() -> Result<Self, String> {
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        Self::load_at(
            &Self::path()?,
            executable
                .parent()
                .ok_or("Executable has no parent directory")?,
        )
    }
    fn load_at(path: &Path, directory: &Path) -> Result<Self, String> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self {
                close_after_launch: false,
                orbit_binary: directory.join(executable_name("orbit")),
                launcher_binary: directory.join(executable_name("orbit-launcher")),
                selected_instance: None,
                language: Default::default(),
                theme_mode: "system".into(),
                accent_theme: "indigo".into(),
                installation_binding: None,
            }),
            Err(error) => Err(error.to_string()),
        }
    }
    pub fn save(&self) -> Result<(), String> {
        self.save_at(&Self::path()?)
    }
    pub fn retain_installation_binding(&mut self, previous: &Self) {
        self.installation_binding = previous.installation_binding.clone();
    }
    fn save_at(&self, path: &Path) -> Result<(), String> {
        if !["system", "light", "dark"].contains(&self.theme_mode.as_str())
            || !["indigo", "emerald", "amber"].contains(&self.accent_theme.as_str())
        {
            return Err("Unknown appearance setting".into());
        }
        let parent = path.parent().ok_or("Invalid preferences path")?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        serde_json::to_writer_pretty(&mut file, self).map_err(|e| e.to_string())?;
        file.as_file().sync_all().map_err(|e| e.to_string())?;
        file.persist(path).map_err(|e| e.to_string())?;
        Ok(())
    }
}
fn executable_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    }
}

pub fn install_cli_bindings() -> Result<(), String> {
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    install_at(
        &Preferences::path()?,
        executable
            .parent()
            .ok_or("Executable has no parent directory")?,
    )
}

/// Rebind each user's preferences once per installed executable. This also
/// covers users who were not present during a system-wide installation.
pub fn refresh_installation() -> Result<(), String> {
    refresh_at(&Preferences::path()?, InstallationBinding::current()?)
}
fn refresh_at(path: &Path, binding: InstallationBinding) -> Result<(), String> {
    let directory = binding
        .executable
        .parent()
        .ok_or("Executable has no parent directory")?
        .to_path_buf();
    let preferences = Preferences::load_at(path, &directory)?;
    if preferences.installation_binding.as_ref() != Some(&binding) {
        write_bindings(preferences, path, &directory, binding)?;
    }
    Ok(())
}
fn install_at(path: &Path, directory: &Path) -> Result<(), String> {
    write_bindings(
        Preferences::load_at(path, directory)?,
        path,
        directory,
        InstallationBinding::current()?,
    )
}
fn write_bindings(
    mut preferences: Preferences,
    path: &Path,
    directory: &Path,
    binding: InstallationBinding,
) -> Result<(), String> {
    for name in ["orbit", "orbit-launcher"] {
        let executable = directory.join(executable_name(name));
        if !executable.is_absolute() || !executable.is_file() {
            return Err(format!(
                "Installed CLI executable is missing: {}",
                executable.display()
            ));
        }
    }
    preferences.orbit_binary = directory.join(executable_name("orbit"));
    preferences.launcher_binary = directory.join(executable_name("orbit-launcher"));
    preferences.installation_binding = Some(binding);
    preferences.save_at(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linux_install_refresh_is_once_per_installation_and_keeps_manual_choices() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings/preferences.json");
        let directory = temp.path().join("installed");
        std::fs::create_dir(&directory).unwrap();
        for name in ["orbit", "orbit-launcher", "orbit-gui"] {
            std::fs::write(directory.join(executable_name(name)), "fixture").unwrap();
        }
        let binding =
            InstallationBinding::for_executable(directory.join(executable_name("orbit-gui")))
                .unwrap();
        refresh_at(&path, binding.clone()).unwrap();
        let mut preferences = Preferences::load_at(&path, &directory).unwrap();
        preferences.orbit_binary = temp.path().join("manual-orbit");
        preferences.save_at(&path).unwrap();
        refresh_at(&path, binding.clone()).unwrap();
        assert_eq!(
            Preferences::load_at(&path, &directory)
                .unwrap()
                .orbit_binary,
            preferences.orbit_binary
        );
        let mut updated = binding;
        updated.modified_nanos += 1;
        refresh_at(&path, updated).unwrap();
        assert_eq!(
            Preferences::load_at(&path, &directory)
                .unwrap()
                .orbit_binary,
            directory.join(executable_name("orbit"))
        );
    }
    #[test]
    fn presentation_updates_cannot_reset_the_installation_binding() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("preferences.json");
        let mut previous = Preferences::load_at(&path, temp.path()).unwrap();
        previous.installation_binding = Some(InstallationBinding::current().unwrap());
        let mut value = serde_json::to_value(&previous).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("installation_binding");
        value["theme_mode"] = "dark".into();
        let mut update: Preferences = serde_json::from_value(value).unwrap();
        update.retain_installation_binding(&previous);
        assert!(update.installation_binding == previous.installation_binding);
        assert_eq!(update.theme_mode, "dark");
    }
    #[test]
    fn relocated_install_overwrites_only_cli_bindings() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings/preferences.json");
        let old = temp.path().join("old install");
        let new = temp.path().join("new install");
        std::fs::create_dir_all(&new).unwrap();
        for name in ["orbit", "orbit-launcher"] {
            std::fs::write(new.join(executable_name(name)), "fixture").unwrap();
        }
        let mut preferences = Preferences::load_at(&path, &old).unwrap();
        preferences.selected_instance = Some("keep-instance".into());
        preferences.theme_mode = "dark".into();
        preferences.accent_theme = "emerald".into();
        preferences.language = serde_json::from_str("\"zh-CN\"").unwrap();
        preferences.save_at(&path).unwrap();
        install_at(&path, &new).unwrap();
        let current = Preferences::load_at(&path, &new).unwrap();
        assert_eq!(current.orbit_binary, new.join(executable_name("orbit")));
        assert_eq!(
            current.launcher_binary,
            new.join(executable_name("orbit-launcher"))
        );
        assert_eq!(current.selected_instance.as_deref(), Some("keep-instance"));
        assert_eq!(current.theme_mode, "dark");
        assert_eq!(current.accent_theme, "emerald");
        assert_eq!(
            serde_json::to_string(&current.language).unwrap(),
            "\"zh-CN\""
        );
        let bytes = std::fs::read(&path).unwrap();
        install_at(&path, &new).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    #[test]
    fn missing_cli_or_corrupt_preferences_does_not_reset_existing_settings() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("preferences.json");
        let preferences = Preferences::load_at(&path, temp.path()).unwrap();
        preferences.save_at(&path).unwrap();
        let original = std::fs::read(&path).unwrap();
        assert!(install_at(&path, temp.path()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        std::fs::write(&path, "invalid JSON").unwrap();
        assert!(install_at(&path, temp.path()).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "invalid JSON");
    }
}
