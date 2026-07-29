//! Fabric SemanticVersion — 1:1 复刻 Fabric Loader 的版本比较逻辑。
//!
//! 对应 fabric-loader: SemanticVersionImpl.java + VersionPredicateParser.java
//!
//! 关键规则：
//! - `+` 之后是 build metadata，完全忽略
//! - `-` 之后是 prerelease，使版本降级（1.0-alpha < 1.0）
//! - `x`/`X`/`*` 在末位是通配符
//! - 缺少的 component 默认 0，通配符则延续通配符
//! - 复合约束按空格拆分，全部满足才算通过

use std::cmp::Ordering;
use std::hash::{Hash, Hasher};

use super::CorePosition;

// ═══════════════════════════════════════════════════════════════
// SemanticVersion — 对应 Fabric 的 SemanticVersionImpl
// ═══════════════════════════════════════════════════════════════

pub const WILDCARD: i32 = i32::MIN;

#[derive(Debug, Clone)]
pub struct SemanticVersion {
    pub raw: String,
    /// 数字组件（不含通配符），长度至少 1
    pub components: Vec<i32>,
    /// prerelease 后缀（`-` 之后），None 表示正式版
    pub prerelease: Option<String>,
    /// build 后缀（`+` 之后），比较时忽略
    #[allow(dead_code)]
    pub build: Option<String>,
    /// 是否有通配符
    pub has_wildcard: bool,
    position: CorePosition,
}

impl SemanticVersion {
    pub fn parse(raw: &str, store_x: bool) -> Result<Self, String> {
        let mut version = raw.to_string();
        // ── build  ──
        let build = if let Some(pos) = version.find('+') {
            let b = version[pos + 1..].to_string();
            version = version[..pos].to_string();
            Some(b)
        } else {
            None
        };
        // ── prerelease ──
        let prerelease = if let Some(pos) = version.find('-') {
            let p = version[pos + 1..].to_string();
            version = version[..pos].to_string();
            if !is_dot_separated_id(&p) {
                return Err(format!("invalid prerelease string '{p}'"));
            }
            Some(p)
        } else {
            None
        };

        if version.ends_with('.') {
            return Err("negative version component".into());
        }
        if version.starts_with('.') {
            return Err("missing version component".into());
        }

        let comp_strs: Vec<&str> = version.split('.').collect();
        if comp_strs.is_empty() {
            return Err("no version numbers".into());
        }
        let mut components = vec![0i32; comp_strs.len()];
        let mut first_wildcard: Option<usize> = None;
        let mut has_wildcard = false;

        for (i, cs) in comp_strs.iter().enumerate() {
            if store_x && (*cs == "x" || *cs == "X" || *cs == "*") {
                if prerelease.is_some() {
                    return Err("pre-release with X-range not allowed".into());
                }
                components[i] = WILDCARD;
                has_wildcard = true;
                if first_wildcard.is_none() {
                    first_wildcard = Some(i);
                }
                if i > 0 && components[i - 1] == WILDCARD {
                    // already wildcard, keep going
                }
            } else {
                let trimmed = cs.trim();
                if trimmed.is_empty() {
                    return Err("missing version component".into());
                }
                components[i] = trimmed
                    .parse::<i32>()
                    .map_err(|_| format!("invalid component '{cs}'"))?;
                if components[i] < 0 {
                    return Err(format!("negative component '{cs}'"));
                }
            }
        }

        if store_x && components.len() == 1 && components[0] == WILDCARD {
            return Err("version 'x' not allowed".into());
        }
        // strip extra wildcards: 1.x.x → 1.x
        if let Some(fw) = first_wildcard
            && fw > 0
            && components.len() > fw + 1
        {
            components.truncate(fw + 1);
        }

        Ok(Self {
            raw: raw.to_string(),
            components,
            prerelease,
            build,
            has_wildcard,
            position: CorePosition::Concrete,
        })
    }

    fn component(&self, pos: usize) -> i32 {
        if pos >= self.components.len() {
            if self.has_wildcard { WILDCARD } else { 0 }
        } else {
            self.components[pos]
        }
    }

