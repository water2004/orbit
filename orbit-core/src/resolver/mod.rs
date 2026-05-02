//! 依赖解析引擎。
//!
//! 负责版本号归一化和 lock 条目生成。

pub mod version;

use indexmap::IndexMap;

use crate::identification::IdentifiedMod;
use crate::init::ScannedMod;
use crate::lockfile::{ImplantedMod, LockDependency, LockEntry};

/// 系统级依赖（不作为模组依赖处理）
const SYSTEM_DEPS: &[&str] = &["minecraft", "fabricloader", "java"];

/// 根据识别结果生成 lock 条目列表。
pub fn build_lock_entries(
    identified: &[IdentifiedMod],
    scanned: &[ScannedMod],
    embedded: &[IdentifiedMod],
) -> (Vec<LockEntry>, Vec<String>) {
    // 构建查找索引：project_id / slug / mod_id / mod_name / filename → 已安装模组信息
    // needed because API deps use project IDs (e.g. P7dR8mSH) while JAR deps use slugs (e.g. fabric-api)
    #[derive(Clone)]
    struct DepInfo {
        name: String,
        version: String,
    }
    let installed: IndexMap<String, DepInfo> = identified
        .iter()
        .flat_map(|m| {
            let info = DepInfo {
                name: if m.mod_name.is_empty() { m.mod_id.clone() } else { m.mod_name.clone() },
                version: m.version.clone(),
            };
            let mut keys = vec![m.mod_id.clone(), m.mod_name.clone(), m.filename.clone()];
            // Also index by platform project ID (e.g. P7dR8mSH for fabric-api)
            if let crate::identification::IdentifiedSource::Platform { ref project_id, ref slug, .. } = m.source {
                keys.push(slug.clone());
                keys.push(project_id.clone());
            }
            keys.into_iter().filter(|k| !k.is_empty()).map(move |k| (k, info.clone()))
        })
        .collect();

    let mut warnings = vec![];
    let mut entries: Vec<LockEntry> = identified
        .iter()
        .map(|m| {
            let mut entry = LockEntry {
                name: if m.mod_name.is_empty() { m.mod_id.clone() } else { m.mod_name.clone() },
                version: m.version.clone(),
                filename: m.filename.clone(),
                sha256: m.sha256.clone(),
                dependencies: vec![],
                implanted: vec![],
                platform: None,
                mod_id: None,
                url: None,
                source_type: None,
                path: None,
            };

            match &m.source {
                crate::identification::IdentifiedSource::Platform { platform, project_id, .. } => {
                    entry.platform = Some(platform.clone());
                    entry.mod_id = Some(project_id.clone());
                }
                crate::identification::IdentifiedSource::File { path } => {
                    entry.source_type = Some("file".into());
                    entry.path = Some(path.clone());
                }
            }

            for (dep_id, constraint, is_required) in &m.deps {
                if SYSTEM_DEPS.contains(&dep_id.as_str()) {
                    eprintln!("    ↳ depends on {dep_id} {constraint} (system, skipped)");
                    continue;
                }

                if let Some(dep) = installed.get(dep_id) {
                    if version_satisfies(&dep.version, constraint) {
                        entry.dependencies.push(LockDependency {
                            name: dep.name.clone(),
                            version: dep.version.clone(),
                        });
                    } else {
                        let msg = format!(
                            "  ✗ {} requires {dep_id} {constraint} but version {} is installed",
                            entry.name, dep.version
                        );
                        eprintln!("{msg}");
                        warnings.push(msg);
                    }
                } else if *is_required {
                    let msg = format!(
                        "  ⚠ {} depends on '{dep_id}' ({constraint}) which is not installed",
                        entry.name
                    );
                    eprintln!("{msg}");
                    warnings.push(msg);
                } else {
                    eprintln!("    ↳ optional dep {dep_id} not installed, skipped");
                }
            }

            entry
        })
        .collect();

    // 填充 implanted：将内嵌子模组归入父模组
    for m in embedded {
        let Some(sm) = scanned.iter().find(|s| s.filename == m.filename) else { continue };
        let Some(ref parent_name) = sm.embedded_parent else { continue };

        if let Some(parent_entry) = entries.iter_mut().find(|e| e.filename == *parent_name) {
            parent_entry.implanted.push(ImplantedMod {
                name: if m.mod_name.is_empty() { m.mod_id.clone() } else { m.mod_name.clone() },
                version: m.version.clone(),
                sha256: m.sha256.clone(),
                filename: m.filename.clone(),
            });
        }
    }

    (entries, warnings)
}

/// 简单版本约束检查：支持 * / =x.y / >=x.y / >x.y / <x.y / ~x.y
fn version_satisfies(installed: &str, constraint: &str) -> bool {
    let constraint = constraint.trim();
    if constraint == "*" || constraint.is_empty() {
        return true;
    }
    let installed = version::NormalizedVersion::new(installed);

    if let Some(v) = constraint.strip_prefix(">=") {
        installed >= version::NormalizedVersion::new(v.trim())
    } else if let Some(v) = constraint.strip_prefix('>') {
        installed > version::NormalizedVersion::new(v.trim())
    } else if let Some(v) = constraint.strip_prefix("<=") {
        installed <= version::NormalizedVersion::new(v.trim())
    } else if let Some(v) = constraint.strip_prefix('<') {
        installed < version::NormalizedVersion::new(v.trim())
    } else if let Some(v) = constraint.strip_prefix("~") {
        installed >= version::NormalizedVersion::new(v.trim())
    } else if let Some(v) = constraint.strip_prefix('=') {
        installed == version::NormalizedVersion::new(v.trim())
    } else {
        // 无前缀 → 精确匹配或尝试 semver
        installed == version::NormalizedVersion::new(constraint)
    }
}
