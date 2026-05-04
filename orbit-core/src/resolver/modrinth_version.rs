//! Modrinth 版本解析器——使用 `date_published` 时间戳排序。

use super::provider_version::ProviderVersionResolver;
use crate::providers::ResolvedMod;

pub struct ModrinthVersionResolver;

impl ProviderVersionResolver for ModrinthVersionResolver {
    fn provider_name(&self) -> &str { "modrinth" }

    /// 按 date_published 降序（最新在前）。ISO 8601 格式天然可字符串排序。
    fn sort_newest_first(&self, versions: &mut [ResolvedMod]) {
        versions.sort_by(|a, b| b.date_published.cmp(&a.date_published));
    }

    /// Modrinth 约束：对 `modrinth_version` 字段做字符串匹配。
    /// 如果 `modrinth_version` 为空则回退到 `version` 字段的 SemVer 比较。
    fn satisfies(&self, version: &ResolvedMod, constraint: &str) -> bool {
        if constraint == "*" || constraint.is_empty() { return true; }
        let ver_str = match &version.modrinth {
            Some(m) if !m.version_number.is_empty() => &m.version_number,
            _ => &version.version,
        };
        if let Ok(sv) = crate::versions::fabric::SemanticVersion::parse(ver_str, true) {
            return crate::versions::fabric::satisfies(&sv, constraint);
        }
        ver_str == constraint
    }
}
