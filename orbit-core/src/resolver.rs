//! 依赖解析引擎。
//!
//! 负责 lock 条目生成和依赖校验。

use indexmap::IndexMap;

use crate::identification::IdentifiedMod;
use crate::init::ScannedMod;
use crate::lockfile::{ImplantedMod, LockDependency, LockEntry};

/// 各 loader 内嵌的依赖（不检查）
fn embedded_deps(loader: &str) -> &'static [&'static str] {
    match loader {
        "fabric" => &["mixinextras", "java"],
        _ => &["java"],
    }
}

/// 各 loader 的环境级虚拟依赖（版本来自 init 检测）
fn env_deps<'a>(loader: &str, mc_ver: &'a str, loader_ver: &'a str) -> Vec<(&'a str, &'a str)> {
    match loader {
        "fabric" => vec![("minecraft", mc_ver), ("fabricloader", loader_ver)],
        _ => vec![("minecraft", mc_ver)],
    }
}

pub fn build_lock_entries(
    identified: &[IdentifiedMod],
    scanned: &[ScannedMod],
    embedded: &[IdentifiedMod],
    loader: &str,
    mc_version: &str,
    loader_version: &str,
) -> (Vec<LockEntry>, Vec<String>) {
    // 构建查找索引：project_id / slug / mod_id / mod_name / filename → 已安装模组信息
    // needed because API deps use project IDs (e.g. P7dR8mSH) while JAR deps use slugs (e.g. fabric-api)
    #[derive(Clone)]
    struct DepInfo {
        name: String,
        version: String,
    }
    let mut installed: IndexMap<String, DepInfo> = identified
        .iter()
        .chain(embedded.iter())
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

    // 注入环境依赖（根据 loader 类型）
    for (name, version) in env_deps(loader, mc_version, loader_version) {
        installed.entry(name.to_string()).or_insert(DepInfo { name: name.to_string(), version: version.to_string() });
    }

    let mut warnings = vec![];
    let mut entries: Vec<LockEntry> = identified
        .iter()
        .map(|m| {
            let mut entry = LockEntry {
                name: if m.mod_name.is_empty() { m.mod_id.clone() } else { m.mod_name.clone() },
                version: m.version.clone(),
                filename: m.filename.clone(),
                sha256: m.sha256.clone(),
                sha512: String::new(),
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
                if embedded_deps(loader).contains(&dep_id.as_str()) {
                    continue;
                }

                if let Some(dep) = installed.get(dep_id) {
                    if constraint.is_empty() || version_satisfies(&dep.version, constraint) {
                        entry.dependencies.push(LockDependency {
                            name: dep.name.clone(),
                            version: if constraint.is_empty() { dep.version.clone() } else { constraint.to_string() },
                        });
                    } else {
                        warnings.push(format!(
                            "  ✗ {} requires {dep_id} {constraint} but version {} is installed",
                            entry.name, dep.version
                        ));
                    }
                } else if *is_required {
                    warnings.push(format!(
                        "  ⚠ {} depends on '{dep_id}' ({constraint}) which is not installed",
                        entry.name
                    ));
                }
            }

            entry
        })
        .collect();

    // 填充 implanted：将内嵌子模组归入父模组。
    // 注意：多个父 JAR 可能内嵌同名子 JAR（如 conditional-mixin），
    // scanned 中会有多条 filename 相同但 embedded_parent 不同的条目。
    // 必须按 (filename, embedded_parent) 精确匹配，避免重复归入同一父模组。
    for m in embedded {
        // 在 scanned 中找到对应的 ScannedMod，用 sha256 精确匹配（因为同名 JAR 可能来自不同父模组但内容相同）
        let matching_parents: Vec<&str> = scanned.iter()
            .filter(|s| s.filename == m.filename && s.embedded_parent.is_some())
            .filter_map(|s| s.embedded_parent.as_deref())
            .collect();

        // 去重：只归入一次（相同内容的内嵌 JAR 只需在第一个匹配的父模组中出现）
        for parent_name in &matching_parents {
            if let Some(parent_entry) = entries.iter_mut().find(|e| e.filename == *parent_name) {
                // 检查是否已存在相同 filename 的 implanted 条目
                if parent_entry.implanted.iter().any(|imp| imp.filename == m.filename) {
                    continue;
                }
                parent_entry.implanted.push(ImplantedMod {
                    name: if m.mod_name.is_empty() { m.mod_id.clone() } else { m.mod_name.clone() },
                    version: m.version.clone(),
                    sha256: m.sha256.clone(),
                    filename: m.filename.clone(),
                });
            }
        }
    }

    (entries, warnings)
}

fn version_satisfies(installed: &str, constraint: &str) -> bool {
    let Ok(ver) = crate::versions::fabric::SemanticVersion::parse(installed, true) else { return false; };
    crate::versions::fabric::satisfies(&ver, constraint)
}

/// 从 lockfile 的依赖图中反查：哪些模组依赖了 `slug`。
pub fn dependents<'a>(slug: &str, entries: &'a [LockEntry]) -> Vec<&'a str> {
    entries
        .iter()
        .filter(|e| e.dependencies.iter().any(|d| d.name == slug))
        .map(|e| e.name.as_str())
        .collect()
}

/// 在 lockfile 中按 slug 查找条目。
pub fn find_entry<'a>(slug: &str, entries: &'a [LockEntry]) -> Option<&'a LockEntry> {
    entries.iter().find(|e| e.name == slug || e.mod_id.as_deref() == Some(slug))
}

/// 检查新版本是否与 lockfile 中已有版本冲突。
pub fn check_version_conflict(slug: &str, new_version: &str, entries: &[LockEntry]) -> Result<(), String> {
    if let Some(entry) = find_entry(slug, entries) {
        if entry.version != new_version {
            return Err(format!(
                "'{}' version conflict: lock has '{}', resolved '{}'",
                entry.name, entry.version, new_version
            ));
        }
    }
    Ok(())
}
