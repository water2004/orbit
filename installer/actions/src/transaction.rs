use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub source: PathBuf,
    pub staged: PathBuf,
}

/// Select owned subtrees without ever recursing into a protected instance.
/// Only paths on the boundary between settings and game data are enumerated.
pub fn partition(root: &Path, protected: &[PathBuf], output: &mut Vec<PathBuf>) -> Result<()> {
    let Some(metadata) = metadata(root)? else {
        return Ok(());
    };
    check_root(root)?;
    let canonical = dunce::canonicalize(root)?;
    let root = canonical.as_path();
    if protected.iter().any(|keep| root.starts_with(keep)) {
        return Ok(());
    }
    if protected.iter().any(|keep| keep.starts_with(root)) {
        if !metadata.is_dir() {
            return Err("a protected instance overlaps a settings file".into());
        }
        for entry in std::fs::read_dir(root)? {
            partition(&entry?.path(), protected, output)?;
        }
    } else {
        output.push(root.to_path_buf());
    }
    Ok(())
}

pub fn metadata(path: &Path) -> Result<Option<std::fs::Metadata>> {
    match std::fs::symlink_metadata(path) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub fn check_root(path: &Path) -> Result<()> {
    if !path.is_absolute() || path.parent().is_none() || path.components().count() < 3 {
        return Err(format!(
            "refusing broad or relative cleanup target '{}'",
            path.display()
        )
        .into());
    }
    if let Some(metadata) = metadata(path)? {
        #[cfg(windows)]
        let redirected = {
            use std::os::windows::fs::MetadataExt;
            metadata.file_attributes() & 0x400 != 0
        };
        #[cfg(not(windows))]
        let redirected = metadata.file_type().is_symlink();
        let canonical = dunce::canonicalize(path)?;
        #[cfg(windows)]
        let same_path = canonical.components().count() == path.components().count()
            && canonical
                .components()
                .zip(path.components())
                .all(|(left, right)| {
                    left.as_os_str()
                        .to_string_lossy()
                        .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
                });
        #[cfg(not(windows))]
        let same_path = canonical == path;
        if redirected || !same_path {
            return Err(format!(
                "refusing redirected or non-canonical cleanup target '{}'",
                path.display()
            )
            .into());
        }
    }
    Ok(())
}

pub fn entries(mut paths: Vec<PathBuf>, forbidden: &[PathBuf]) -> Result<Vec<Entry>> {
    paths.sort();
    paths.dedup();
    let mut selected: Vec<PathBuf> = Vec::new();
    for path in paths {
        check_target(&path, forbidden)?;
        if !selected.iter().any(|root| path.starts_with(root)) {
            selected.push(path);
        }
    }
    let transaction = uuid::Uuid::new_v4();
    Ok(selected
        .into_iter()
        .enumerate()
        .map(|(index, source)| Entry {
            staged: source.with_file_name(format!(".orbit-uninstall-{transaction}-{index}")),
            source,
        })
        .collect())
}

pub fn check_target(path: &Path, forbidden: &[PathBuf]) -> Result<()> {
    check_root(path)?;
    let normalized = if metadata(path)?.is_some() {
        dunce::canonicalize(path)?
    } else {
        path.into()
    };
    for root in forbidden {
        let root = if metadata(root)?.is_some() {
            dunce::canonicalize(root)?
        } else {
            root.clone()
        };
        if !root.starts_with(&normalized) {
            continue;
        }
        return Err(format!(
            "cleanup target '{}' contains a protected system/storage directory",
            path.display()
        )
        .into());
    }
    Ok(())
}

fn validate_entry(entry: &Entry) -> Result<()> {
    check_root(&entry.source)?;
    if entry.source.parent() != entry.staged.parent()
        || !entry
            .staged
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with(".orbit-uninstall-"))
    {
        return Err("invalid uninstall recovery path".into());
    }
    Ok(())
}

pub fn stage(entries: &[Entry]) -> Result<()> {
    // Preflight all targets before changing the first one.
    for entry in entries {
        validate_entry(entry)?;
        if metadata(&entry.staged)?.is_some() {
            return Err("uninstall recovery path already exists".into());
        }
        if metadata(&entry.source)?.is_none() {
            return Err(format!("cleanup target disappeared: {}", entry.source.display()).into());
        }
    }
    for entry in entries {
        if let Err(error) = std::fs::rename(&entry.source, &entry.staged) {
            let recovered = rollback(entries);
            return Err(format!(
                "cannot stage '{}': {error}; rollback: {recovered:?}",
                entry.source.display()
            )
            .into());
        }
    }
    Ok(())
}

