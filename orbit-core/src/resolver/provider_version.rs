//! Provider 版本解析器——仅在 add 时处理 API 返回的依赖版本比较。
//!
//! 与 `versions/` 的字符串比较不同，provider resolver 使用 provider 特定逻辑：
//! - Modrinth: `date_published` 时间戳排序 + `version_number` 字符串约束
//! - 默认回退: SemanticVersion 字符串比较

use crate::providers::ResolvedMod;

/// Provider 版本比较与约束检查。
pub trait ProviderVersionResolver: Send + Sync {
    fn provider_name(&self) -> &str;

    /// 从最新到最旧排序（会修改传入的 slice）
    fn sort_newest_first(&self, versions: &mut [ResolvedMod]);

    /// 检查版本是否满足约束（provider 特定逻辑）
    fn satisfies(&self, version: &ResolvedMod, constraint: &str) -> bool;

    /// 从列表中选出满足约束的最新版本
    fn pick_best(&self, versions: &[ResolvedMod], constraint: &str) -> Option<ResolvedMod> {
        let mut candidates: Vec<&ResolvedMod> = versions
            .iter()
            .filter(|v| self.satisfies(v, constraint))
            .collect();
        // sort_newest_first needs mutable slice, clone to sort
        let mut cloned: Vec<ResolvedMod> = candidates.iter().map(|v| (*v).clone()).collect();
        self.sort_newest_first(&mut cloned);
        cloned.into_iter().next()
    }
}

/// 默认回退：字符串比较（用于未知 provider 或跨 provider 比较）
pub struct FallbackResolver;

impl ProviderVersionResolver for FallbackResolver {
    fn provider_name(&self) -> &str { "fallback" }

    fn sort_newest_first(&self, versions: &mut [ResolvedMod]) {
        versions.sort_by(|a, b| {
            let va = crate::versions::fabric::SemanticVersion::parse(&a.version, true);
            let vb = crate::versions::fabric::SemanticVersion::parse(&b.version, true);
            match (va, vb) {
                (Ok(sva), Ok(svb)) => svb.cmp(&sva),
                _ => b.version.cmp(&a.version),
            }
        });
    }

    fn satisfies(&self, version: &ResolvedMod, constraint: &str) -> bool {
        if constraint == "*" || constraint.is_empty() { return true; }
        if let Ok(sv) = crate::versions::fabric::SemanticVersion::parse(&version.version, true) {
            return crate::versions::fabric::satisfies(&sv, constraint);
        }
        version.version == constraint
    }
}
