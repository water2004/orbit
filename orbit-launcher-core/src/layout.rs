use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::LauncherError;
use crate::instance::InstanceKind;

const VERSIONS_DIRECTORY: &str = "versions";

/// Exact filesystem layout of a registered runtime.
///
/// Client instances use the standard multi-version Minecraft repository:
/// shared assets, libraries and version metadata live under
/// `minecraft_directory`, while mutable game data lives in the isolated
/// `game_directory` below `versions/`. Dedicated servers remain single-root
/// runtimes because that is their native distribution model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum InstanceLocation {
    Client {
        minecraft_directory: PathBuf,
        game_directory: PathBuf,
    },
    Server {
        server_directory: PathBuf,
    },
}

impl InstanceLocation {
    pub fn client(
        minecraft_directory: PathBuf,
        game_directory: PathBuf,
    ) -> Result<Self, LauncherError> {
        let location = Self::Client {
            minecraft_directory,
            game_directory,
        };
        location
            .validate()
            .map_err(LauncherError::InvalidRegistry)?;
        Ok(location)
    }

    pub fn server(server_directory: PathBuf) -> Result<Self, LauncherError> {
        let location = Self::Server { server_directory };
        location
            .validate()
            .map_err(LauncherError::InvalidRegistry)?;
        Ok(location)
    }

    pub fn import(instance_directory: &Path, kind: InstanceKind) -> Result<Self, LauncherError> {
        match kind {
            InstanceKind::Client => {
                let game_directory = dunce::canonicalize(instance_directory)?;
                let versions = game_directory.parent().ok_or_else(|| {
                    LauncherError::InvalidRegistry(format!(
                        "client game directory '{}' has no versions parent",
                        game_directory.display()
                    ))
                })?;
                if versions.file_name().and_then(|value| value.to_str()) != Some(VERSIONS_DIRECTORY)
                {
                    return Err(LauncherError::InvalidRegistry(format!(
                        "client game directory '{}' must be an immediate child of a 'versions' directory",
                        game_directory.display()
                    )));
                }
                let minecraft_directory = versions.parent().ok_or_else(|| {
                    LauncherError::InvalidRegistry(format!(
                        "client versions directory '{}' has no Minecraft repository parent",
                        versions.display()
                    ))
                })?;
                Self::client(minecraft_directory.to_path_buf(), game_directory)
            }
            InstanceKind::Server => Self::server(dunce::canonicalize(instance_directory)?),
        }
    }

    pub const fn kind(&self) -> InstanceKind {
        match self {
            Self::Client { .. } => InstanceKind::Client,
            Self::Server { .. } => InstanceKind::Server,
        }
    }

    pub fn instance_directory(&self) -> &Path {
        match self {
            Self::Client { game_directory, .. } => game_directory,
            Self::Server { server_directory } => server_directory,
        }
    }

    pub fn artifact_directory(&self) -> &Path {
        match self {
            Self::Client {
                minecraft_directory,
                ..
            } => minecraft_directory,
            Self::Server { server_directory } => server_directory,
        }
    }

    pub fn minecraft_directory(&self) -> Option<&Path> {
        match self {
            Self::Client {
                minecraft_directory,
                ..
            } => Some(minecraft_directory),
            Self::Server { .. } => None,
        }
    }

    pub fn game_directory(&self) -> Option<&Path> {
        match self {
            Self::Client { game_directory, .. } => Some(game_directory),
            Self::Server { .. } => None,
        }
    }

    pub fn instance_relative_path(&self, path: &str) -> Result<String, LauncherError> {
        match self {
            Self::Client {
                minecraft_directory,
                game_directory,
            } => {
                let relative = game_directory
                    .strip_prefix(minecraft_directory)
                    .map_err(|_| {
                        LauncherError::InvalidRegistry(format!(
                            "client game directory '{}' is outside Minecraft directory '{}'",
                            game_directory.display(),
                            minecraft_directory.display()
                        ))
                    })?;
                let prefix = crate::lockfile::portable_relative_path(relative)?;
                Ok(format!("{prefix}/{path}"))
            }
            Self::Server { .. } => Ok(path.to_string()),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Client {
                minecraft_directory,
                game_directory,
            } => {
                validate_absolute_directory(minecraft_directory, "Minecraft directory")?;
                validate_absolute_directory(game_directory, "client game directory")?;
                let expected_parent = minecraft_directory.join(VERSIONS_DIRECTORY);
                if game_directory.parent() != Some(expected_parent.as_path()) {
                    return Err(format!(
                        "client game directory '{}' must be an immediate child of '{}'",
                        game_directory.display(),
                        expected_parent.display()
                    ));
                }
                let name = game_directory
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default();
                validate_directory_name(name)?;
                Ok(())
            }
            Self::Server { server_directory } => {
                validate_absolute_directory(server_directory, "server directory")
            }
        }
    }
}

pub fn validate_directory_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.ends_with([' ', '.'])
        || name.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
        })
    {
        return Err(format!(
            "'{name}' is not a portable instance directory name"
        ));
    }
    let stem = name.split('.').next().unwrap_or_default();
    let upper = stem.to_ascii_uppercase();
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit()
            && upper.as_bytes()[3] != b'0')
    {
        return Err(format!("'{name}' is a reserved instance directory name"));
    }
    Ok(())
}

fn validate_absolute_directory(path: &Path, subject: &str) -> Result<(), String> {
    if !path.is_absolute() {
        return Err(format!("{subject} '{}' is not absolute", path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_layout_separates_repository_and_isolated_game_directory() {
        let base = if cfg!(windows) {
            PathBuf::from(r"C:\Games\.minecraft")
        } else {
            PathBuf::from("/games/.minecraft")
        };
        let game = base.join("versions").join("fabric-1.21.1");
        let location = InstanceLocation::client(base.clone(), game.clone()).unwrap();
        assert_eq!(location.artifact_directory(), base);
        assert_eq!(location.instance_directory(), game);
        assert_eq!(
            location.instance_relative_path("natives/x.dll").unwrap(),
            "versions/fabric-1.21.1/natives/x.dll"
        );
    }

    #[test]
    fn client_layout_rejects_a_flat_single_version_root() {
        let base = if cfg!(windows) {
            PathBuf::from(r"C:\Games\.minecraft")
        } else {
            PathBuf::from("/games/.minecraft")
        };
        assert!(InstanceLocation::client(base.clone(), base.join("client")).is_err());
    }
}
