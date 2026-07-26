//! 深度清理启发式搜索。
//!
//! 按模组名称/slug 匹配 config/ 目录下的候选配置文件。

use crate::error::OrbitError;

/// 候选配置文件
#[derive(Debug, Clone)]
pub struct CandidateConfig {
    pub path: String,
    pub reason: String,
}

/// 启发式搜索 config/ 目录中与指定模组相关的配置文件。
pub fn find_config_candidates(
    mod_name: &str,
    mod_slug: Option<&str>,
    config_dir: &std::path::Path,
) -> Result<Vec<CandidateConfig>, OrbitError> {
    if !config_dir.exists() {
        return Ok(Vec::new());
    }
    let normalized_name = normalize(mod_name);
    let normalized_slug = mod_slug.map(normalize).filter(|slug| !slug.is_empty());
    let mut candidates = Vec::new();
    let mut pending = vec![config_dir.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                pending.push(entry.path());
                continue;
            }
            if !metadata.is_file() {
                continue;
            }
            let entry_path = entry.path();
            let relative = entry_path
                .strip_prefix(config_dir)
                .unwrap_or(entry_path.as_path())
                .to_string_lossy()
                .into_owned();
            let normalized_path = normalize(&relative);
            let reason =
                if !normalized_name.is_empty() && normalized_path.contains(&normalized_name) {
                    Some(format!("path matches mod id '{mod_name}'"))
                } else if normalized_slug
                    .as_ref()
                    .is_some_and(|slug| normalized_path.contains(slug))
                {
                    Some(format!(
                        "path matches slug '{}'",
                        mod_slug.unwrap_or_default()
                    ))
                } else {
                    None
                };
            if let Some(reason) = reason {
                candidates.push(CandidateConfig {
                    path: entry_path.to_string_lossy().into_owned(),
                    reason,
                });
            }
        }
    }
    candidates.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(candidates)
}

pub fn remove_config_candidates(
    config_dir: &std::path::Path,
    candidates: &[CandidateConfig],
) -> Result<Vec<String>, OrbitError> {
    if !config_dir.exists() {
        return Ok(Vec::new());
    }
    let root = config_dir.canonicalize()?;
    let mut removed = Vec::new();
    for candidate in candidates {
        let path = std::path::PathBuf::from(&candidate.path);
        let resolved = path.canonicalize()?;
        if !resolved.starts_with(&root) || !resolved.is_file() {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "refusing to remove config outside '{}': {}",
                root.display(),
                resolved.display()
            )));
        }
        std::fs::remove_file(&resolved)?;
        removed.push(resolved.to_string_lossy().into_owned());
    }
    Ok(removed)
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("orbit-purge-test-{name}-{}", std::process::id()))
    }

    #[test]
    fn finds_normalized_name_and_slug_matches() {
        let directory = test_dir("find");
        std::fs::create_dir_all(directory.join("voxel-map")).unwrap();
        std::fs::write(directory.join("voxel_map.toml"), b"").unwrap();
        std::fs::write(directory.join("voxel-map").join("waypoints.db"), b"").unwrap();
        std::fs::write(directory.join("unrelated.toml"), b"").unwrap();

        let candidates = find_config_candidates("voxelmap", Some("voxel-map"), &directory).unwrap();

        assert_eq!(candidates.len(), 2);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn removes_only_candidates_below_config_root() {
        let directory = test_dir("remove");
        std::fs::create_dir_all(&directory).unwrap();
        let target = directory.join("example.toml");
        std::fs::write(&target, b"").unwrap();
        let candidates = vec![CandidateConfig {
            path: target.to_string_lossy().into_owned(),
            reason: "test".to_string(),
        }];

        let removed = remove_config_candidates(&directory, &candidates).unwrap();

        assert_eq!(removed.len(), 1);
        assert!(!target.exists());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
