//! Remote candidate inventory for one managed logical package.

use std::path::Path;

use crate::error::OrbitError;
use crate::lockfile::{LockMeta, OrbitLockfile};
use crate::progress::ProgressReporter;
use crate::providers::ModProvider;
use crate::versions::Version;
use crate::workspace::{Lockfile, ManifestFile};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageVersionCandidate {
    /// Content identity retained only for deterministic internal ordering.
    pub(crate) identity: String,
    pub version: String,
    pub numeric_core: Option<String>,
    pub string_tokens: Vec<String>,
    pub numeric_filterable: bool,
    pub numeric_error: Option<String>,
    pub sources: Vec<String>,
    pub details: String,
    pub selected: bool,
    pub matches_constraint: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageVersionsReport {
    pub package: String,
    pub constraint: String,
    pub string: String,
    pub policy: crate::package_constraint::PackageVersionPolicy,
    pub selected_version: Option<String>,
    pub candidates: Vec<PackageVersionCandidate>,
}

pub async fn list_package_versions(
    instance_dir: &Path,
    package: &str,
    providers: &[Box<dyn ModProvider>],
    storage: crate::version_repository::CandidateStorage<'_>,
    progress: Option<ProgressReporter>,
) -> Result<PackageVersionsReport, OrbitError> {
    let manifest = ManifestFile::open(instance_dir)?;
    let platform = crate::platform::Platform::load(instance_dir, &manifest.inner)?;
    let lockfile = Lockfile::open_or_default(
        instance_dir,
        LockMeta {
            mc_version: manifest.inner.project.mc_version.clone(),
            modloader: manifest.inner.project.modloader.clone(),
            modloader_version: manifest.inner.project.modloader_version.clone(),
        },
    )?;
    let specification = manifest
        .inner
        .packages
        .get(package)
        .ok_or_else(|| OrbitError::ModNotFound(package.to_string()))?;
    let loader = platform.loader;
    let discovery_lock = OrbitLockfile {
        meta: lockfile.inner.meta.clone(),
        packages: Vec::new(),
    };
    let catalog = crate::outdated::download_candidate_catalog(
        crate::outdated::CandidateDiscoveryInput {
            instance_dir,
            providers,
            additional_remotes: &[],
            lockfile: &discovery_lock,
            mc_version: &manifest.inner.project.mc_version,
            loader,
            java_feature: platform.minecraft_version.java_version,
            storage,
            progress,
        },
        &specification.remotes,
    )
    .await?;
    let selected = lockfile.inner.find(package);
    let range = Version::parse_constraint(&specification.version, loader);
    let string_expression = crate::VersionStringRule::parse(&specification.string)?;
    let mut candidates = catalog
        .candidates
        .get(package)
        .into_iter()
        .flatten()
        .map(|candidate| {
            let version = Version::parse(&candidate.jar_version, loader);
            let numeric = version.numeric_analysis();
            PackageVersionCandidate {
                identity: candidate.id.clone(),
                numeric_core: numeric.numeric_core().map(|core| core.join(".")),
                string_tokens: version.string_tokens(),
                numeric_filterable: numeric.numeric_filterable(),
                numeric_error: numeric.reason().map(str::to_string),
                version: candidate.jar_version.clone(),
                sources: candidate.display_sources.clone(),
                details: candidate_details(candidate),
                selected: selected.is_some_and(|entry| {
                    candidate
                        .id
                        .strip_prefix("sha512:")
                        .is_some_and(|hash| hash.eq_ignore_ascii_case(&entry.sha512))
                }),
                matches_constraint: (!numeric.numeric_filterable() || range.contains(&version))
                    && string_expression.matches(&candidate.jar_version),
            }
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        let left_version = Version::parse(&left.version, loader);
        let right_version = Version::parse(&right.version, loader);
        right_version
            .cmp_precedence(&left_version)
            .then_with(|| right_version.cmp(&left_version))
            .then_with(|| left.details.cmp(&right.details))
            .then_with(|| left.identity.cmp(&right.identity))
    });
    if candidates.is_empty() {
        let declared = catalog
            .requested_packages
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        return Err(OrbitError::Other(anyhow::anyhow!(
            "package '{package}' has no remote candidate declaring that mod_id{}",
            if declared.is_empty() {
                String::new()
            } else {
                format!("; configured remotes declared: {declared}")
            }
        )));
    }

    Ok(PackageVersionsReport {
        package: package.to_string(),
        constraint: specification.version.clone(),
        string: string_expression.canonical(),
        policy: crate::package_constraint::PackageVersionPolicy::from_requirement(
            &specification.version,
        ),
        selected_version: selected.map(|entry| entry.version.clone()),
        candidates,
    })
}

fn candidate_details(candidate: &crate::resolver::types::CandidateVersion) -> String {
    let dependency_count = candidate
        .dependencies
        .iter()
        .flat_map(crate::metadata::DependencyExpression::relations)
        .filter(|dependency| dependency.kind.installs_target())
        .count();
    let mut details = Vec::new();
    if dependency_count > 0 {
        details.push(format!(
            "{dependency_count} dependency constraint{}",
            if dependency_count == 1 { "" } else { "s" }
        ));
    }
    if candidate.environment != crate::metadata::Environment::Both {
        details.push(format!("environment {}", candidate.environment.as_str()));
    }
    if !candidate.bundled.is_empty() {
        details.push(format!(
            "{} bundled module{}",
            candidate.bundled.len(),
            if candidate.bundled.len() == 1 {
                ""
            } else {
                "s"
            }
        ));
    }
    if details.is_empty() {
        "no additional JAR requirements".to_string()
    } else {
        details.join("; ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn precedence_sort_keeps_suffix_variants_adjacent_and_deterministic() {
        let loader = crate::loader::LoaderKind::Fabric;
        let mut versions = ["1.2.3-alpha", "1.2.4-alpha", "1.2.3-beta"];
        versions.sort_by(|left, right| {
            let left = Version::parse(left, loader);
            let right = Version::parse(right, loader);
            right.cmp_precedence(&left).then_with(|| right.cmp(&left))
        });

        assert_eq!(versions, ["1.2.4-alpha", "1.2.3-beta", "1.2.3-alpha"]);
        assert_eq!(
            Version::parse("1.2.3-alpha", loader)
                .cmp_precedence(&Version::parse("1.2.3-beta", loader)),
            std::cmp::Ordering::Equal
        );
    }

    #[tokio::test]
    async fn local_remote_inventory_uses_jar_versions_and_does_not_require_a_lock() {
        let directory = tempfile::tempdir().unwrap();
        write_fabric_jar(&directory.path().join("example-alpha.jar"), "1.2.3-alpha");
        write_fabric_jar(&directory.path().join("example-newer.jar"), "1.2.4-preview");
        let (minecraft_sha256, loader_sha256) = write_platform(directory.path());
        std::fs::write(
            directory.path().join("orbit.toml"),
            format!(
                r#"
[project]
name = "test"
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.16"
[platform]
minecraft_jar = {{ path = "minecraft.jar", sha256 = "{minecraft_sha256}" }}
loader_jar = {{ path = "loader.jar", sha256 = "{loader_sha256}" }}
runtime_jars = []
physical_environment = "client"
[packages]
example = {{ version = ">=1.2.3", remotes = [
  {{ type = "file", path = "example-alpha.jar" }},
  {{ type = "file", path = "example-newer.jar" }},
] }}
"#
            ),
        )
        .unwrap();
        let cache = crate::jar_cache::JarCache::open(directory.path().join("cache")).unwrap();
        let repository =
            crate::version_repository::VersionRepository::open(directory.path().join("repository"))
                .unwrap();

        let report = list_package_versions(
            directory.path(),
            "example",
            &[],
            crate::version_repository::CandidateStorage::new(&cache, &repository),
            None,
        )
        .await
        .unwrap();

        assert_eq!(report.selected_version, None);
        assert_eq!(
            report
                .candidates
                .iter()
                .map(|candidate| candidate.version.as_str())
                .collect::<Vec<_>>(),
            ["1.2.4-preview", "1.2.3-alpha"]
        );
        assert!(
            report
                .candidates
                .iter()
                .all(|candidate| candidate.matches_constraint)
        );
    }

    #[tokio::test]
    async fn opaque_versions_bypass_only_numeric_filtering() {
        let directory = tempfile::tempdir().unwrap();
        write_fabric_jar(&directory.path().join("example.jar"), "release-vNext");
        let (minecraft_sha256, loader_sha256) = write_platform(directory.path());
        std::fs::write(
            directory.path().join("orbit.toml"),
            format!(
                r#"
[project]
name = "test"
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.16"
[platform]
minecraft_jar = {{ path = "minecraft.jar", sha256 = "{minecraft_sha256}" }}
loader_jar = {{ path = "loader.jar", sha256 = "{loader_sha256}" }}
runtime_jars = []
physical_environment = "client"
[packages]
example = {{ version = "=999", string = 'all; intersect not contains(i"release")', remotes = [
  {{ type = "file", path = "example.jar" }},
] }}
"#
            ),
        )
        .unwrap();
        let cache = crate::jar_cache::JarCache::open(directory.path().join("cache")).unwrap();
        let repository =
            crate::version_repository::VersionRepository::open(directory.path().join("repository"))
                .unwrap();

        let report = list_package_versions(
            directory.path(),
            "example",
            &[],
            crate::version_repository::CandidateStorage::new(&cache, &repository),
            None,
        )
        .await
        .unwrap();
        let candidate = &report.candidates[0];
        assert!(!candidate.numeric_filterable);
        assert!(
            candidate
                .numeric_error
                .as_deref()
                .unwrap()
                .contains("opaque")
        );
        assert_eq!(candidate.numeric_core, None);
        assert!(
            candidate
                .string_tokens
                .contains(&"release-vNext".to_string())
        );
        assert!(!candidate.matches_constraint);

        let mut manifest = ManifestFile::open(directory.path()).unwrap();
        manifest.inner.packages["example"].string = "all".to_string();
        manifest.save().unwrap();
        let report = list_package_versions(
            directory.path(),
            "example",
            &[],
            crate::version_repository::CandidateStorage::new(&cache, &repository),
            None,
        )
        .await
        .unwrap();
        assert!(report.candidates[0].matches_constraint);
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

    fn write_platform(root: &Path) -> (String, String) {
        let minecraft = root.join("minecraft.jar");
        let file = std::fs::File::create(&minecraft).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        archive
            .start_file("version.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        write!(
            archive,
            r#"{{"id":"1.20.1","name":"1.20.1","world_version":3465,"protocol_version":763,"pack_version":{{"resource":15,"data":15}},"java_version":17,"stable":true}}"#
        )
        .unwrap();
        archive.finish().unwrap();

        let loader = root.join("loader.jar");
        let file = std::fs::File::create(&loader).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        archive
            .start_file("fabric.mod.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        write!(
            archive,
            r#"{{"schemaVersion":1,"id":"fabricloader","version":"0.16","name":"Fabric Loader"}}"#
        )
        .unwrap();
        archive.finish().unwrap();

        (
            crate::jar::compute_sha256(&minecraft).unwrap(),
            crate::jar::compute_sha256(&loader).unwrap(),
        )
    }
}