pub fn rollback(entries: &[Entry]) -> Result<()> {
    let mut errors = Vec::new();
    for entry in entries.iter().rev() {
        if let Err(error) = (|| -> Result<()> {
            validate_entry(entry)?;
            if metadata(&entry.staged)?.is_some() {
                check_root(&entry.staged)?;
                if metadata(&entry.source)?.is_some() {
                    return Err(format!(
                        "cannot overwrite recreated '{}'; recovery: '{}'",
                        entry.source.display(),
                        entry.staged.display()
                    )
                    .into());
                }
                std::fs::rename(&entry.staged, &entry.source)?;
            }
            Ok(())
        })() {
            errors.push(error.to_string());
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; ").into())
    }
}

pub fn commit(entries: &[Entry]) -> Result<()> {
    for entry in entries {
        validate_entry(entry)?;
        if let Some(metadata) = metadata(&entry.staged)? {
            check_root(&entry.staged)?;
            // std::fs does not follow nested directory symlinks during removal.
            let result = if metadata.is_dir() {
                std::fs::remove_dir_all(&entry.staged)
            } else {
                std::fs::remove_file(&entry.staged)
            };
            result.map_err(|error| {
                format!(
                    "cannot finish cleanup; retained recovery path '{}': {error}",
                    entry.staged.display()
                )
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redirected_roots_are_rejected_and_nested_links_never_delete_the_target() {
        let temp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(temp.path()).unwrap();
        let outside = root.join("keep");
        let owned = root.join("owned");
        std::fs::create_dir(&outside).unwrap();
        std::fs::create_dir(&owned).unwrap();
        std::fs::write(outside.join("world"), "keep").unwrap();
        let link = owned.join("link");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let output = std::process::Command::new("cmd.exe")
                .args(["/c", "mklink", "/J"])
                .arg(&link)
                .arg(&outside)
                .creation_flags(0x08000000)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        assert!(check_root(&link).is_err());
        let plan = entries(vec![owned], &[]).unwrap();
        stage(&plan).unwrap();
        commit(&plan).unwrap();
        assert_eq!(
            std::fs::read_to_string(outside.join("world")).unwrap(),
            "keep"
        );
    }
    #[test]
    fn settings_partition_never_traverses_game_data() {
        let temp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(temp.path()).unwrap();
        let games = root.join("minecraft/instances");
        std::fs::create_dir_all(games.join("world/saves")).unwrap();
        std::fs::write(games.join("world/saves/level.dat"), "world").unwrap();
        std::fs::create_dir_all(root.join("minecraft/assets")).unwrap();
        std::fs::write(root.join("config.toml"), "settings").unwrap();
        let mut paths = Vec::new();
        partition(&root, std::slice::from_ref(&games), &mut paths).unwrap();
        assert_eq!(paths.len(), 2);
        let entries = entries(paths, &[]).unwrap();
        stage(&entries).unwrap();
        commit(&entries).unwrap();
        assert!(games.join("world/saves/level.dat").is_file());
        assert!(!root.join("config.toml").exists());
        assert!(!root.join("minecraft/assets").exists());
    }
    #[test]
    fn rollback_restores_exact_bytes_and_commit_removes_only_confirmed_targets() {
        let temp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(temp.path()).unwrap();
        let target = root.join("instance");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("world"), [1, 2, 3]).unwrap();
        let plan = entries(vec![target.clone()], &[]).unwrap();
        stage(&plan).unwrap();
        assert!(!target.exists());
        rollback(&plan).unwrap();
        assert_eq!(std::fs::read(target.join("world")).unwrap(), [1, 2, 3]);
        stage(&plan).unwrap();
        commit(&plan).unwrap();
        assert!(!target.exists());
        assert!(root.exists());
    }
    #[test]
    fn changed_targets_are_rejected_before_staging_and_rollback_never_overwrites() {
        let temp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(temp.path()).unwrap();
        let a = root.join("a");
        let b = root.join("b");
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();
        let plan = entries(vec![a.clone(), b.clone()], &[]).unwrap();
        std::fs::remove_file(&b).unwrap();
        assert!(stage(&plan).is_err());
        assert!(a.exists());
        let plan = entries(vec![a.clone()], &[]).unwrap();
        stage(&plan).unwrap();
        std::fs::write(&a, "new").unwrap();
        assert!(rollback(&plan).is_err());
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "new");
        assert!(plan[0].staged.exists());
    }
    #[test]
    fn refuses_protected_ancestors_and_relative_paths() {
        assert!(entries(vec![PathBuf::from("relative")], &[]).is_err());
        let temp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(temp.path()).unwrap();
        assert!(entries(vec![root.clone()], &[root.join("protected")]).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn occupied_file_rolls_back_earlier_targets() {
        use std::os::windows::fs::OpenOptionsExt;
        let temp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(temp.path()).unwrap();
        let a = root.join("a");
        let b = root.join("b");
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();
        let plan = entries(vec![a.clone(), b.clone()], &[]).unwrap();
        let _lock = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&b)
            .unwrap();
        assert!(stage(&plan).is_err());
        assert!(a.exists());
        assert!(b.exists());
    }
}
