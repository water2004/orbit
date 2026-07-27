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
    instance_dir: &std::path::Path,
    lockfile: &OrbitLockfile,
    target_mc_version: &str,
    target_loader: &str,
    providers: &[Box<dyn ModProvider>],
    jar_cache: &crate::jar_cache::JarCache,
) -> Result<Vec<CheckResult>, OrbitError> {
    check_compatibility_with_progress(
        instance_dir,
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
    instance_dir: &std::path::Path,
    lockfile: &OrbitLockfile,
    target_mc_version: &str,
    target_loader: &str,
    providers: &[Box<dyn ModProvider>],
    jar_cache: &crate::jar_cache::JarCache,
    progress: Option<ProgressReporter>,
) -> Result<Vec<CheckResult>, OrbitError> {
    let target_loader = target_loader
        .parse::<crate::loader::LoaderKind>()
        .map_err(|message: String| OrbitError::Other(anyhow::anyhow!(message)))?;
    let catalog = crate::outdated::download_candidate_catalog(
        crate::outdated::CandidateDiscoveryInput {
            instance_dir,
            providers,
            additional_remotes: &[],
            lockfile,
            mc_version: target_mc_version,
            loader: target_loader.as_str(),
            jar_cache,
            progress,
        },
        &[],
    )
    .await?;
    let mut results = Vec::new();
    for entry in &lockfile.packages {
        if entry.remotes.is_empty() {
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
            provider: {
                let mut providers: Vec<_> = entry
                    .remotes
                    .iter()
                    .map(|remote| remote.provider())
                    .collect();
                providers.sort();
                providers.dedup();
                providers.join(", ")
            },
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
        let result = check_compatibility(
            std::path::Path::new("."),
            &lockfile,
            "2",
            "fabric",
            &[],
            &cache,
        )
        .await
        .unwrap();

        assert!(result.is_empty());
    }
}
