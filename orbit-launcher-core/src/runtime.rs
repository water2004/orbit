use std::path::{Path, PathBuf};

use crate::config::GlobalConfig;
use crate::error::LauncherError;

const APPLICATION_DIRECTORY: &str = "orbit-launcher";

pub trait RuntimeEnvironment {
    fn config_root(&self) -> Result<PathBuf, LauncherError>;
    fn data_root(&self) -> Result<PathBuf, LauncherError>;
    fn cache_root(&self) -> Result<PathBuf, LauncherError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NativeRuntimeEnvironment;

impl RuntimeEnvironment for NativeRuntimeEnvironment {
    fn config_root(&self) -> Result<PathBuf, LauncherError> {
        platform::config_root()
    }

    fn data_root(&self) -> Result<PathBuf, LauncherError> {
        platform::data_root()
    }

    fn cache_root(&self) -> Result<PathBuf, LauncherError> {
        platform::cache_root()
    }
}

#[derive(Debug, Clone, Default)]
pub struct RuntimePathOptions {
    pub config_dir: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
    pub cache_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePaths {
    config_dir: PathBuf,
    data_dir: PathBuf,
    cache_dir: PathBuf,
}

impl RuntimePaths {
    pub fn resolve(options: &RuntimePathOptions) -> Result<Self, LauncherError> {
        Self::resolve_with(&NativeRuntimeEnvironment, options)
    }

