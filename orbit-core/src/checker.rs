//! 跨版本升级预检 (orbit check)。
//!
//! 检查当前已安装的模组集合是否已有目标 MC 版本的兼容版本。

use crate::error::OrbitError;
use crate::lockfile::OrbitLockfile;
use crate::progress::ProgressReporter;
use crate::providers::ModProvider;

/// 兼容性检查结果
#[derive(Debug, Clone)]
pub struct CheckResult {
    pub mod_name: String,
    pub current_version: String,
    pub provider: String,
    pub compatible: bool,
    pub available_version: Option<String>,
}

/// 检查所有已安装模组在目标 MC 版本下的兼容性。
pub async fn check_compatibility(
    lockfile: &OrbitLockfile,
    target_mc_version: &str,
    target_loader: &str,
    providers: &[Box<dyn ModProvider>],
    jar_cache: &crate::jar_cache::JarCache,
) -> Result<Vec<CheckResult>, OrbitError> {
    check_compatibility_with_progress(
        lockfile,
        target_mc_version,
        target_loader,
        providers,
        jar_cache,
        None,
    )
    .await
}

pub async fn check_compatibility_with_progress(
    lockfile: &OrbitLockfile,
    target_mc_version: &str,
    target_loader: &str,
    providers: &[Box<dyn ModProvider>],
    jar_cache: &crate::jar_cache::JarCache,
    progress: Option<ProgressReporter>,
) -> Result<Vec<CheckResult>, OrbitError> {
    let catalog = crate::outdated::download_lockfile_candidate_catalog(
        providers,
        lockfile,
        target_mc_version,
        target_loader,
        jar_cache,
        progress,
    )
    .await?;
    let mut results = Vec::new();
    for entry in &lockfile.packages {
        if entry.provider == "file" {
            continue;
        }
        if entry.source_project_id().is_none() {
            continue;
        }
        let available_version = catalog
            .candidates
            .get(&entry.mod_id)
            .and_then(|candidates| {
                candidates
                    .iter()
                    .max_by_key(|candidate| {
                        crate::versions::Version::parse(&candidate.jar_version, target_loader)
                    })
                    .map(|candidate| candidate.jar_version.clone())
            });
        results.push(CheckResult {
            mod_name: entry.mod_id.clone(),
            current_version: entry.version.clone(),
            provider: entry.provider.clone(),
            compatible: available_version.is_some(),
            available_version,
        });
    }
    results.sort_by(|left, right| left.mod_name.cmp(&right.mod_name));
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockfile::LockMeta;

    #[tokio::test]
    async fn local_file_packages_do_not_require_a_provider() {
        let lockfile = OrbitLockfile {
            meta: LockMeta {
                mc_version: "1".to_string(),
                modloader: "fabric".to_string(),
                modloader_version: "1".to_string(),
            },
            packages: Vec::new(),
        };

        let cache =
            crate::jar_cache::JarCache::open(std::env::temp_dir().join("orbit-check-local-test"))
                .unwrap();
        let result = check_compatibility(&lockfile, "2", "fabric", &[], &cache)
            .await
            .unwrap();

        assert!(result.is_empty());
    }
}
