//! Typed instance settings stored in orbit.toml, independent of global config.

use std::path::Path;

use crate::{GlobalConfig, ManifestFile, OrbitError, manifest::ResolverConfig};

#[derive(Debug, Clone)]
pub struct CatalogOption {
    pub id: &'static str,
    pub name: &'static str,
    /// Credentials are configured; this is not an online authentication check.
    pub available: bool,
}

#[derive(Debug, Clone)]
pub struct CatalogSettings {
    pub catalogs: Vec<String>,
    pub options: Vec<CatalogOption>,
}

fn settings(catalogs: Vec<String>, config: &GlobalConfig) -> CatalogSettings {
    let options = [
        ("modrinth", "Modrinth", true),
        (
            "curseforge",
            "CurseForge",
            config
                .auth
                .curseforge_api_key
                .as_deref()
                .is_some_and(|key| !key.trim().is_empty()),
        ),
    ]
    .into_iter()
    .map(|(id, name, available)| CatalogOption {
        id,
        name,
        available,
    })
    .collect();
    CatalogSettings { catalogs, options }
}

pub fn list_catalogs(
    instance: &Path,
    config: &GlobalConfig,
) -> Result<CatalogSettings, OrbitError> {
    Ok(settings(
        ManifestFile::open(instance)?.inner.resolver.catalogs,
        config,
    ))
}

/// Search outside a workspace retains the default catalog; malformed manifests
/// must still fail rather than silently changing the search scope.
pub fn discovery_catalogs(instance: &Path) -> Result<Vec<String>, OrbitError> {
    match ManifestFile::open(instance) {
        Ok(manifest) => Ok(manifest.inner.resolver.catalogs),
        Err(OrbitError::ManifestNotFound) => Ok(ResolverConfig::default().catalogs),
        Err(error) => Err(error),
    }
}

fn set_catalogs(
    instance: &Path,
    catalogs: Vec<String>,
    config: &GlobalConfig,
    dry_run: bool,
) -> Result<CatalogSettings, OrbitError> {
    let mut manifest = ManifestFile::open(instance)?;
    if catalogs.is_empty() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "at least one discovery catalog is required"
        )));
    }
    let mut unique = Vec::new();
    for catalog in catalogs {
        if !unique.contains(&catalog) {
            unique.push(catalog);
        }
    }
    // Use the same validation (including mandatory credentials) as real search.
    crate::providers::create_providers(&unique, config)?;
    if manifest.inner.resolver.catalogs != unique {
        manifest.inner.resolver.catalogs = unique.clone();
        if !dry_run {
            manifest.save()?;
        }
    }
    Ok(settings(unique, config))
}

#[derive(Debug, Clone, Copy)]
pub enum InstanceConfigKey {
    ResolverCatalogs,
}

impl InstanceConfigKey {
    pub fn parse(key: &str) -> Result<Self, OrbitError> {
        match key {
            "resolver.catalogs" => Ok(Self::ResolverCatalogs),
            _ => Err(OrbitError::Other(anyhow::anyhow!(
                "unknown instance configuration key '{key}'; run 'orbit config --scope instance list' to see supported keys"
            ))),
        }
    }

