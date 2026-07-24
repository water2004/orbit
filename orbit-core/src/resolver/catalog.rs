//! Completes the in-memory candidate catalog before exhaustive resolution.

use crate::lockfile::OrbitLockfile;
use crate::metadata::Environment;
use crate::providers::ModProvider;
use crate::resolver::graph::{ExclusionMap, required_candidate_packages};
use crate::resolver::types::{CandidateCatalog, CandidateVersion};

pub(crate) struct CatalogRequest<'a> {
    pub(crate) catalog: &'a mut CandidateCatalog,
    pub(crate) lockfile: &'a OrbitLockfile,
    pub(crate) providers: &'a [Box<dyn ModProvider>],
    pub(crate) minecraft_version: &'a str,
    pub(crate) loader: &'a str,
    pub(crate) exclusions: &'a ExclusionMap,
    pub(crate) target: Environment,
}

/// Fetch every lock-backed package referenced by candidate metadata.
///
/// Enumeration is only complete when the graph is closed before solving. Newly downloaded JAR
/// metadata can introduce more required packages, so this repeats until no catalog entry is added.
pub(crate) async fn complete_candidate_catalog(
    mut request: CatalogRequest<'_>,
) -> Result<(), String> {
    while fetch_missing_candidates(&mut request).await? {}
    Ok(())
}

async fn fetch_missing_candidates(request: &mut CatalogRequest<'_>) -> Result<bool, String> {
    let needed = required_candidate_packages(
        &request.catalog.candidates,
        request.exclusions,
        request.target,
    );

    let mut added = false;
    for package in needed {
        if request.catalog.candidates.contains_key(&package) {
            continue;
        }
        let Some(entry) = request.lockfile.find(&package) else {
            continue;
        };
        if entry.provider == "file" {
            continue;
        }
        let Some(project_id) = entry.source_project_id() else {
            continue;
        };
        let Some(provider) = crate::providers::find_provider(request.providers, &entry.provider)
        else {
            return Err(format!(
                "cannot fetch missing {} dependencies: provider is not configured",
                entry.provider
            ));
        };

        let versions = provider
            .get_versions(
                &project_id,
                Some(request.minecraft_version),
                Some(request.loader),
            )
            .await
            .map_err(|error| {
                format!(
                    "failed to query {} candidates for '{package}' (project {project_id}): \
                     {error}",
                    entry.provider
                )
            })?;

        let mut downloaded = Vec::new();
        let mut first_error = None;
        for resolved in versions {
            match crate::jar::download_and_parse(
                provider.artifact_downloader(),
                &resolved.download_url,
                &resolved.filename,
                &resolved.sha1,
                &resolved.sha512,
                request.loader,
            )
            .await
            {
                Ok(metadata) => {
                    downloaded.push((CandidateVersion::from_jar_metadata(metadata), resolved));
                }
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        if downloaded.is_empty() {
            if let Some(error) = first_error {
                return Err(format!(
                    "failed to fetch candidate metadata for '{package}' from {}: {error}",
                    entry.provider
                ));
            }
            continue;
        }

        for (candidate, resolved) in downloaded {
            request.catalog.record(package.clone(), candidate, resolved);
        }
        added = true;
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockfile::LockMeta;

    fn candidate_with_dependency(package: &str) -> CandidateVersion {
        CandidateVersion {
            jar_version: "1".to_string(),
            dependencies: vec![crate::metadata::ModDependency::required(package, "*").into()],
            environment: Default::default(),
            provides: Vec::new(),
            language_loader: None,
            embedded_artifacts: Vec::new(),
            bundled: Vec::new(),
        }
    }

    #[tokio::test]
    async fn closed_catalog_does_not_require_a_network_provider() {
        let mut catalog = CandidateCatalog::default();
        catalog
            .candidates
            .insert("a".to_string(), vec![candidate_with_dependency("b")]);
        catalog.candidates.insert(
            "b".to_string(),
            vec![CandidateVersion {
                dependencies: Vec::new(),
                ..candidate_with_dependency("unused")
            }],
        );
        let lockfile = OrbitLockfile {
            meta: LockMeta {
                mc_version: "1".to_string(),
                modloader: "forge".to_string(),
                modloader_version: "1".to_string(),
            },
            packages: Vec::new(),
        };

        complete_candidate_catalog(CatalogRequest {
            catalog: &mut catalog,
            lockfile: &lockfile,
            providers: &[],
            minecraft_version: "1",
            loader: "forge",
            exclusions: &ExclusionMap::new(),
            target: Environment::Both,
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn missing_fetch_provider_returns_an_error() {
        let mut catalog = CandidateCatalog::default();
        catalog
            .candidates
            .insert("a".to_string(), vec![candidate_with_dependency("b")]);
        let lockfile: OrbitLockfile = toml::from_str(
            r#"
[meta]
mc_version = "1"
modloader = "forge"
modloader_version = "1"

[[package]]
mod_id = "b"
version = "1"
sha256 = "unused"
provider = "modrinth"

[package.modrinth]
project_id = "b-project"
version_id = "b-version"
version = "1"
slug = "b"
"#,
        )
        .unwrap();

        let error = complete_candidate_catalog(CatalogRequest {
            catalog: &mut catalog,
            lockfile: &lockfile,
            providers: &[],
            minecraft_version: "1",
            loader: "forge",
            exclusions: &ExclusionMap::new(),
            target: Environment::Both,
        })
        .await
        .unwrap_err();

        assert_eq!(
            error,
            "cannot fetch missing modrinth dependencies: provider is not configured"
        );
    }
}
