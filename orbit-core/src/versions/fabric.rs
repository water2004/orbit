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

// ═══════════════════════════════════════════════════════════════
// SemanticVersion — 对应 Fabric 的 SemanticVersionImpl
// ═══════════════════════════════════════════════════════════════

const WILDCARD: i32 = i32::MIN;

#[derive(Debug, Clone)]
pub struct SemanticVersion {
    pub raw: String,
    /// 数字组件（不含通配符），长度至少 1
    components: Vec<i32>,
    /// prerelease 后缀（`-` 之后），None 表示正式版
    prerelease: Option<String>,
    /// build 后缀（`+` 之后），比较时忽略
    #[allow(dead_code)]
    build: Option<String>,
    /// 是否有通配符
    has_wildcard: bool,
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
                if first_wildcard.is_none() { first_wildcard = Some(i); }
                if i > 0 && components[i - 1] == WILDCARD {
                    // already wildcard, keep going
                }
            } else {
                let trimmed = cs.trim();
                if trimmed.is_empty() {
                    return Err("missing version component".into());
                }
                components[i] = trimmed.parse::<i32>().map_err(|_| format!("invalid component '{cs}'"))?;
                if components[i] < 0 {
                    return Err(format!("negative component '{cs}'"));
                }
            }
        }

        if store_x && components.len() == 1 && components[0] == WILDCARD {
            return Err("version 'x' not allowed".into());
        }
        // strip extra wildcards: 1.x.x → 1.x
        if let Some(fw) = first_wildcard {
            if fw > 0 && components.len() > fw + 1 {
                components.truncate(fw + 1);
            }
        }

        Ok(Self { raw: raw.to_string(), components, prerelease, build, has_wildcard })
    }

    /// 获取第 pos 个组件，超出范围返回 0 或 WILDCARD
    fn component(&self, pos: usize) -> i32 {
        if pos >= self.components.len() {
            if self.has_wildcard { WILDCARD } else { 0 }
        } else {
            self.components[pos]
        }
    }
}

impl PartialEq for SemanticVersion {
    fn eq(&self, other: &Self) -> bool {
        self.components == other.components && self.prerelease == other.prerelease
    }
}
impl Eq for SemanticVersion {}

impl PartialOrd for SemanticVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) }
}

impl Ord for SemanticVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        // 1. 比较核心组件
        let max = self.components.len().max(other.components.len());
        for i in 0..max {
            let a = self.component(i);
            let b = other.component(i);
            if a == WILDCARD || b == WILDCARD { continue; }
            match a.cmp(&b) {
                Ordering::Equal => continue,
                o => return o,
            }
        }
        // 2. prerelease
        match (&self.prerelease, &other.prerelease) {
            (Some(pa), Some(pb)) => compare_prerelease(pa, pb),
            (Some(_), None) => if other.has_wildcard { Ordering::Equal } else { Ordering::Less },
            (None, Some(_)) => if self.has_wildcard { Ordering::Equal } else { Ordering::Greater },
            (None, None) => Ordering::Equal,
        }
    }
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
                        match pa.cmp(pb) { Ordering::Equal => {}, o => return o }
                    }
                    (true, false) => return Ordering::Less,
                    (false, true) => return Ordering::Greater,
                    (false, false) => match pa.cmp(pb) { Ordering::Equal => {}, o => return o },
                }
            }
            (Some(_), None) => return Ordering::Greater,
            (None, Some(_)) => return Ordering::Less,
            (None, None) => return Ordering::Equal,
        }
    }
}

fn is_dot_separated_id(s: &str) -> bool {
    if s.is_empty() { return true; }
    s.split('.').all(|part| {
        !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

// ═══════════════════════════════════════════════════════════════
// 约束检查 — 对应 Fabric 的 VersionPredicateParser
// ═══════════════════════════════════════════════════════════════

/// 检查版本是否满足约束表达式（Fabric 格式）
pub fn satisfies(version: &SemanticVersion, raw_constraint: &str) -> bool {
    let constraint = raw_constraint.trim();
    if constraint == "*" || constraint.is_empty() {
        return true;
    }
    for part in constraint.split_whitespace() {
        let part = part.trim();
        if part.is_empty() || part == "*" { continue; }
        if !satisfies_single(version, part) {
            return false;
        }
    }
    true
}

fn satisfies_single(version: &SemanticVersion, predicate: &str) -> bool {
    let (op, ver_str) = parse_operator(predicate);
    let mut ref_ver = match SemanticVersion::parse(ver_str, true) {
        Ok(v) => v,
        Err(_) => return false,
    };

    // 通配符处理: 1.0.x → 替换为 >=1.0 <1.1
    if ref_ver.has_wildcard {
        if op != "=" {
            return false;
        }
        let comp_count = ref_ver.components.len();
        let mut new_components = vec![0i32; comp_count - 1];
        for i in 0..comp_count - 1 {
            new_components[i] = ref_ver.component(i);
        }
        ref_ver = SemanticVersion {
            raw: String::new(),
            components: new_components,
            prerelease: None,
            build: None,
            has_wildcard: false,
        };
        // 检查下界: >= lower
        if version.cmp(&ref_ver) == Ordering::Less { return false; }
        // 检查上界: < upper (bump last component)
        let mut upper = ref_ver.clone();
        if let Some(last) = upper.components.last_mut() { *last += 1; }
        return version.cmp(&upper) == Ordering::Less;
    }

    match op {
        ">=" => version >= &ref_ver,
        ">" => version > &ref_ver,
        "<=" => version <= &ref_ver,
        "<" => version < &ref_ver,
        "=" => version == &ref_ver,
        _ => version == &ref_ver,
    }
}

fn parse_operator(predicate: &str) -> (&str, &str) {
    for op in &[">=", "<=", ">", "<", "="] {
        if predicate.starts_with(op) {
            return (op, predicate[op.len()..].trim());
        }
    }
    ("=", predicate)
}

// ═══════════════════════════════════════════════════════════════
// 测试
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> SemanticVersion { SemanticVersion::parse(s, true).unwrap() }

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
        // 来自实际报错的 case
        assert!(satisfies(&v("0.8.10+mc26.1.2"), "0.8.x"));
        assert!(satisfies(&v("26.1+v260402"), ">=26.1-"));
        assert!(satisfies(&v("6.7.1"), ">=6.7.1 <6.8"));
        assert!(satisfies(&v("0.28.3"), ">=0.28.3- <0.29.0-"));
    }
}
