//! orbit.toml 的 serde 结构体与解析/序列化。
//!
//! 格式规格参见 docs/orbit-toml-spec.md

use serde::{Deserialize, Serialize};
use indexmap::IndexMap;

use crate::error::OrbitError;

/// orbit.toml 的完整表示
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrbitManifest {
    pub project: ProjectMeta,
    #[serde(default)]
    pub resolver: ResolverConfig,
    #[serde(default)]
    pub dependencies: IndexMap<String, DependencySpec>,
    #[serde(default)]
    pub groups: IndexMap<String, GroupSpec>,
    #[serde(default)]
    pub overrides: IndexMap<String, DependencySpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMeta {
    pub name: String,
    pub mc_version: String,
    pub modloader: String,
    pub modloader_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authors: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolverConfig {
    #[serde(default = "default_platforms")]
    pub platforms: Vec<String>,
    #[serde(default)]
    pub prerelease: bool,
}

fn default_platforms() -> Vec<String> {
    vec!["modrinth".into(), "curseforge".into()]
}

impl Default for ResolverConfig {
    fn default() -> Self {
        Self {
            platforms: default_platforms(),
            prerelease: false,
        }
    }
}

/// 依赖声明值 —— 简写字符串或内联表（extra fields）
///
/// ```toml
/// sodium = "^0.5"                          # → Short
/// zoomify = { version = "*", optional = true }  # → Full
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DependencySpec {
    Short(String),
    Full {
        #[serde(skip_serializing_if = "Option::is_none")]
        version: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        optional: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        env: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        exclude: Option<Vec<String>>,
    },
}

impl DependencySpec {
    pub fn version_constraint(&self) -> Option<&str> {
        match self {
            DependencySpec::Short(v) => Some(v.as_str()),
            DependencySpec::Full { version, .. } => version.as_deref(),
        }
    }

    pub fn env(&self) -> Option<&str> {
        match self {
            DependencySpec::Short(_) => None,
            DependencySpec::Full { env, .. } => env.as_deref(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupSpec {
    pub dependencies: Vec<String>,
}

impl OrbitManifest {
    /// 从文件路径解析 orbit.toml
    pub fn from_path(path: &std::path::Path) -> Result<Self, OrbitError> {
        let content = std::fs::read_to_string(path)
            .map_err(|_| OrbitError::ManifestNotFound)?;
        let manifest: Self = toml::from_str(&content)?;
        Ok(manifest)
    }

    /// 序列化为 TOML 字符串
    pub fn to_toml_string(&self) -> Result<String, OrbitError> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// 从当前目录（或指定路径）加载 orbit.toml
    pub fn from_dir(dir: &std::path::Path) -> Result<Self, OrbitError> {
        let path = dir.join("orbit.toml");
        Self::from_path(&path)
    }

    /// 从目录中读取项目的 MC 版本（如果 orbit.toml 存在）
    pub fn mc_version_from_dir(dir: &std::path::Path) -> Option<String> {
        Self::from_dir(dir).ok().map(|m| m.project.mc_version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_short_form_dependency() {
        let toml_str = r#"
[project]
name = "test"
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.15.7"

[dependencies]
sodium = "*"
lithium = ">=0.11, <0.14"
"#;
        let manifest: OrbitManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.project.name, "test");
        assert_eq!(manifest.dependencies.len(), 2);
        assert_eq!(
            manifest.dependencies.get("sodium").unwrap().version_constraint(),
            Some("*")
        );
    }

    #[test]
    fn parse_full_form_dependency() {
        let toml_str = r#"
[project]
name = "test"
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.15.7"

[dependencies]
jei = { version = "^12" }
zoomify = { version = "*", optional = true, env = "client" }
"#;
        let manifest: OrbitManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.dependencies.len(), 2);

        let jei = &manifest.dependencies["jei"];
        assert_eq!(jei.version_constraint(), Some("^12"));

        let zoomify = &manifest.dependencies["zoomify"];
        assert_eq!(zoomify.env(), Some("client"));
    }

    #[test]
    fn default_resolver_config() {
        let toml_str = r#"
[project]
name = "test"
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.15.7"
"#;
        let manifest: OrbitManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.resolver.platforms, vec!["modrinth", "curseforge"]);
        assert!(!manifest.resolver.prerelease);
    }
}