    pub fn bump(&self) -> Self {
        let mut new_v = self.clone();
        if let Some(last) = new_v
            .components
            .iter_mut()
            .filter(|x| **x != WILDCARD)
            .last()
        {
            *last = last.saturating_add(1);
        } else {
            new_v.components.push(1);
        }
        new_v.prerelease = None;
        new_v.position = CorePosition::Concrete;
        new_v.raw = format!("{}.bump", new_v.raw);
        new_v
    }

    fn before_core(&self) -> Self {
        let mut boundary = self.clone();
        boundary.raw = format!("{}-core-lower", self.core_display());
        boundary.prerelease = None;
        boundary.build = None;
        boundary.has_wildcard = false;
        boundary.position = CorePosition::Before;
        boundary
    }

    fn after_core(&self) -> Self {
        let mut boundary = self.before_core();
        boundary.raw = format!("{}-core-upper", self.core_display());
        boundary.position = CorePosition::After;
        boundary
    }

    fn core_display(&self) -> String {
        self.components
            .iter()
            .map(i32::to_string)
            .collect::<Vec<_>>()
            .join(".")
    }

    pub(super) fn cmp_precedence(&self, other: &Self) -> Ordering {
        compare_components(self, other)
    }
}

pub(crate) fn wildcard_for_core_bounds(
    lower: &SemanticVersion,
    upper: &SemanticVersion,
) -> Option<String> {
    if lower.position != CorePosition::Before || upper.position != CorePosition::Before {
        return None;
    }
    let mut expected_upper = lower.components.clone();
    let last = expected_upper.last_mut()?;
    *last = last.saturating_add(1);
    if expected_upper != upper.components {
        return None;
    }
    Some(format!("{}.x", lower.core_display()))
}

impl PartialEq for SemanticVersion {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for SemanticVersion {}

impl Hash for SemanticVersion {
    fn hash<H: Hasher>(&self, state: &mut H) {
        canonical_components(self).hash(state);
        self.position.hash(state);
        if self.position == CorePosition::Concrete {
            self.prerelease.hash(state);
        }
    }
}

impl PartialOrd for SemanticVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SemanticVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        let core = compare_components(self, other);
        if core != Ordering::Equal {
            return core;
        }
        match (self.position, other.position) {
            (CorePosition::Before, CorePosition::Before)
            | (CorePosition::After, CorePosition::After) => return Ordering::Equal,
            (CorePosition::Before, _) | (_, CorePosition::After) => return Ordering::Less,
            (CorePosition::After, _) | (_, CorePosition::Before) => return Ordering::Greater,
            (CorePosition::Concrete, CorePosition::Concrete) => {}
        }
        match (&self.prerelease, &other.prerelease) {
            (Some(pa), Some(pb)) => compare_prerelease(pa, pb),
            (Some(_), None) => {
                if other.has_wildcard {
                    Ordering::Equal
                } else {
                    Ordering::Less
                }
            }
            (None, Some(_)) => {
                if self.has_wildcard {
                    Ordering::Equal
                } else {
                    Ordering::Greater
                }
            }
            (None, None) => Ordering::Equal,
        }
    }
}

