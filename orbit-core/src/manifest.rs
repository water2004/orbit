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
    pub platform: PlatformSnapshot,
    #[serde(default)]
    pub resolver: ResolverConfig,
    #[serde(default)]
    pub packages: IndexMap<String, PackageSpec>,
    #[serde(default)]
    pub groups: IndexMap<String, GroupSpec>,
}

/// Exact platform runtime snapshot produced by `init` or `sync`.
///
/// Versions alone are insufficient: launchers may keep multiple Minecraft and
/// loader versions in shared directories. Every other command consumes these
/// exact paths and hashes; it never scans launcher state or substitutes another
/// file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PlatformSnapshot {
    pub minecraft_jar: PlatformArtifact,
    pub loader_jar: PlatformArtifact,
    pub runtime_jars: Vec<PlatformArtifact>,
    pub physical_environment: crate::metadata::Environment,
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

impl ProjectMeta {
    pub(crate) fn loader_kind(&self) -> Result<crate::loader::LoaderKind, OrbitError> {
        self.modloader.parse().map_err(|message: String| {
            OrbitError::Other(anyhow::anyhow!("invalid project.modloader: {message}"))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolverConfig {
    /// Provider catalogs enabled for unqualified search and add operations.
    /// Exact package remotes are not prioritized and always participate.
    #[serde(default = "default_catalogs")]
    pub catalogs: Vec<String>,
}

fn default_catalogs() -> Vec<String> {
    vec!["modrinth".into()]
}

impl Default for ResolverConfig {
    fn default() -> Self {
        Self {
            catalogs: default_catalogs(),
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

/// One managed logical-package declaration.
///
/// The schema deliberately has no short string form: every package must
/// name at least one candidate source, which keeps all commands on the same
/// discovery path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PackageSpec {
    #[serde(default = "default_version_constraint")]
    pub version: String,
    /// Ordered set rule over the raw text following the leading numeric
    /// version core. It is independent from the Loader-native numeric range.
    #[serde(
        default = "default_suffix_expression",
        skip_serializing_if = "is_all_suffix"
    )]
    pub suffix: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub optional: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<crate::metadata::Environment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remotes: Vec<PackageRemote>,
}

fn default_version_constraint() -> String {
    "*".to_string()
}

fn default_suffix_expression() -> String {
    "all".to_string()
}

fn is_all_suffix(value: &String) -> bool {
    value == "all"
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl PackageSpec {
    pub fn new(version: impl Into<String>, remotes: Vec<PackageRemote>) -> Self {
        Self {
            version: version.into(),
            suffix: default_suffix_expression(),
            optional: false,
            env: None,
            exclude: Vec::new(),
            remotes,
        }
    }

    pub fn version_constraint(&self) -> &str {
        &self.version
    }

    pub fn suffix_expression(&self) -> &str {
        &self.suffix
    }

    pub fn env(&self) -> Option<crate::metadata::Environment> {
        self.env
    }

    /// Resolve the optional user filter against the selected JAR's declaration.
    pub fn effective_environment(
        &self,
        declared: crate::metadata::Environment,
    ) -> crate::metadata::Environment {
        self.env.unwrap_or(declared)
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
        crate::version_suffix::VersionSuffixRule::parse(&self.suffix).map_err(|error| {
            OrbitError::Other(anyhow::anyhow!(
                "package '{package}' has an invalid suffix expression '{}': {error}",
                self.suffix
            ))
        })?;
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
    pub packages: Vec<String>,
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
        self.project.loader_kind()?;
        for (package, specification) in &self.packages {
            if package.trim().is_empty() {
                return Err(OrbitError::Other(anyhow::anyhow!(
                    "orbit.toml contains an empty package id"
                )));
            }
            specification.validate(package)?;
        }
        for (group_name, group) in &self.groups {
            let mut unique = std::collections::BTreeSet::new();
            for package in &group.packages {
                if !self.packages.contains_key(package) {
                    return Err(OrbitError::Other(anyhow::anyhow!(
                        "group '{group_name}' references unmanaged package '{package}'"
                    )));
                }
                if !unique.insert(package) {
                    return Err(OrbitError::Other(anyhow::anyhow!(
                        "group '{group_name}' lists package '{package}' more than once"
                    )));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_package_with_multiple_remotes() {
        let toml_str = r#"
[project]
name = "test"
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.15.7"

[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }
runtime_jars = []
physical_environment = "client"

[packages]
sodium = { version = "*", remotes = [
  { type = "modrinth", project_id = "AANobbMI" },
  { type = "curseforge", project_id = 394468 },
] }
"#;
        let manifest: OrbitManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.project.name, "test");
        assert_eq!(manifest.packages.len(), 1);
        assert_eq!(
            manifest
                .packages
                .get("sodium")
                .unwrap()
                .version_constraint(),
            "*"
        );
        assert_eq!(manifest.packages["sodium"].remotes.len(), 2);
        assert_eq!(manifest.packages["sodium"].suffix_expression(), "all");
    }

    #[test]
    fn suffix_set_rule_is_strictly_validated_and_roundtrips() {
        let mut package = PackageSpec::new(
            "*",
            vec![PackageRemote::Modrinth {
                project_id: "AANobbMI".to_string(),
            }],
        );
        package.suffix = "all; intersect not contains(i\"beta\"); complement".to_string();
        package.validate("sodium").unwrap();
        let encoded = toml::to_string(&package).unwrap();
        let decoded: PackageSpec = toml::from_str(&encoded).unwrap();
        assert_eq!(decoded.suffix, package.suffix);

        package.suffix = "all; exclude \"beta\"".to_string();
        assert!(package.validate("sodium").is_err());
    }

    #[test]
    fn parse_full_package_form() {
        let toml_str = r#"
[project]
name = "test"
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.15.7"

[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }
runtime_jars = []
physical_environment = "client"

[packages]
jei = { version = "^12", remotes = [{ type = "curseforge", project_id = 238222 }] }
zoomify = { version = "*", optional = true, env = "client", remotes = [{ type = "modrinth", project_id = "w7ThoJFB" }] }
"#;
        let manifest: OrbitManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.packages.len(), 2);

        let jei = &manifest.packages["jei"];
        assert_eq!(jei.version_constraint(), "^12");

        let zoomify = &manifest.packages["zoomify"];
        assert_eq!(zoomify.env(), Some(crate::metadata::Environment::Client));
    }

    #[test]
    fn rejects_invalid_package_environment() {
        let toml_str = r#"
[project]
name = "test"
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.15.7"
[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }
runtime_jars = []
physical_environment = "client"
[packages]
example = { version = "*", env = "desktop", remotes = [{ type = "file", path = "example.jar" }] }
"#;

        assert!(toml::from_str::<OrbitManifest>(toml_str).is_err());
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
runtime_jars = []
physical_environment = "client"
"#;
        let manifest: OrbitManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.resolver.catalogs, vec!["modrinth"]);
    }

    #[test]
    fn incomplete_platform_snapshot_is_rejected() {
        let error = toml::from_str::<OrbitManifest>(
            r#"
[project]
name = "test"
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.15.7"

[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }
"#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("missing field `runtime_jars`"));
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
runtime_jars = []
physical_environment = "client"
[resolver]
platforms = ["modrinth"]
"#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn persisted_package_requires_a_remote() {
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
runtime_jars = []
physical_environment = "client"
[packages]
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
        manifest.packages["sodium"]
            .remotes
            .push(PackageRemote::Modrinth {
                project_id: "AANobbMI".to_string(),
            });
        assert!(manifest.to_toml_string().is_ok());
    }

    #[test]
    fn obsolete_dependency_and_override_tables_are_rejected() {
        let base = r#"
[project]
name = "test"
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.16.10"
[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }
runtime_jars = []
physical_environment = "client"
"#;

        let dependency_error = toml::from_str::<OrbitManifest>(&format!(
            "{base}\n[dependencies]\nsodium = {{ version = \"*\", remotes = [{{ type = \"file\", path = \"sodium.jar\" }}] }}\n"
        ))
        .unwrap_err();
        let override_error =
            toml::from_str::<OrbitManifest>(&format!("{base}\n[overrides]\nsodium = \"=1\"\n"))
                .unwrap_err();

        assert!(dependency_error.to_string().contains("unknown field"));
        assert!(override_error.to_string().contains("unknown field"));
    }
}
