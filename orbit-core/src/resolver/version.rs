use std::cmp::Ordering;
use std::hash::{Hash, Hasher};

/// 统一的版本号表示，封装特定平台的版本规则，以便适配 PubGrub。
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum NormalizedVersion {
    Fabric(crate::versions::fabric::SemanticVersion),
    Generic(String),
}

impl Hash for NormalizedVersion {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Fabric(f) => {
                state.write_u8(0);
                f.hash(state);
            }
            Self::Generic(s) => {
                state.write_u8(1);
                s.hash(state);
            }
        }
    }
}

impl PartialOrd for NormalizedVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for NormalizedVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Fabric(a), Self::Fabric(b)) => a.cmp(b),
            (Self::Generic(a), Self::Generic(b)) => a.cmp(b),
            (Self::Fabric(_), Self::Generic(_)) => Ordering::Less,
            (Self::Generic(_), Self::Fabric(_)) => Ordering::Greater,
        }
    }
}

impl std::fmt::Display for NormalizedVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fabric(v) => write!(f, "{}", v.raw),
            Self::Generic(s) => write!(f, "{}", s),
        }
    }
}

impl NormalizedVersion {
    pub fn new(raw: &str) -> Self {
        Self::Generic(raw.to_string())
    }

    /// 根据当前环境的 modloader 解析版本号
    pub fn parse(raw: &str, loader: &str) -> Self {
        if loader == "fabric" || loader == "quilt" {
            if let Ok(v) = crate::versions::fabric::SemanticVersion::parse(raw, true) {
                return Self::Fabric(v);
            }
        }
        Self::Generic(raw.to_string())
    }

    pub fn zero() -> Self {
        Self::Generic("0.0.0".to_string())
    }
}

impl pubgrub::version::Version for NormalizedVersion {
    fn lowest() -> Self {
        Self::Generic("".to_string())
    }

    fn bump(&self) -> Self {
        match self {
            Self::Fabric(f) => Self::Fabric(f.bump()),
            Self::Generic(s) => Self::Generic(format!("{}~", s)),
        }
    }
}
