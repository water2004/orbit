//! Runtime path selection and process-wide services.
//!
//! Business modules consume [`RuntimeContext`] instead of consulting process
//! environment variables or platform-specific directories themselves.

use std::path::{Path, PathBuf};

use crate::config::GlobalConfig;
use crate::error::OrbitError;
use crate::jar_cache::JarCache;

const APPLICATION_DIRECTORY: &str = "orbit";

/// Built-in directory layouts. Callers may also override individual paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathLayout {
    /// Store configuration beside the executable and cache under `cache/`.
    Executable,
    /// Use the platform's configuration and cache roots.
    System,
}

impl std::str::FromStr for PathLayout {
    type Err = OrbitError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "executable" => Ok(Self::Executable),
            "system" => Ok(Self::System),
            other => Err(OrbitError::Other(anyhow::anyhow!(
                "unknown data layout '{other}'; expected 'system' or 'executable'"
            ))),
        }
    }
}

/// Compile-time default. The `portable` Cargo feature changes only this
/// default; callers can still select either layout at runtime.
pub const fn compiled_default_layout() -> PathLayout {
    if cfg!(feature = "portable") {
        PathLayout::Executable
    } else {
        PathLayout::System
    }
}

/// Platform-specific directory discovery.
///
/// Implementations return roots only. Orbit's common layout (`orbit/`,
/// `config.toml`, `instances.toml`, `cache/`) is assembled by
/// [`RuntimePaths`].
pub trait RuntimeEnvironment {
    fn executable_dir(&self) -> Result<PathBuf, OrbitError>;
    fn config_root(&self) -> Result<PathBuf, OrbitError>;
    fn cache_root(&self) -> Result<PathBuf, OrbitError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NativeRuntimeEnvironment;

impl RuntimeEnvironment for NativeRuntimeEnvironment {
    fn executable_dir(&self) -> Result<PathBuf, OrbitError> {
        let executable = std::env::current_exe().map_err(|error| {
            OrbitError::Other(anyhow::anyhow!(
                "failed to locate the Orbit executable: {error}"
            ))
        })?;
        executable.parent().map(Path::to_path_buf).ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!(
                "Orbit executable path '{}' has no parent directory",
                executable.display()
            ))
        })
    }

    fn config_root(&self) -> Result<PathBuf, OrbitError> {
        platform::config_root()
    }

    fn cache_root(&self) -> Result<PathBuf, OrbitError> {
        platform::cache_root()
    }
}

#[derive(Debug, Clone, Default)]
pub struct RuntimePathOptions {
    pub layout: Option<PathLayout>,
    /// Exact path of the global `config.toml`.
    pub config_file: Option<PathBuf>,
    /// Exact directory of the global JAR cache.
    pub cache_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePaths {
    config_file: PathBuf,
    instances_file: PathBuf,
    cache_dir: PathBuf,
}

impl RuntimePaths {
    pub fn resolve(options: &RuntimePathOptions) -> Result<Self, OrbitError> {
        Self::resolve_with(&NativeRuntimeEnvironment, options)
    }

    pub fn resolve_with(
        environment: &dyn RuntimeEnvironment,
        options: &RuntimePathOptions,
    ) -> Result<Self, OrbitError> {
        let layout = options.layout.unwrap_or_else(compiled_default_layout);
        let config_file = resolve_config_file(environment, options)?;
        let instances_file = config_file
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("instances.toml");
        let cache_dir = match &options.cache_dir {
            Some(path) => path.clone(),
            None => match layout {
                PathLayout::Executable => environment.executable_dir()?.join("cache"),
                PathLayout::System => environment.cache_root()?.join(APPLICATION_DIRECTORY),
            },
        };
        Ok(Self {
            config_file,
            instances_file,
            cache_dir,
        })
    }

    pub fn config_file(&self) -> &Path {
        &self.config_file
    }