    /// A missing value resets to the documented default, never an empty catalog.
    pub fn set(
        self,
        instance: &Path,
        value: Option<&str>,
        config: &GlobalConfig,
        dry_run: bool,
    ) -> Result<CatalogSettings, OrbitError> {
        match self {
            Self::ResolverCatalogs => {
                let catalogs = value.map_or_else(
                    || ResolverConfig::default().catalogs,
                    |value| {
                        value
                            .split(',')
                            .map(|part| part.trim().to_string())
                            .collect()
                    },
                );
                set_catalogs(instance, catalogs, config, dry_run)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        let manifest = crate::OrbitManifest {
            project: crate::manifest::ProjectMeta {
                name: "catalog-test".into(),
                mc_version: "26.2".into(),
                modloader: "fabric".into(),
                modloader_version: "0.19.5".into(),
                description: None,
                authors: None,
                version: None,
            },
            platform: crate::PlatformSnapshot {
                minecraft_jar: crate::PlatformArtifact {
                    path: "minecraft.jar".into(),
                    sha256: "a".repeat(64),
                },
                loader_jar: crate::PlatformArtifact {
                    path: "fabric.jar".into(),
                    sha256: "b".repeat(64),
                },
                runtime_jars: vec![],
                physical_environment: crate::metadata::Environment::Client,
            },
            resolver: ResolverConfig::default(),
            packages: Default::default(),
            groups: Default::default(),
        };
        ManifestFile::new(directory.path(), manifest)
            .save()
            .unwrap();
        directory
    }

    #[test]
    fn persists_only_catalogs_and_keeps_lock_bytes() {
        let directory = fixture();
        let mut config = GlobalConfig::default();
        config.auth.curseforge_api_key = Some("private-test-key".into());
        std::fs::write(directory.path().join("orbit.lock"), b"untouched").unwrap();
        let before = ManifestFile::open(directory.path()).unwrap().inner;
        let report = set_catalogs(
            directory.path(),
            vec!["curseforge".into(), "modrinth".into(), "curseforge".into()],
            &config,
            false,
        )
        .unwrap();
        assert_eq!(report.catalogs, ["curseforge", "modrinth"]);
        let mut after = ManifestFile::open(directory.path()).unwrap().inner;
        after.resolver = before.resolver.clone();
        assert_eq!(
            after.to_toml_string().unwrap(),
            before.to_toml_string().unwrap()
        );
        assert_eq!(
            std::fs::read(directory.path().join("orbit.lock")).unwrap(),
            b"untouched"
        );
        assert!(
            !std::fs::read_to_string(directory.path().join("orbit.toml"))
                .unwrap()
                .contains("private-test-key")
        );
        assert_eq!(
            discovery_catalogs(directory.path()).unwrap(),
            report.catalogs
        );
    }

    #[test]
    fn invalid_or_unauthenticated_updates_do_not_write() {
        let directory = fixture();
        let config = GlobalConfig::default();
        let before = std::fs::read(directory.path().join("orbit.toml")).unwrap();
        for catalogs in [vec![], vec!["file".into()], vec!["curseforge".into()]] {
            assert!(set_catalogs(directory.path(), catalogs, &config, false).is_err());
            assert_eq!(
                std::fs::read(directory.path().join("orbit.toml")).unwrap(),
                before
            );
        }
        let report = list_catalogs(directory.path(), &config).unwrap();
        assert_eq!(report.catalogs, ["modrinth"]);
        assert!(!report.options[1].available);
    }

    #[test]
    fn dry_run_does_not_write_and_bad_manifests_do_not_fall_back() {
        let directory = fixture();
        let mut config = GlobalConfig::default();
        config.auth.curseforge_api_key = Some("test-key".into());
        let report =
            set_catalogs(directory.path(), vec!["curseforge".into()], &config, true).unwrap();
        assert_eq!(report.catalogs, ["curseforge"]);
        assert_eq!(discovery_catalogs(directory.path()).unwrap(), ["modrinth"]);
        std::fs::write(directory.path().join("orbit.toml"), "invalid").unwrap();
        assert!(discovery_catalogs(directory.path()).is_err());
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(discovery_catalogs(empty.path()).unwrap(), ["modrinth"]);
    }

    #[test]
    fn typed_key_parses_list_values_and_resets_to_default() {
        let directory = fixture();
        let mut config = GlobalConfig::default();
        config.auth.curseforge_api_key = Some("test-key".into());
        let key = InstanceConfigKey::parse("resolver.catalogs").unwrap();
        let report = key
            .set(
                directory.path(),
                Some("curseforge, modrinth,curseforge"),
                &config,
                false,
            )
            .unwrap();
        assert_eq!(report.catalogs, ["curseforge", "modrinth"]);
        let before = std::fs::read(directory.path().join("orbit.toml")).unwrap();
        for invalid in ["", "modrinth,", "invalid"] {
            assert!(
                key.set(directory.path(), Some(invalid), &config, false)
                    .is_err()
            );
            assert_eq!(
                std::fs::read(directory.path().join("orbit.toml")).unwrap(),
                before
            );
        }
        assert!(InstanceConfigKey::parse("auth.curseforge-api-key").is_err());
        assert_eq!(
            key.set(directory.path(), None, &config, false)
                .unwrap()
                .catalogs,
            ["modrinth"]
        );
        assert_eq!(discovery_catalogs(directory.path()).unwrap(), ["modrinth"]);
    }
}
