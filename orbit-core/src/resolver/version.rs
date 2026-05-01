//! 版本号归一化——分词比较法。
//!
//! 将 MC 模组群魔乱舞的版本字符串切分为 Token 序列，
//! 数字 Token 按数值比较（10 > 8），字母 Token 按字典序。

use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionToken {
    Num(u64),
    Alpha(String),
}

impl PartialOrd for VersionToken {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for VersionToken {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (VersionToken::Num(a), VersionToken::Num(b)) => a.cmp(b),
            (VersionToken::Alpha(a), VersionToken::Alpha(b)) => a.cmp(b),
            (VersionToken::Alpha(_), VersionToken::Num(_)) => Ordering::Less,
            (VersionToken::Num(_), VersionToken::Alpha(_)) => Ordering::Greater,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NormalizedVersion {
    pub raw: String,
    tokens: Vec<VersionToken>,
}

impl NormalizedVersion {
    pub fn new(raw: &str) -> Self {
        Self { raw: raw.to_string(), tokens: tokenize(raw) }
    }

    pub fn zero() -> Self {
        Self { raw: "0.0.0".into(), tokens: vec![VersionToken::Num(0); 3] }
    }
}

impl PartialEq for NormalizedVersion {
    fn eq(&self, other: &Self) -> bool { self.tokens == other.tokens }
}
impl Eq for NormalizedVersion {}

impl PartialOrd for NormalizedVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) }
}

impl Ord for NormalizedVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        for (a, b) in self.tokens.iter().zip(other.tokens.iter()) {
            let ord = a.cmp(b);
            if ord != Ordering::Equal { return ord; }
        }
        other.tokens.len().cmp(&self.tokens.len())
    }
}

/// 按分隔符（. - + _）切分版本字符串为 Token 序列
fn tokenize(raw: &str) -> Vec<VersionToken> {
    let mut tokens = vec![];
    let mut current = String::new();
    let mut is_numeric: Option<bool> = None;

    for ch in raw.chars() {
        if ch == '.' || ch == '-' || ch == '+' || ch == '_' {
            flush_token(&mut tokens, &mut current, &mut is_numeric);
            continue;
        }
        let ch_is_digit = ch.is_ascii_digit();
        match is_numeric {
            Some(was_digit) if was_digit == ch_is_digit => current.push(ch),
            _ => {
                flush_token(&mut tokens, &mut current, &mut is_numeric);
                current.push(ch);
                is_numeric = Some(ch_is_digit);
            }
        }
    }
    flush_token(&mut tokens, &mut current, &mut is_numeric);
    tokens
}

fn flush_token(
    tokens: &mut Vec<VersionToken>,
    current: &mut String,
    is_numeric: &mut Option<bool>,
) {
    if current.is_empty() { return; }
    let token = if is_numeric.unwrap_or(false) {
        VersionToken::Num(current.parse().unwrap_or(0))
    } else {
        VersionToken::Alpha(current.clone())
    };
    tokens.push(token);
    current.clear();
    *is_numeric = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ordering() {
        assert!(NormalizedVersion::new("0.5.10") > NormalizedVersion::new("0.5.8"));
        assert!(NormalizedVersion::new("1.0") > NormalizedVersion::new("1.0-alpha"));
        assert!(NormalizedVersion::new("0.8.7") < NormalizedVersion::new("0.8.10"));
        assert!(NormalizedVersion::new("mc1.21.11") > NormalizedVersion::new("mc1.20.1"));
    }

    #[test]
    fn test_equal() {
        assert_eq!(NormalizedVersion::new("0.5.8"), NormalizedVersion::new("0.5.8"));
        assert_ne!(NormalizedVersion::new("0.5.8"), NormalizedVersion::new("0.5.8-hotfix"));
    }
}
