//! Version policy inspection and transactional application for managed packages.

use std::cmp::Ordering;
use std::path::Path;

use crate::error::OrbitError;
use crate::installer::{InstallInteraction, InstallReport};
use crate::loader::LoaderKind;
use crate::providers::ModProvider;
use crate::versions::Version;
use crate::workspace::{Lockfile, ManifestFile};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionComparison {
    Exact,
    GreaterThan,
    AtLeast,
    LessThan,
    AtMost,
}

impl VersionComparison {
    pub fn operator(self) -> &'static str {
        match self {
            Self::Exact => "=",
            Self::GreaterThan => ">",
            Self::AtLeast => ">=",
            Self::LessThan => "<",
            Self::AtMost => "<=",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageVersionPolicy {
    Any,
    Comparison {
        operator: VersionComparison,
        version: String,
    },
    Range {
        lower: String,
        upper: String,
        include_lower: bool,
        include_upper: bool,
    },
    /// A valid loader expression written outside Orbit's structured policy UI.
    /// It can be displayed but must be replaced by a structured policy before
    /// the transactional CLI can apply it.
    Custom(String),
}

impl PackageVersionPolicy {
    pub fn from_requirement(requirement: &str) -> Self {
        let requirement = requirement.trim();
        if requirement.is_empty() || requirement == "*" {
            return Self::Any;
        }
        for (prefix, operator) in [
            (">=", VersionComparison::AtLeast),
            ("<=", VersionComparison::AtMost),
            ("=", VersionComparison::Exact),
            (">", VersionComparison::GreaterThan),
            ("<", VersionComparison::LessThan),
        ] {
            if let Some(version) = requirement.strip_prefix(prefix)
                && !version.trim().is_empty()
                && !version.chars().any(char::is_whitespace)
            {
                return Self::Comparison {
                    operator,
                    version: version.trim().to_string(),
                };
            }
        }
        if let Some(policy) =
            parse_maven_range(requirement).or_else(|| parse_fabric_range(requirement))
        {
            return policy;
        }
        Self::Custom(requirement.to_string())
    }

    pub fn requirement(&self, loader: LoaderKind) -> Result<String, OrbitError> {
        match self {
            Self::Any => Ok("*".to_string()),
            Self::Comparison { operator, version } => {
                validate_version(version)?;
                Ok(format!("{}{}", operator.operator(), version.trim()))
            }
            Self::Range {
                lower,
                upper,
                include_lower,
                include_upper,
            } => {
                validate_range(lower, upper, *include_lower, *include_upper, loader)?;
                match loader {
                    LoaderKind::Fabric | LoaderKind::Quilt => Ok(format!(
                        "{}{} {}{}",
                        if *include_lower { ">=" } else { ">" },
                        lower.trim(),
                        if *include_upper { "<=" } else { "<" },
                        upper.trim()
                    )),
                    LoaderKind::Forge | LoaderKind::NeoForge => Ok(format!(
                        "{}{},{}{}",
                        if *include_lower { '[' } else { '(' },
                        lower.trim(),
                        upper.trim(),
                        if *include_upper { ']' } else { ')' }
                    )),
                }
            }
            Self::Custom(requirement) => Err(OrbitError::Other(anyhow::anyhow!(
                "custom version requirement '{requirement}' is display-only; choose a structured policy"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageConstraintState {
    pub package: String,
    pub constraint: String,
    pub suffix: String,
    pub policy: PackageVersionPolicy,
    pub selected_version: Option<String>,
    pub selected_satisfies: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct PackageConstraintApplyReport {
    pub package: String,
    pub previous: String,
    pub current: String,
    pub previous_suffix: String,
    pub suffix: String,
    pub policy: PackageVersionPolicy,
    pub previous_selected_version: Option<String>,
    pub selected_version: Option<String>,
    pub selected_satisfies: Option<bool>,
    pub changed: bool,
    pub applied: bool,
    pub dry_run: bool,
    pub transaction: InstallReport,
}

pub fn package_constraint(
    instance_dir: &Path,
    package: &str,
) -> Result<PackageConstraintState, OrbitError> {
    let manifest = ManifestFile::open(instance_dir)?;
    let loader = manifest.inner.project.loader_kind()?;
    let specification = manifest
        .inner
        .packages
        .get(package)
        .ok_or_else(|| OrbitError::ModNotFound(package.to_string()))?;
    let selected_version = selected_version(instance_dir, package)?;
    let suffix = crate::VersionSuffixRule::parse(&specification.suffix)?.canonical();
    let selected_satisfies = selection_satisfies(
        selected_version.as_deref(),
        &specification.version,
        &suffix,
        loader,
    );
    Ok(PackageConstraintState {
        package: package.to_string(),
        constraint: specification.version.clone(),
        suffix,
        policy: PackageVersionPolicy::from_requirement(&specification.version),
        selected_version,
        selected_satisfies,
    })
}

pub async fn apply_package_constraint(
    instance_dir: &Path,
    package: &str,
    policy: PackageVersionPolicy,
    suffix: Option<String>,
    providers: &[Box<dyn ModProvider>],
    jar_cache: &crate::jar_cache::JarCache,
    dry_run: bool,
    interaction: InstallInteraction,
) -> Result<PackageConstraintApplyReport, OrbitError> {
    let mut manifest = ManifestFile::open(instance_dir)?;
    let loader = manifest.inner.project.loader_kind()?;
    let current = policy.requirement(loader)?;
    let specification = manifest
        .inner
        .packages
        .get_mut(package)
        .ok_or_else(|| OrbitError::ModNotFound(package.to_string()))?;
    let suffix =
        crate::VersionSuffixRule::parse(suffix.as_deref().unwrap_or(&specification.suffix))?
            .canonical();
    let previous = std::mem::replace(&mut specification.version, current.clone());
    let previous_suffix = std::mem::replace(&mut specification.suffix, suffix.clone());
    let changed = previous != current || previous_suffix != suffix;
    let previous_selected_version = selected_version(instance_dir, package)?;

    let outcome = crate::installer::repair_manifest_instance(
        instance_dir,
        providers,
        jar_cache,
        dry_run,
        interaction,
        manifest,
        changed,
    )
    .await?;
    let selected_version = if outcome.committed {
        selected_version(instance_dir, package)?
    } else {
        planned_selected_version(&outcome.report, package)
            .or_else(|| previous_selected_version.clone())
    };
    let selected_satisfies =
        selection_satisfies(selected_version.as_deref(), &current, &suffix, loader);

    Ok(PackageConstraintApplyReport {
        package: package.to_string(),
        previous,
        current,
        previous_suffix,
        suffix,
        policy,
        previous_selected_version,
        selected_version,
        selected_satisfies,
        changed,
        applied: outcome.committed,
        dry_run,
        transaction: outcome.report,
    })
}

fn selected_version(instance_dir: &Path, package: &str) -> Result<Option<String>, OrbitError> {
    if !instance_dir.join("orbit.lock").is_file() {
        return Ok(None);
    }
    Ok(Lockfile::open(instance_dir)?
        .inner
        .find(package)
        .map(|entry| entry.version.clone()))
}

fn planned_selected_version(report: &InstallReport, package: &str) -> Option<String> {
    report
        .installed
        .iter()
        .find(|installed| installed.mod_id == package)
        .map(|installed| installed.version.clone())
        .or_else(|| {
            report
                .changes
                .iter()
                .find(|change| change.package == package)
                .and_then(|change| change.selected_version.clone())
        })
}

fn selection_satisfies(
    selected: Option<&str>,
    constraint: &str,
    suffix: &str,
    loader: LoaderKind,
) -> Option<bool> {
    selected.map(|version| {
        let version = Version::parse(version, loader);
        Version::parse_constraint(constraint, loader).contains(&version)
            && crate::VersionSuffixRule::parse(suffix)
                .expect("manifest suffix expression was validated")
                .matches(version.suffix().as_deref())
    })
}

fn validate_version(version: &str) -> Result<(), OrbitError> {
    if version.trim().is_empty() || version.chars().any(char::is_whitespace) {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "version policy requires one non-empty JAR-declared version"
        )));
    }
    Ok(())
}

fn validate_range(
    lower: &str,
    upper: &str,
    include_lower: bool,
    include_upper: bool,
    loader: LoaderKind,
) -> Result<(), OrbitError> {
    validate_version(lower)?;
    validate_version(upper)?;
    match Version::parse(lower, loader).cmp_precedence(&Version::parse(upper, loader)) {
        Ordering::Greater => Err(OrbitError::Other(anyhow::anyhow!(
            "version range lower bound '{lower}' is newer than upper bound '{upper}'"
        ))),
        Ordering::Equal if !include_lower || !include_upper => Err(OrbitError::Other(
            anyhow::anyhow!("an equal-bound version range must include both bounds"),
        )),
        _ => Ok(()),
    }
}

fn parse_maven_range(requirement: &str) -> Option<PackageVersionPolicy> {
    let include_lower = requirement.starts_with('[');
    let include_upper = requirement.ends_with(']');
    if !matches!(requirement.chars().next(), Some('[' | '('))
        || !matches!(requirement.chars().last(), Some(']' | ')'))
    {
        return None;
    }
    let (lower, upper) = requirement[1..requirement.len() - 1].split_once(',')?;
    if lower.trim().is_empty() || upper.trim().is_empty() {
        return None;
    }
    Some(PackageVersionPolicy::Range {
        lower: lower.trim().to_string(),
        upper: upper.trim().to_string(),
        include_lower,
        include_upper,
    })
}

fn parse_fabric_range(requirement: &str) -> Option<PackageVersionPolicy> {
    let mut parts = requirement.split_whitespace();
    let lower = parts.next()?;
    let upper = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let (lower, include_lower) = lower
        .strip_prefix(">=")
        .map(|version| (version, true))
        .or_else(|| lower.strip_prefix('>').map(|version| (version, false)))?;
    let (upper, include_upper) = upper
        .strip_prefix("<=")
        .map(|version| (version, true))
        .or_else(|| upper.strip_prefix('<').map(|version| (version, false)))?;
    if lower.is_empty() || upper.is_empty() {
        return None;
    }
    Some(PackageVersionPolicy::Range {
        lower: lower.to_string(),
        upper: upper.to_string(),
        include_lower,
        include_upper,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockfile::{ArtifactSource, LockMeta, OrbitLockfile, PackageEntry};
    use crate::manifest::{OrbitManifest, PackageRemote, PackageSpec, ProjectMeta};
    use crate::metadata::Environment;
    use indexmap::IndexMap;
    use std::io::Write;

    #[test]
    fn structured_ranges_use_each_loader_family_native_syntax() {
        let policy = PackageVersionPolicy::Range {
            lower: "1.2.3".to_string(),
            upper: "2.0.0".to_string(),
            include_lower: true,
            include_upper: false,
        };

        assert_eq!(
            policy.requirement(LoaderKind::Fabric).unwrap(),
            ">=1.2.3 <2.0.0"
        );
        assert_eq!(
            policy.requirement(LoaderKind::Forge).unwrap(),
            "[1.2.3,2.0.0)"
        );
    }

    #[test]
    fn canonical_policies_roundtrip_to_structured_state() {
        for requirement in ["*", "=1.2.3-alpha", ">=1.2.3", ">=1 <2", "[1,2)"] {
            let policy = PackageVersionPolicy::from_requirement(requirement);
            assert!(
                !matches!(policy, PackageVersionPolicy::Custom(_)),
                "{requirement}"
            );
        }
        assert!(matches!(
            PackageVersionPolicy::from_requirement("^1.2"),
            PackageVersionPolicy::Custom(_)
        ));
    }

    #[test]
    fn invalid_ranges_are_rejected_before_discovery() {
        let reversed = PackageVersionPolicy::Range {
            lower: "2".to_string(),
            upper: "1".to_string(),
            include_lower: true,
            include_upper: true,
        };
        assert!(reversed.requirement(LoaderKind::Fabric).is_err());

        let empty = PackageVersionPolicy::Range {
            lower: "1".to_string(),
            upper: "1".to_string(),
            include_lower: false,
            include_upper: true,
        };
        assert!(empty.requirement(LoaderKind::Forge).is_err());
    }

    fn write_fabric_jar(path: &Path, version: &str) {
        let file = std::fs::File::create(path).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        archive
            .start_file("fabric.mod.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        write!(
            archive,
            r#"{{"schemaVersion":1,"id":"example","version":"{version}","name":"Example"}}"#
        )
        .unwrap();
        archive.finish().unwrap();
    }

    fn write_constraint_instance(directory: &Path) -> crate::jar_cache::JarCache {
        crate::platform_detection::test_support::write_platform(directory, "1", "fabric", "1");
        let discovered =
            crate::platform_detection::discover_platform_for_init(directory, "1", "fabric", "1")
                .unwrap();
        let sources = directory.join(".orbit/sources");
        std::fs::create_dir_all(&sources).unwrap();
        let alpha = sources.join("example-alpha.jar");
        let beta = sources.join("example-beta.jar");
        write_fabric_jar(&alpha, "1.2.3-alpha");
        write_fabric_jar(&beta, "1.2.3-beta");
        let alpha_remote = PackageRemote::File {
            path: ".orbit/sources/example-alpha.jar".to_string(),
        };
        let beta_remote = PackageRemote::File {
            path: ".orbit/sources/example-beta.jar".to_string(),
        };
        let manifest = OrbitManifest {
            project: ProjectMeta {
                name: "constraint-test".to_string(),
                mc_version: "1".to_string(),
                modloader: "fabric".to_string(),
                modloader_version: "1".to_string(),
                description: None,
                authors: None,
                version: None,
            },
            platform: discovered.snapshot(directory).unwrap(),
            resolver: Default::default(),
            packages: IndexMap::from([(
                "example".to_string(),
                PackageSpec::new("*", vec![alpha_remote, beta_remote.clone()]),
            )]),
            groups: Default::default(),
        };
        ManifestFile::new(directory, manifest).save().unwrap();
        let mods = directory.join("mods");
        std::fs::create_dir_all(&mods).unwrap();
        std::fs::copy(&beta, mods.join("example.jar")).unwrap();
        Lockfile::new(
            directory,
            OrbitLockfile {
                meta: LockMeta {
                    mc_version: "1".to_string(),
                    modloader: "fabric".to_string(),
                    modloader_version: "1".to_string(),
                },
                packages: vec![PackageEntry {
                    mod_id: "example".to_string(),
                    version: "1.2.3-beta".to_string(),
                    sha1: crate::jar::compute_sha1(&beta).unwrap(),
                    sha256: crate::jar::compute_sha256(&beta).unwrap(),
                    sha512: crate::jar::compute_sha512(&beta).unwrap(),
                    filename: "example.jar".to_string(),
                    remotes: vec![beta_remote],
                    artifact_sources: vec![ArtifactSource::File {
                        path: ".orbit/sources/example-beta.jar".to_string(),
                    }],
                    dependencies: Vec::new(),
                    environment: Environment::Both,
                    provides: Vec::new(),
                    language_loader: None,
                    embedded_artifacts: Vec::new(),
                    bundled: Vec::new(),
                }],
            },
        )
        .save()
        .unwrap();
        crate::jar_cache::JarCache::open(directory.join("cache")).unwrap()
    }

    fn accept_transaction() -> InstallInteraction {
        InstallInteraction {
            confirm_install: Some(Box::new(|_| true)),
            ..InstallInteraction::default()
        }
    }

    #[tokio::test]
    async fn applying_a_policy_atomically_reselects_the_package() {
        let directory = tempfile::tempdir().unwrap();
        let cache = write_constraint_instance(directory.path());

        let report = apply_package_constraint(
            directory.path(),
            "example",
            PackageVersionPolicy::Comparison {
                operator: VersionComparison::Exact,
                version: "1.2.3-alpha".to_string(),
            },
            Some("all".to_string()),
            &[],
            &cache,
            false,
            accept_transaction(),
        )
        .await
        .unwrap();

        assert!(report.applied);
        assert_eq!(report.selected_version.as_deref(), Some("1.2.3-alpha"));
        assert_eq!(
            ManifestFile::open(directory.path()).unwrap().inner.packages["example"].version,
            "=1.2.3-alpha"
        );
        assert_eq!(
            Lockfile::open(directory.path())
                .unwrap()
                .inner
                .find("example")
                .unwrap()
                .version,
            "1.2.3-alpha"
        );
    }

    #[tokio::test]
    async fn a_satisfied_policy_is_persisted_without_rewriting_the_package() {
        let directory = tempfile::tempdir().unwrap();
        let cache = write_constraint_instance(directory.path());
        let jar_before = std::fs::read(directory.path().join("mods/example.jar")).unwrap();

        let report = apply_package_constraint(
            directory.path(),
            "example",
            PackageVersionPolicy::Comparison {
                operator: VersionComparison::Exact,
                version: "1.2.3".to_string(),
            },
            Some("all".to_string()),
            &[],
            &cache,
            false,
            accept_transaction(),
        )
        .await
        .unwrap();

        assert!(report.applied);
        assert!(report.transaction.installed.is_empty());
        assert_eq!(
            ManifestFile::open(directory.path()).unwrap().inner.packages["example"].version,
            "=1.2.3"
        );
        assert_eq!(
            std::fs::read(directory.path().join("mods/example.jar")).unwrap(),
            jar_before
        );
    }

    #[tokio::test]
    async fn suffix_expression_participates_in_the_same_solver_transaction() {
        let directory = tempfile::tempdir().unwrap();
        let cache = write_constraint_instance(directory.path());

        let report = apply_package_constraint(
            directory.path(),
            "example",
            PackageVersionPolicy::Comparison {
                operator: VersionComparison::Exact,
                version: "1.2.3".to_string(),
            },
            Some("all; intersect \"alpha\"".to_string()),
            &[],
            &cache,
            false,
            accept_transaction(),
        )
        .await
        .unwrap();

        assert!(report.applied);
        assert_eq!(report.selected_version.as_deref(), Some("1.2.3-alpha"));
        let manifest = ManifestFile::open(directory.path()).unwrap();
        assert_eq!(manifest.inner.packages["example"].version, "=1.2.3");
        assert_eq!(
            manifest.inner.packages["example"].suffix,
            "all; intersect \"alpha\""
        );
    }

    #[tokio::test]
    async fn omitted_suffix_preserves_the_existing_rule() {
        let directory = tempfile::tempdir().unwrap();
        let cache = write_constraint_instance(directory.path());
        let mut manifest = ManifestFile::open(directory.path()).unwrap();
        manifest.inner.packages["example"].suffix = "all; intersect not \"alpha\"".to_string();
        manifest.save().unwrap();

        let report = apply_package_constraint(
            directory.path(),
            "example",
            PackageVersionPolicy::Comparison {
                operator: VersionComparison::Exact,
                version: "1.2.3".to_string(),
            },
            None,
            &[],
            &cache,
            false,
            accept_transaction(),
        )
        .await
        .unwrap();

        assert!(report.applied);
        assert_eq!(report.suffix, "all; intersect not \"alpha\"");
        assert_eq!(
            ManifestFile::open(directory.path()).unwrap().inner.packages["example"].suffix,
            report.suffix
        );
    }

    #[tokio::test]
    async fn an_unsatisfiable_policy_leaves_manifest_lock_and_jar_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let cache = write_constraint_instance(directory.path());
        let manifest_before = std::fs::read(directory.path().join("orbit.toml")).unwrap();
        let lock_before = std::fs::read(directory.path().join("orbit.lock")).unwrap();
        let jar_before = std::fs::read(directory.path().join("mods/example.jar")).unwrap();

        let result = apply_package_constraint(
            directory.path(),
            "example",
            PackageVersionPolicy::Comparison {
                operator: VersionComparison::Exact,
                version: "9".to_string(),
            },
            Some("all".to_string()),
            &[],
            &cache,
            false,
            accept_transaction(),
        )
        .await;

        assert!(result.is_err());
        assert_eq!(
            std::fs::read(directory.path().join("orbit.toml")).unwrap(),
            manifest_before
        );
        assert_eq!(
            std::fs::read(directory.path().join("orbit.lock")).unwrap(),
            lock_before
        );
        assert_eq!(
            std::fs::read(directory.path().join("mods/example.jar")).unwrap(),
            jar_before
        );
    }

    #[tokio::test]
    async fn rejecting_the_transaction_does_not_persist_the_policy() {
        let directory = tempfile::tempdir().unwrap();
        let cache = write_constraint_instance(directory.path());
        let manifest_before = std::fs::read(directory.path().join("orbit.toml")).unwrap();
        let lock_before = std::fs::read(directory.path().join("orbit.lock")).unwrap();

        let report = apply_package_constraint(
            directory.path(),
            "example",
            PackageVersionPolicy::Comparison {
                operator: VersionComparison::Exact,
                version: "1.2.3-alpha".to_string(),
            },
            Some("all".to_string()),
            &[],
            &cache,
            false,
            InstallInteraction {
                confirm_install: Some(Box::new(|_| false)),
                ..InstallInteraction::default()
            },
        )
        .await
        .unwrap();

        assert!(!report.applied);
        assert_eq!(
            std::fs::read(directory.path().join("orbit.toml")).unwrap(),
            manifest_before
        );
        assert_eq!(
            std::fs::read(directory.path().join("orbit.lock")).unwrap(),
            lock_before
        );
    }
}
