//! 依赖解析引擎。
//!
//! 负责版本号归一化和 lock 条目生成。

pub mod version;

use indexmap::IndexMap;

use crate::identification::IdentifiedMod;
use crate::init::ScannedMod;
use crate::lockfile::{LockDependency, LockEntry};

/// 系统级依赖（不作为模组依赖处理）
const SYSTEM_DEPS: &[&str] = &["minecraft", "fabricloader", "java"];

/// 根据识别结果生成 lock 条目列表。
///
/// 依赖校验：
/// - 跳过系统依赖（minecraft / fabricloader / java）
/// - 检查每个声明依赖是否在磁盘上存在，且版本满足约束
/// - 版本不满足 → error
/// - 依赖不存在 → warning（只记录存在的）
pub fn build_lock_entries(
    identified: &[IdentifiedMod],
    scanned: &[ScannedMod],
) -> (Vec<LockEntry>, Vec<String>) {
    let installed: IndexMap<String, &ScannedMod> = scanned
        .iter()
        .flat_map(|s| {
            let mut keys = vec![];
            if let Some(ref id) = s.mod_id { keys.push(id.clone()); }
            if let Some(ref name) = s.mod_name { keys.push(name.clone()); }
            keys.push(s.filename.clone());
            keys.into_iter().map(move |k| (k, s))
        })
        .collect();

    let mut warnings = vec![];
    let entries = identified
        .iter()
        .map(|m| {
            let mut entry = LockEntry {
                name: if m.mod_name.is_empty() { m.mod_id.clone() } else { m.mod_name.clone() },
                version: m.version.clone(),
                filename: m.filename.clone(),
                sha256: m.sha256.clone(),
                dependencies: vec![],
                platform: None,
                mod_id: None,
                url: None,
                source_type: None,
                path: None,
            };

            match &m.source {
                crate::identification::IdentifiedSource::Platform { platform, slug } => {
                    entry.platform = Some(platform.clone());
                    entry.mod_id = Some(slug.clone());
                }
                crate::identification::IdentifiedSource::File { path } => {
                    entry.source_type = Some("file".into());
                    entry.path = Some(path.clone());
                }
            }

            for (dep_id, constraint) in &m.deps {
                // 跳过系统依赖
                if SYSTEM_DEPS.contains(&dep_id.as_str()) {
                    eprintln!("    ↳ depends on {dep_id} {constraint} (system, skipped)");
                    continue;
                }

                if let Some(dep) = installed.get(dep_id) {
                    let dep_ver = dep.version.as_deref().unwrap_or("?");
                    if version_satisfies(dep_ver, constraint) {
                        entry.dependencies.push(LockDependency {
                            name: dep.mod_name.clone().unwrap_or_else(|| dep.filename.clone()),
                            version: dep.version.clone().unwrap_or_default(),
                        });
                    } else {
                        let msg = format!(
                            "  ✗ {} requires {dep_id} {constraint} but version {dep_ver} is installed",
                            entry.name
                        );
                        eprintln!("{msg}");
                        warnings.push(msg);
                    }
                } else {
                    let msg = format!(
                        "  ⚠ {} depends on '{dep_id}' ({constraint}) which is not installed",
                        entry.name
                    );
                    eprintln!("{msg}");
                    warnings.push(msg);
                }
            }

            entry
        })
        .collect();

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