fn compare_components(left: &SemanticVersion, right: &SemanticVersion) -> Ordering {
    let max = left.components.len().max(right.components.len());
    for index in 0..max {
        let left = left.component(index);
        let right = right.component(index);
        if left == WILDCARD || right == WILDCARD {
            continue;
        }
        let ordering = left.cmp(&right);
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    Ordering::Equal
}

fn canonical_components(version: &SemanticVersion) -> Vec<i32> {
    let mut components = version.components.clone();
    while components.len() > 1 && components.last() == Some(&0) {
        components.pop();
    }
    components
}

fn compare_prerelease(a: &str, b: &str) -> Ordering {
    let mut ta = a.split('.');
    let mut tb = b.split('.');
    loop {
        match (ta.next(), tb.next()) {
            (Some(pa), Some(pb)) => {
                let na = pa.chars().all(|c| c.is_ascii_digit());
                let nb = pb.chars().all(|c| c.is_ascii_digit());
                match (na, nb) {
                    (true, true) => {
                        // both numeric: compare length, then value
                        match pa.len().cmp(&pb.len()) {
                            Ordering::Equal => {}
                            o => return o,
                        }
                        match pa.cmp(pb) {
                            Ordering::Equal => {}
                            o => return o,
                        }
                    }
                    (true, false) => return Ordering::Less,
                    (false, true) => return Ordering::Greater,
                    (false, false) => match pa.cmp(pb) {
                        Ordering::Equal => {}
                        o => return o,
                    },
                }
            }
            (Some(_), None) => return Ordering::Greater,
            (None, Some(_)) => return Ordering::Less,
            (None, None) => return Ordering::Equal,
        }
    }
}

fn is_dot_separated_id(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    s.split('.')
        .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

// ═══════════════════════════════════════════════════════════════
// 约束检查 — 对应 Fabric 的 VersionPredicateParser
// ═══════════════════════════════════════════════════════════════

/// 检查版本是否满足约束表达式（Fabric 格式）
/// 检查版本是否满足约束。
/// 空格分隔 = AND，`||` 分隔 = OR（OR 优先级低于 AND）。
pub fn satisfies(version: &SemanticVersion, raw_constraint: &str) -> bool {
    parse_constraint(raw_constraint).contains(&Version::Fabric(version.clone()))
}

fn parse_operator(predicate: &str) -> (&str, &str) {
    for op in &[">=", "<=", "!=", "~", "^", ">", "<", "="] {
        if let Some(stripped) = predicate.strip_prefix(op) {
            return (op, stripped.trim());
        }
    }
    ("=", predicate)
}

// ═══════════════════════════════════════════════════════════════
// 测试
// ═══════════════════════════════════════════════════════════════

use super::Version;
use pubgrub::Ranges;

pub fn parse_constraint(constraint: &str) -> Ranges<Version> {
    let mut final_range: Option<Ranges<Version>> = None;

    for or_group in constraint.split("||") {
        let mut group_range = Ranges::full();
        let mut parts = or_group.split_whitespace().peekable();
        while let Some(part) = parts.next() {
            let part = part.trim();
            if part.is_empty() || part == "*" {
                continue;
            }

            let mut combined = part.to_string();
            if ["<", ">", "<=", ">=", "!=", "~", "^", "="].contains(&part)
                && let Some(next_part) = parts.next()
            {
                combined.push_str(next_part);
            }

            let (op, ver_str) = parse_operator(&combined);
            if let Ok(ref_ver) = SemanticVersion::parse(ver_str, true) {
                if ref_ver.has_wildcard {
                    let wildcard = ref_ver
                        .components
                        .iter()
                        .position(|component| *component == WILDCARD)
                        .unwrap_or(ref_ver.components.len());
                    let mut lower_components = ref_ver.components[..wildcard].to_vec();
                    if lower_components.is_empty() {
                        continue;
                    }
                    let mut upper_components = lower_components.clone();
                    let last = upper_components.len() - 1;
                    upper_components[last] = upper_components[last].saturating_add(1);
                    let mut lower_version = ref_ver.clone();
                    lower_version.components = std::mem::take(&mut lower_components);
                    lower_version.position = CorePosition::Concrete;
                    let lower = Version::Fabric(lower_version.before_core());
                    let mut upper_version = ref_ver.clone();
                    upper_version.components = upper_components;
                    upper_version.position = CorePosition::Concrete;
                    let upper = Version::Fabric(upper_version.before_core());
                    let range = if op == "=" {
                        Ranges::between(lower, upper)
                    } else {
                        Ranges::empty()
                    };
                    group_range = group_range.intersection(&range);
                    continue;
                }
                let r = match op {
                    ">=" => Ranges::higher_than(Version::Fabric(ref_ver.before_core())),
                    "<=" => Ranges::lower_than(Version::Fabric(ref_ver.after_core())),
                    ">" => Ranges::strictly_higher_than(Version::Fabric(ref_ver.after_core())),
                    "<" => Ranges::strictly_lower_than(Version::Fabric(ref_ver.before_core())),
                    "=" => exact_range(&ref_ver, ver_str),
                    "!=" => exact_range(&ref_ver, ver_str).complement(),
                    "~" => {
                        let lower = Version::Fabric(ref_ver.before_core());
                        let mut upper_comp = ref_ver.components.clone();
                        if upper_comp.len() >= 2 {
                            if upper_comp[1] == WILDCARD {
                                upper_comp[0] = upper_comp[0].saturating_add(1);
                                upper_comp.truncate(1);
                            } else {
                                upper_comp[1] = upper_comp[1].saturating_add(1);
                                upper_comp.truncate(2);
                            }
                        } else {
                            upper_comp.push(1);
                        }
                        let mut upper_ver = ref_ver.clone();
                        upper_ver.components = upper_comp;
                        upper_ver.has_wildcard = false;
                        Ranges::between(lower, Version::Fabric(upper_ver.before_core()))
                    }
                    "^" => {
                        let lower = Version::Fabric(ref_ver.before_core());
                        let mut upper_comp = ref_ver.components.clone();
                        if !upper_comp.is_empty() {
                            upper_comp[0] = upper_comp[0].saturating_add(1);
                            upper_comp.truncate(1);
                        } else {
                            upper_comp.push(1);
                        }
                        let mut upper_ver = ref_ver.clone();
                        upper_ver.components = upper_comp;
                        upper_ver.has_wildcard = false;
                        Ranges::between(lower, Version::Fabric(upper_ver.before_core()))
                    }
                    _ => exact_range(&ref_ver, ver_str),
                };
                group_range = group_range.intersection(&r);
            } else {
                let range = if op == "=" {
                    Ranges::singleton(Version::Generic(ver_str.to_string()))
                } else {
                    Ranges::empty()
                };
                group_range = group_range.intersection(&range);
            }
        }

        if let Some(fr) = final_range {
            final_range = Some(fr.union(&group_range));
        } else {
            final_range = Some(group_range);
        }
    }

    final_range.unwrap_or_else(Ranges::full)
}

fn exact_range(version: &SemanticVersion, raw: &str) -> Ranges<Version> {
    if super::has_explicit_suffix(raw) {
        Ranges::singleton(Version::Fabric(version.clone()))
    } else {
        precedence_class(version)
    }
}

pub(super) fn precedence_class(version: &SemanticVersion) -> Ranges<Version> {
    Ranges::between(
        Version::Fabric(version.before_core()),
        Version::Fabric(version.after_core()),
    )
}

pub(super) fn strictly_higher_precedence(version: &SemanticVersion) -> Ranges<Version> {
    Ranges::strictly_higher_than(Version::Fabric(version.after_core()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> SemanticVersion {
        SemanticVersion::parse(s, true).unwrap()
    }

    #[test]
    fn test_parse_basic() {
        assert_eq!(v("0.5.8").components, vec![0, 5, 8]);
        assert_eq!(v("26.1").components, vec![26, 1]);
    }

    #[test]
    fn test_parse_prerelease() {
        let ver = v("1.0-alpha");
        assert_eq!(ver.components, vec![1, 0]);
        assert_eq!(ver.prerelease.as_deref(), Some("alpha"));
    }

    #[test]
    fn test_parse_build_ignored() {
        let ver = v("0.8.10+mc26.1.2");
        assert_eq!(ver.components, vec![0, 8, 10]);
        assert_eq!(ver.build.as_deref(), Some("mc26.1.2"));
    }

    #[test]
    fn test_parse_wildcard() {
        let ver = v("0.8.x");
        assert_eq!(ver.components, vec![0, 8, WILDCARD]);
        assert!(ver.has_wildcard);
    }

    #[test]
    fn test_ordering_numeric() {
        assert!(v("0.5.10") > v("0.5.8"));
        assert!(v("0.8.10") > v("0.8.7"));
        assert!(v("26.1.11") > v("26.1"));
        assert_eq!(v("26.1"), v("26.1"));
    }

    #[test]
    fn test_ordering_build_ignored() {
        // build metadata ignored for comparison
        assert_eq!(v("0.8.10"), v("0.8.10+mc26.1.2"));
        assert_eq!(v("26.1+v260402"), v("26.1"));
    }

    #[test]
    fn test_hash_ignores_build_metadata_like_equality() {
        let mut versions = std::collections::HashSet::new();
        versions.insert(v("0.8.10"));

        assert!(versions.contains(&v("0.8.10+mc26.1.2")));
    }

    #[test]
    fn test_ordering_prerelease() {
        // prerelease < release
        assert!(v("1.0-alpha") < v("1.0"));
        assert!(v("1.0-beta") < v("1.0"));
        assert!(v("0.5.8-hotfix") < v("0.5.8"));
    }

    #[test]
    fn test_ordering_prerelease_numeric() {
        // within prerelease: numeric comparison
        assert!(v("1.0-beta.2") > v("1.0-beta.1"));
    }

    #[test]
    fn test_satisfies_simple() {
        let ver = v("0.8.10");
        assert!(satisfies(&ver, ">=0.8"));
        assert!(satisfies(&ver, "<0.9"));
        assert!(satisfies(&ver, ">=0.8 <0.9"));
    }

    #[test]
    fn test_satisfies_wildcard() {
        let ver = v("0.8.10");
        assert!(satisfies(&ver, "0.8.x")); // 1.0.x → >=1.0 <1.1
    }

    #[test]
    fn test_satisfies_compound() {
        let ver = v("6.7.1");
        assert!(satisfies(&ver, ">=6.7.1 <6.8"));
        assert!(!satisfies(&ver, ">=6.8"));
    }

    #[test]
    fn test_satisfies_prerelease() {
        let ver = v("0.28.3");
        assert!(satisfies(&ver, ">=0.28.3-"));
        assert!(satisfies(&ver, ">=0.28.3- <0.29.0-"));
    }

    #[test]
    fn test_real_world_cases() {
        assert!(satisfies(&v("0.8.10+mc26.1.2"), "0.8.x"));
        assert!(satisfies(&v("26.1+v260402"), ">=26.1-"));
        assert!(satisfies(&v("6.7.1"), ">=6.7.1 <6.8"));
        assert!(satisfies(&v("0.28.3"), ">=0.28.3- <0.29.0-"));
    }

    #[test]
    fn test_tilde_operator() {
        assert!(satisfies(&v("26.1.2"), "~26.1"));
        assert!(satisfies(&v("26.1.10"), "~26.1"));
        assert!(!satisfies(&v("26.2.0"), "~26.1"));
        assert!(!satisfies(&v("27.0.0"), "~26.1"));
    }

    #[test]
    fn test_caret_operator() {
        assert!(satisfies(&v("1.2.3"), "^1.2"));
        assert!(satisfies(&v("1.9.0"), "^1.2"));
        assert!(!satisfies(&v("2.0.0"), "^1.2"));
    }

    #[test]
    fn pubgrub_ranges_match_fabric_predicates() {
        let wildcard = parse_constraint("1.2.x");
        assert!(wildcard.contains(&Version::Fabric(v("1.2.99"))));
        assert!(!wildcard.contains(&Version::Fabric(v("1.3.0-alpha"))));

        let greater = parse_constraint(">1.2.3");
        assert!(greater.contains(&Version::Fabric(v("1.2.4-alpha"))));
        assert!(!greater.contains(&Version::Fabric(v("1.2.3"))));

        let caret = parse_constraint("^0.5");
        assert!(caret.contains(&Version::Fabric(v("0.99"))));
        assert!(!caret.contains(&Version::Fabric(v("1.0-alpha"))));
    }

    #[test]
    fn suffix_free_exact_constraint_matches_the_whole_precedence_class() {
        let range = parse_constraint("=1.2.3");

        assert!(range.contains(&Version::Fabric(v("1.2.3-alpha"))));
        assert!(range.contains(&Version::Fabric(v("1.2.3-beta"))));
        assert!(range.contains(&Version::Fabric(v("1.2.3"))));
        assert!(!range.contains(&Version::Fabric(v("1.2.4-alpha"))));
    }

    #[test]
    fn suffixed_exact_constraint_matches_the_complete_representation() {
        let range = parse_constraint("=1.2.3-alpha");

        assert!(range.contains(&Version::Fabric(v("1.2.3-alpha"))));
        assert!(!range.contains(&Version::Fabric(v("1.2.3-beta"))));
        assert!(!range.contains(&Version::Fabric(v("1.2.3"))));
    }

    #[test]
    fn ordered_constraints_compare_numeric_cores_only() {
        let inclusive = parse_constraint(">=1.2.3-alpha");
        assert!(inclusive.contains(&Version::Fabric(v("1.2.3-anything"))));
        assert!(inclusive.contains(&Version::Fabric(v("1.2.3"))));

        let strict = parse_constraint(">1.2.3-alpha");
        assert!(!strict.contains(&Version::Fabric(v("1.2.3-beta"))));
        assert!(!strict.contains(&Version::Fabric(v("1.2.3"))));
        assert!(strict.contains(&Version::Fabric(v("1.2.4-alpha"))));
    }
}
