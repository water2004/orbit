//! orbit.toml 的 serde 结构体与解析/序列化。
//!
//! 格式规格参见 docs/orbit-toml-spec.md

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::error::OrbitError;

/// orbit.toml 的完整表示
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrbitManifest {
    pub project: ProjectMeta,
    pub platform: PlatformArtifacts,
    #[serde(default)]
    pub resolver: ResolverConfig,
    #[serde(default)]
    pub dependencies: IndexMap<String, DependencySpec>,
    #[serde(default)]
    pub groups: IndexMap<String, GroupSpec>,
    #[serde(default)]
    pub overrides: IndexMap<String, DependencySpec>,
}

/// Concrete runtime artifacts used by the selected launcher instance.
///
/// Versions alone are insufficient: launchers may keep multiple Minecraft and
/// loader versions in shared directories. Orbit therefore pins the exact files
/// it inspected, while `sync` may deliberately refresh these pins.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PlatformArtifacts {
    pub minecraft_jar: PlatformArtifact,
    pub loader_jar: PlatformArtifact,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PlatformArtifact {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct ResolverConfig {
    /// Provider catalogs enabled for unqualified search and add operations.
    /// Exact package remotes are not prioritized and always participate.
    #[serde(default = "default_catalogs")]
    pub catalogs: Vec<String>,
    #[serde(default)]
    pub prerelease: bool,
}

fn default_catalogs() -> Vec<String> {
    vec!["modrinth".into()]
}

impl Default for ResolverConfig {
    fn default() -> Self {
        Self {
            catalogs: default_catalogs(),
            prerelease: false,
        }
    }
}

/// A package source locator. It identifies where candidates can be discovered,
/// never the package identity or version; those facts come only from JAR
/// metadata after downloading.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum PackageRemote {
    File { path: String },
    Modrinth { project_id: String },
    Curseforge { project_id: u32 },
}

impl PackageRemote {
    pub fn provider(&self) -> &'static str {
        match self {
            Self::File { .. } => "file",
            Self::Modrinth { .. } => "modrinth",
            Self::Curseforge { .. } => "curseforge",
        }
    }

    pub fn locator(&self) -> String {
        match self {
            Self::File { path } => path.clone(),
            Self::Modrinth { project_id } => project_id.clone(),
            Self::Curseforge { project_id } => project_id.to_string(),
        }
    }

    pub fn display_locator(&self) -> String {
        match self {
            Self::File { path } if path.replace('\\', "/").starts_with(".orbit/sources/") => {
                "file:managed local source".to_string()
            }
            _ => format!("{}:{}", self.provider(), self.locator()),
        }
    }

    pub(crate) fn validate(&self, package: &str) -> Result<(), OrbitError> {
        let valid = match self {
            Self::File { path } => !path.trim().is_empty(),
            Self::Modrinth { project_id } => !project_id.trim().is_empty(),
            Self::Curseforge { project_id } => *project_id > 0,
        };
        if !valid {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "package '{package}' contains an empty or invalid {} remote",
                self.provider()
            )));
        }
        Ok(())
    }
}

/// One root package declaration.
///
/// The schema deliberately has no short string form: every root package must
/// name at least one candidate source, which keeps all commands on the same
/// discovery path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DependencySpec {
    #[serde(default = "default_version_constraint")]
    pub version: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub optional: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remotes: Vec<PackageRemote>,
}

fn default_version_constraint() -> String {
    "*".to_string()
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl DependencySpec {
    pub fn new(version: impl Into<String>, remotes: Vec<PackageRemote>) -> Self {
        Self {
            version: version.into(),
            optional: false,
            env: None,
            exclude: Vec::new(),
            remotes,
        }
    }

    pub fn version_constraint(&self) -> Option<&str> {
        Some(&self.version)
    }

    pub fn env(&self) -> Option<&str> {
        self.env.as_deref()
    }

    pub fn optional(&self) -> bool {
        self.optional
    }

    pub fn exclusions(&self) -> &[String] {
        &self.exclude
    }

    pub fn remote(&self, remote: &PackageRemote) -> bool {
        self.remotes.contains(remote)
    }

    pub fn validate(&self, package: &str) -> Result<(), OrbitError> {
        if self.remotes.is_empty() {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "package '{package}' must declare at least one remote"
            )));
        }
        for remote in &self.remotes {
            remote.validate(package)?;
        }
        let mut unique = std::collections::BTreeSet::new();
        if self.remotes.iter().any(|remote| !unique.insert(remote)) {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "package '{package}' declares the same remote more than once"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupSpec {
    pub dependencies: Vec<String>,
}

impl OrbitManifest {
    /// 从文件路径解析 orbit.toml
    pub fn from_path(path: &std::path::Path) -> Result<Self, OrbitError> {
        let content = std::fs::read_to_string(path).map_err(|_| OrbitError::ManifestNotFound)?;
        let manifest: Self = toml::from_str(&content)?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// 序列化为 TOML 字符串
    pub fn to_toml_string(&self) -> Result<String, OrbitError> {
        self.validate()?;
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

    pub fn validate(&self) -> Result<(), OrbitError> {
        for (package, dependency) in &self.dependencies {
            dependency.validate(package)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_dependency_with_multiple_remotes() {
        let toml_str = r#"
[project]
name = "test"
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.15.7"

[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }

[dependencies]
sodium = { version = "*", remotes = [
  { type = "modrinth", project_id = "AANobbMI" },
  { type = "curseforge", project_id = 394468 },
] }
"#;
        let manifest: OrbitManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.project.name, "test");
        assert_eq!(manifest.dependencies.len(), 1);
        assert_eq!(
            manifest
                .dependencies
                .get("sodium")
                .unwrap()
                .version_constraint(),
            Some("*")
        );
        assert_eq!(manifest.dependencies["sodium"].remotes.len(), 2);
    }

    #[test]
    fn parse_full_form_dependency() {
        let toml_str = r#"
[project]
name = "test"
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.15.7"

[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }

[dependencies]
jei = { version = "^12", remotes = [{ type = "curseforge", project_id = 238222 }] }
zoomify = { version = "*", optional = true, env = "client", remotes = [{ type = "modrinth", project_id = "w7ThoJFB" }] }
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

[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }
"#;
        let manifest: OrbitManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.resolver.catalogs, vec!["modrinth"]);
        assert!(!manifest.resolver.prerelease);
    }

    #[test]
    fn obsolete_platform_priority_field_is_rejected() {
        let error = toml::from_str::<OrbitManifest>(
            r#"
[project]
name = "test"
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.16.10"
[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }
[resolver]
platforms = ["modrinth"]
"#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn persisted_root_dependency_requires_a_remote() {
        let mut manifest: OrbitManifest = toml::from_str(
            r#"
[project]
name = "test"
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.16.10"
[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }
[dependencies]
sodium = { version = "*" }
"#,
        )
        .unwrap();

        assert!(
            manifest
                .to_toml_string()
                .unwrap_err()
                .to_string()
                .contains("at least one remote")
        );
        manifest.dependencies["sodium"]
            .remotes
            .push(PackageRemote::Modrinth {
                project_id: "AANobbMI".to_string(),
            });
        assert!(manifest.to_toml_string().is_ok());
    }
}