    pub fn instances_file(&self) -> &Path {
        &self.instances_file
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeContext {
    paths: RuntimePaths,
    config: GlobalConfig,
    jar_cache: JarCache,
}

impl RuntimeContext {
    pub fn load(options: RuntimePathOptions) -> Result<Self, OrbitError> {
        Self::load_with(&NativeRuntimeEnvironment, options)
    }

    pub fn load_with(
        environment: &dyn RuntimeEnvironment,
        mut options: RuntimePathOptions,
    ) -> Result<Self, OrbitError> {
        let config_file = resolve_config_file(environment, &options)?;
        let config = GlobalConfig::load(&config_file)?;
        options.config_file = Some(config_file);
        if options.cache_dir.is_none() {
            options.cache_dir = config.cache.dir.as_deref().map(PathBuf::from);
        }
        let paths = RuntimePaths::resolve_with(environment, &options)?;
        let jar_cache = JarCache::open(paths.cache_dir().to_path_buf())?;
        Ok(Self {
            paths,
            config,
            jar_cache,
        })
    }

    pub fn paths(&self) -> &RuntimePaths {
        &self.paths
    }

    pub fn config(&self) -> &GlobalConfig {
        &self.config
    }

    pub fn jar_cache(&self) -> &JarCache {
        &self.jar_cache
    }
}

fn resolve_config_file(
    environment: &dyn RuntimeEnvironment,
    options: &RuntimePathOptions,
) -> Result<PathBuf, OrbitError> {
    match &options.config_file {
        Some(path) => Ok(path.clone()),
        None => match options.layout.unwrap_or_else(compiled_default_layout) {
            PathLayout::Executable => Ok(environment.executable_dir()?.join("config.toml")),
            PathLayout::System => Ok(environment
                .config_root()?
                .join(APPLICATION_DIRECTORY)
                .join("config.toml")),
        },
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use std::path::PathBuf;

    use crate::error::OrbitError;

    fn required_directory(variable: &str) -> Result<PathBuf, OrbitError> {
        std::env::var_os(variable)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| {
                OrbitError::Other(anyhow::anyhow!(
                    "{variable} is not set; pass explicit runtime paths or use executable layout"
                ))
            })
    }

    pub(super) fn config_root() -> Result<PathBuf, OrbitError> {
        required_directory("APPDATA")
    }

    pub(super) fn cache_root() -> Result<PathBuf, OrbitError> {
        required_directory("LOCALAPPDATA").or_else(|_| required_directory("APPDATA"))
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::path::PathBuf;

    use crate::error::OrbitError;

    fn home() -> Result<PathBuf, OrbitError> {
        std::env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| {
                OrbitError::Other(anyhow::anyhow!(
                    "HOME is not set; pass explicit runtime paths or use executable layout"
                ))
            })
    }

    pub(super) fn config_root() -> Result<PathBuf, OrbitError> {
        Ok(std::env::var_os("XDG_CONFIG_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or(home()?.join(".config")))
    }

    pub(super) fn cache_root() -> Result<PathBuf, OrbitError> {
        Ok(std::env::var_os("XDG_CACHE_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or(home()?.join(".cache")))
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::path::PathBuf;

    use crate::error::OrbitError;

    fn home() -> Result<PathBuf, OrbitError> {
        std::env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| {
                OrbitError::Other(anyhow::anyhow!(
                    "HOME is not set; pass explicit runtime paths or use executable layout"
                ))
            })
    }

    pub(super) fn config_root() -> Result<PathBuf, OrbitError> {
        Ok(home()?.join("Library").join("Application Support"))
    }

    pub(super) fn cache_root() -> Result<PathBuf, OrbitError> {
        Ok(home()?.join("Library").join("Caches"))
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
mod platform {
    use std::path::PathBuf;

    use crate::error::OrbitError;

    pub(super) fn config_root() -> Result<PathBuf, OrbitError> {
        Err(OrbitError::Other(anyhow::anyhow!(
            "system data layout is unsupported on this target; pass explicit runtime paths or use executable layout"
        )))
    }

    pub(super) fn cache_root() -> Result<PathBuf, OrbitError> {
        config_root()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeEnvironment;

    impl RuntimeEnvironment for FakeEnvironment {
        fn executable_dir(&self) -> Result<PathBuf, OrbitError> {
            Ok(PathBuf::from("/opt/orbit/bin"))
        }

        fn config_root(&self) -> Result<PathBuf, OrbitError> {
            Ok(PathBuf::from("/platform/config"))
        }

        fn cache_root(&self) -> Result<PathBuf, OrbitError> {
            Ok(PathBuf::from("/platform/cache"))
        }
    }

    struct UnavailableEnvironment;

    impl RuntimeEnvironment for UnavailableEnvironment {
        fn executable_dir(&self) -> Result<PathBuf, OrbitError> {
            Err(OrbitError::Other(anyhow::anyhow!("unavailable")))
        }

        fn config_root(&self) -> Result<PathBuf, OrbitError> {
            Err(OrbitError::Other(anyhow::anyhow!("unavailable")))
        }

        fn cache_root(&self) -> Result<PathBuf, OrbitError> {
            Err(OrbitError::Other(anyhow::anyhow!("unavailable")))
        }
    }

    #[test]
    fn system_layout_keeps_config_and_cache_in_platform_roots() {
        let paths = RuntimePaths::resolve_with(
            &FakeEnvironment,
            &RuntimePathOptions {
                layout: Some(PathLayout::System),
                ..RuntimePathOptions::default()
            },
        )
        .unwrap();

        assert_eq!(
            paths.config_file(),
            Path::new("/platform/config/orbit/config.toml")
        );
        assert_eq!(
            paths.instances_file(),
            Path::new("/platform/config/orbit/instances.toml")
        );
        assert_eq!(paths.cache_dir(), Path::new("/platform/cache/orbit"));
    }

    #[test]
    fn executable_layout_and_explicit_overrides_are_platform_independent() {
        let paths = RuntimePaths::resolve_with(
            &UnavailableEnvironment,
            &RuntimePathOptions {
                layout: Some(PathLayout::Executable),
                config_file: Some(PathBuf::from("/custom/global.toml")),
                cache_dir: Some(PathBuf::from("/custom/jars")),
            },
        )
        .unwrap();

        assert_eq!(paths.config_file(), Path::new("/custom/global.toml"));
        assert_eq!(paths.instances_file(), Path::new("/custom/instances.toml"));
        assert_eq!(paths.cache_dir(), Path::new("/custom/jars"));
    }

    #[test]
    fn configured_cache_path_does_not_require_platform_discovery() {
        let directory =
            std::env::temp_dir().join(format!("orbit-runtime-config-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let config_file = directory.join("global.toml");
        let cache_dir = directory.join("configured-cache");
        std::fs::write(
            &config_file,
            format!("[cache]\ndir = {:?}\n", cache_dir.to_string_lossy()),
        )
        .unwrap();

        let runtime = RuntimeContext::load_with(
            &UnavailableEnvironment,
            RuntimePathOptions {
                config_file: Some(config_file),
                ..RuntimePathOptions::default()
            },
        )
        .unwrap();

        assert_eq!(runtime.paths().cache_dir(), cache_dir);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