    pub fn resolve_with(
        environment: &dyn RuntimeEnvironment,
        options: &RuntimePathOptions,
    ) -> Result<Self, LauncherError> {
        let config_dir = options.config_dir.clone().map(Ok).unwrap_or_else(|| {
            environment
                .config_root()
                .map(|root| root.join(APPLICATION_DIRECTORY))
        })?;
        let data_dir = options.data_dir.clone().map(Ok).unwrap_or_else(|| {
            environment
                .data_root()
                .map(|root| root.join(APPLICATION_DIRECTORY))
        })?;
        let cache_dir = options.cache_dir.clone().map(Ok).unwrap_or_else(|| {
            environment
                .cache_root()
                .map(|root| root.join(APPLICATION_DIRECTORY))
        })?;
        Ok(Self {
            config_dir,
            data_dir,
            cache_dir,
        })
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    pub fn instances_file(&self) -> PathBuf {
        self.data_dir.join("instances.toml")
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeContext {
    paths: RuntimePaths,
    config: GlobalConfig,
}

impl RuntimeContext {
    pub fn load(options: RuntimePathOptions) -> Result<Self, LauncherError> {
        Self::load_with(&NativeRuntimeEnvironment, options)
    }

    pub fn load_with(
        environment: &dyn RuntimeEnvironment,
        options: RuntimePathOptions,
    ) -> Result<Self, LauncherError> {
        let paths = RuntimePaths::resolve_with(environment, &options)?;
        let config = GlobalConfig::load(&paths.config_file())?;
        Ok(Self { paths, config })
    }

    pub fn paths(&self) -> &RuntimePaths {
        &self.paths
    }

    pub fn config(&self) -> &GlobalConfig {
        &self.config
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use std::path::PathBuf;

    use crate::error::LauncherError;

    fn required(variable: &str) -> Result<PathBuf, LauncherError> {
        std::env::var_os(variable)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| {
                LauncherError::InvalidConfig(format!(
                    "{variable} is not set; pass explicit launcher directories"
                ))
            })
    }

    pub(super) fn config_root() -> Result<PathBuf, LauncherError> {
        required("APPDATA")
    }

    pub(super) fn data_root() -> Result<PathBuf, LauncherError> {
        required("LOCALAPPDATA").or_else(|_| required("APPDATA"))
    }

    pub(super) fn cache_root() -> Result<PathBuf, LauncherError> {
        data_root()
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::path::PathBuf;

    use crate::error::LauncherError;

    fn home() -> Result<PathBuf, LauncherError> {
        std::env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| {
                LauncherError::InvalidConfig(
                    "HOME is not set; pass explicit launcher directories".to_string(),
                )
            })
    }

    fn xdg(variable: &str, fallback: &str) -> Result<PathBuf, LauncherError> {
        Ok(std::env::var_os(variable)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or(home()?.join(fallback)))
    }

    pub(super) fn config_root() -> Result<PathBuf, LauncherError> {
        xdg("XDG_CONFIG_HOME", ".config")
    }

    pub(super) fn data_root() -> Result<PathBuf, LauncherError> {
        xdg("XDG_DATA_HOME", ".local/share")
    }

    pub(super) fn cache_root() -> Result<PathBuf, LauncherError> {
        xdg("XDG_CACHE_HOME", ".cache")
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::path::PathBuf;

    use crate::error::LauncherError;

    fn home() -> Result<PathBuf, LauncherError> {
        std::env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| {
                LauncherError::InvalidConfig(
                    "HOME is not set; pass explicit launcher directories".to_string(),
                )
            })
    }

    pub(super) fn config_root() -> Result<PathBuf, LauncherError> {
        Ok(home()?.join("Library/Application Support"))
    }

    pub(super) fn data_root() -> Result<PathBuf, LauncherError> {
        config_root()
    }

    pub(super) fn cache_root() -> Result<PathBuf, LauncherError> {
        Ok(home()?.join("Library/Caches"))
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
mod platform {
    use std::path::PathBuf;

    use crate::error::LauncherError;

    pub(super) fn config_root() -> Result<PathBuf, LauncherError> {
        Err(LauncherError::UnsupportedPlatform)
    }

    pub(super) fn data_root() -> Result<PathBuf, LauncherError> {
        Err(LauncherError::UnsupportedPlatform)
    }

    pub(super) fn cache_root() -> Result<PathBuf, LauncherError> {
        Err(LauncherError::UnsupportedPlatform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeEnvironment;

    impl RuntimeEnvironment for FakeEnvironment {
        fn config_root(&self) -> Result<PathBuf, LauncherError> {
            Ok(PathBuf::from("/config"))
        }

        fn data_root(&self) -> Result<PathBuf, LauncherError> {
            Ok(PathBuf::from("/data"))
        }

        fn cache_root(&self) -> Result<PathBuf, LauncherError> {
            Ok(PathBuf::from("/cache"))
        }
    }

    #[test]
    fn resolves_independent_platform_roots() {
        let paths =
            RuntimePaths::resolve_with(&FakeEnvironment, &RuntimePathOptions::default()).unwrap();
        assert_eq!(
            paths.config_file(),
            PathBuf::from("/config/orbit-launcher/config.toml")
        );
        assert_eq!(
            paths.instances_file(),
            PathBuf::from("/data/orbit-launcher/instances.toml")
        );
        assert_eq!(paths.cache_dir(), Path::new("/cache/orbit-launcher"));
    }

    #[test]
    fn explicit_directories_do_not_consult_the_platform() {
        struct Unavailable;
        impl RuntimeEnvironment for Unavailable {
            fn config_root(&self) -> Result<PathBuf, LauncherError> {
                Err(LauncherError::UnsupportedPlatform)
            }
            fn data_root(&self) -> Result<PathBuf, LauncherError> {
                Err(LauncherError::UnsupportedPlatform)
            }
            fn cache_root(&self) -> Result<PathBuf, LauncherError> {
                Err(LauncherError::UnsupportedPlatform)
            }
        }

        let paths = RuntimePaths::resolve_with(
            &Unavailable,
            &RuntimePathOptions {
                config_dir: Some(PathBuf::from("config")),
                data_dir: Some(PathBuf::from("data")),
                cache_dir: Some(PathBuf::from("cache")),
            },
        )
        .unwrap();
        assert_eq!(paths.config_dir(), Path::new("config"));
    }
}
