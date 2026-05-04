//! 过时检查核心逻辑（纯函数，无 I/O）。
//!
//! `check_mod_outdated` 供 `outdated` 和 `upgrade` 命令复用。

use std::collections::HashSet;

use crate::providers::ResolvedMod;

#[derive(Debug, Clone)]
pub struct OutdatedMod {
    pub mod_id: String,
    pub current_version: String,
    /// 最新兼容版本（所有 required 依赖均在 lockfile 中存在）
    pub latest_compatible: Option<VersionInfo>,
    /// 最新版本（可能不兼容）
    pub latest_overall: Option<VersionInfo>,
}

#[derive(Debug, Clone)]
pub struct VersionInfo {
    pub version_number: String,
    pub date_published: String,
}

/// 对单个模组的版本列表判断是否有可用更新。
///
/// - `versions` 应为已过滤（mc_version + loader）并按 date_published 降序排列的列表
/// - `current_version_number` 为 lockfile 中记录的 modrinth.version（即 version_number）
/// - `installed_ids` 为当前 lockfile 中所有 mod_id 的集合（用于依赖兼容性检查）
pub fn check_mod_outdated(
    mod_id: &str,
    current_version_number: &str,
    versions: &[ResolvedMod],
    installed_ids: &HashSet<&str>,
) -> Option<OutdatedMod> {
    if versions.is_empty() {
        return None;
    }

    // 找到当前版本的 date_published
    let current_date = versions.iter()
        .find(|v| v.modrinth.as_ref()
            .map(|m| m.version_number.as_str() == current_version_number)
            .unwrap_or(false))
        .map(|v| v.date_published.clone());

    let newer: Vec<&ResolvedMod> = if let Some(ref cd) = current_date {
        versions.iter().filter(|v| v.date_published > *cd).collect()
    } else {
        // 找不到当前版本 → 全部视为候选
        versions.iter().collect()
    };

    if newer.is_empty() {
        return None;
    }

    let latest_overall = newer.first().map(|v| VersionInfo {
        version_number: v.modrinth.as_ref().map(|m| m.version_number.clone()).unwrap_or_default(),
        date_published: v.date_published.clone(),
    });

    let latest_compatible = newer.iter().find(|v| {
        v.dependencies.iter()
            .filter(|d| d.required)
            .all(|d| d.slug.as_ref().map(|s| installed_ids.contains(s.as_str())).unwrap_or(false))
    }).map(|v| VersionInfo {
        version_number: v.modrinth.as_ref().map(|m| m.version_number.clone()).unwrap_or_default(),
        date_published: v.date_published.clone(),
    });

    Some(OutdatedMod {
        mod_id: mod_id.to_string(),
        current_version: current_version_number.to_string(),
        latest_compatible,
        latest_overall,
    })
}
