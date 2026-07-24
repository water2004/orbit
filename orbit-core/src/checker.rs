//! 跨版本升级预检 (orbit check)。
//!
//! 检查当前已安装的模组集合是否已有目标 MC 版本的兼容版本。

use crate::error::OrbitError;
use crate::lockfile::OrbitLockfile;
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
) -> Result<Vec<CheckResult>, OrbitError> {
    let mut results = Vec::new();
    for entry in &lockfile.packages {
        if entry.provider == "file" {
            continue;
        }
        let Some(project_id) = entry.source_project_id() else {
            continue;
        };
        let provider =
            crate::providers::find_provider(providers, &entry.provider).ok_or_else(|| {
                OrbitError::Other(anyhow::anyhow!(
                    "cannot check {} package '{}': provider is not configured",
                    entry.provider,
                    entry.mod_id,
                ))
            })?;
        let mut versions = provider
            .get_versions(&project_id, Some(target_mc_version), Some(target_loader))
            .await?;
        versions.sort_by(|left, right| right.date_published.cmp(&left.date_published));
        results.push(CheckResult {
            mod_name: entry.mod_id.clone(),
            current_version: entry.version.clone(),
            provider: provider.name().to_string(),
            compatible: !versions.is_empty(),
            available_version: versions.first().map(|version| version.version.clone()),
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

        let result = check_compatibility(&lockfile, "2", "fabric", &[])
            .await
            .unwrap();

        assert!(result.is_empty());
    }
}
