//! 版本号解析与比较。
//!
//! 不同加载器可能采用不同的版本语义（如 Fabric 基于 SemVer 的变体），
//! 通过 `VersionScheme` trait 统一接口。
//! 当前仅实现 Fabric。

pub mod fabric;

/// 各 loader 实现此 trait
pub trait VersionScheme: Ord + Clone {
    /// 从字符串解析
    fn parse(raw: &str) -> Self;
    /// 是否满足给定的约束表达式
    fn satisfies(&self, constraint: &str) -> bool;
}

/// 版本约束的 AND 组合：所有 term 必须满足
pub fn satisfies_all(version: &dyn Fn(&str) -> bool, constraint: &str) -> bool {
    let constraint = constraint.trim();
    if constraint == "*" || constraint.is_empty() {
        return true;
    }
    for part in constraint.split_whitespace() {
        if !version(part.trim()) {
            return false;
        }
    }
    true
}
