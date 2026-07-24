//! Runs PubGrub and expands the graph from remote metadata between failed attempts.

use std::collections::HashMap;

use pubgrub::SelectedDependencies;

use crate::lockfile::OrbitLockfile;
use crate::metadata::Environment;
use crate::providers::ModProvider;
use crate::resolver::diagnostics::{ResolutionTrace, describe_no_solution};
use crate::resolver::graph::{
    ExclusionMap, OverrideMap, register_candidate_versions, required_candidate_packages,
};
use crate::resolver::ordering::register_ordering_cycles;
use crate::resolver::provider::OrbitDependencyProvider;
use crate::resolver::types::CandidateVersion;
use crate::versions::Version;

pub(crate) struct SolveOutcome {
    pub(crate) solution: SelectedDependencies<String, Version>,
    pub(crate) trace: ResolutionTrace,
}

pub(crate) struct SolveRequest<'a> {
    pub(crate) provider: &'a mut OrbitDependencyProvider,
    pub(crate) root_package: &'a str,
    pub(crate) root_version: &'a Version,
    pub(crate) candidates: &'a mut HashMap<String, Vec<CandidateVersion>>,
    pub(crate) lockfile: &'a OrbitLockfile,
    pub(crate) providers: &'a [Box<dyn ModProvider>],
    pub(crate) minecraft_version: &'a str,
    pub(crate) loader: &'a str,
    pub(crate) exclusions: &'a ExclusionMap,
    pub(crate) overrides: &'a OverrideMap,
    pub(crate) target: Environment,
}

pub(crate) async fn solve_with_fetch_retry(
    request: SolveRequest<'_>,
) -> Result<SolveOutcome, String> {
    let SolveRequest {
        provider,
        root_package,
        root_version,
        candidates,
        lockfile,
        providers,
        minecraft_version,
        loader,
        exclusions,
        overrides,
        target,
    } = request;

    loop {
        let watched_candidates = candidates.iter().filter_map(|(package, versions)| {
            versions.first().map(|candidate| {
                (
                    package.clone(),
                    Version::parse(&candidate.jar_version, loader),
                )
            })
        });
        let mut trace = ResolutionTrace::new(watched_candidates);

        match pubgrub::resolve_with_observer(
            provider,
            root_package.to_string(),
            root_version.clone(),
            &mut trace,
        ) {
            Ok(solution) => return Ok(SolveOutcome { solution, trace }),
            Err(pubgrub::PubGrubError::NoSolution(derivation_tree)) => {
                let added = fetch_missing_candidates(FetchRequest {
                    provider,
                    candidates,
                    lockfile,
                    providers,
                    minecraft_version,
                    loader,
                    exclusions,
                    overrides,
                    target,
                })
                .await?;
                if !added {
                    return Err(describe_no_solution(&derivation_tree));
                }
            }
            Err(pubgrub::PubGrubError::ErrorChoosingVersion { package, source: _ }) => {
                return Err(format!(
                    "internal error: no version of '{package}' matches constraint"
                ));
            }
            Err(pubgrub::PubGrubError::ErrorRetrievingDependencies {
                package,
                version,
                source,
            }) => {
                return Err(format!(
                    "internal error: deps of '{package}' v{version}: {source}"
                ));
            }
            Err(pubgrub::PubGrubError::ErrorInShouldCancel(error)) => {
                return Err(error.to_string());
            }
        }
    }
}

struct FetchRequest<'a> {
    provider: &'a mut OrbitDependencyProvider,
    candidates: &'a mut HashMap<String, Vec<CandidateVersion>>,
    lockfile: &'a OrbitLockfile,
    providers: &'a [Box<dyn ModProvider>],
    minecraft_version: &'a str,
    loader: &'a str,
    exclusions: &'a ExclusionMap,
    overrides: &'a OverrideMap,
    target: Environment,
}

async fn fetch_missing_candidates(request: FetchRequest<'_>) -> Result<bool, String> {
    let FetchRequest {
        provider,
        candidates,
        lockfile,
        providers,
        minecraft_version,
        loader,
        exclusions,
        overrides,
        target,
    } = request;
    let needed = required_candidate_packages(candidates, exclusions, target);

    let mut added = false;
    for package in needed {
        if candidates.contains_key(&package) || is_bundled(lockfile, &package) {
            continue;
        }
        let Some(entry) = lockfile.find(&package) else {
            continue;
        };
        if entry.provider == "file" {
            continue;
        }
        let Some(project_id) = entry.source_project_id() else {
            continue;
        };
        let Some(mod_provider) = crate::providers::find_provider(providers, &entry.provider) else {
            return Err(format!(
                "cannot fetch missing {} dependencies: provider is not configured",
                entry.provider
            ));
        };

        let versions = mod_provider
            .get_versions(&project_id, Some(minecraft_version), Some(loader))
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
        for version in versions {
            match crate::jar::download_and_parse(
                &version.download_url,
                &version.filename,
                &version.sha1,
                &version.sha512,
                loader,
            )
            .await
            {
                Ok(metadata) => downloaded.push(CandidateVersion::from_jar_metadata(metadata)),
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

        register_candidate_versions(
            provider,
            &package,
            &downloaded,
            loader,
            exclusions,
            overrides,
            target,
        );
        candidates.entry(package).or_default().extend(downloaded);
        added = true;
    }
    if added {
        register_ordering_cycles(
            provider, lockfile, candidates, loader, exclusions, overrides, target,
        );
    }
    Ok(added)
}

fn is_bundled(lockfile: &OrbitLockfile, package: &str) -> bool {
    fn contains(mods: &[crate::lockfile::BundledMod], package: &str) -> bool {
        mods.iter()
            .any(|metadata| metadata.mod_id == package || contains(&metadata.bundled, package))
    }
    lockfile
        .packages
        .iter()
        .any(|entry| contains(&entry.bundled, package))
}

#[cfg(test)]
mod tests {
    use pubgrub::Ranges;

    use super::*;
    use crate::lockfile::LockMeta;

    fn version(value: &str) -> Version {
        Version::Generic(value.to_string())
    }

    #[tokio::test]
    async fn successful_resolution_does_not_require_a_network_provider() {
        let mut provider = OrbitDependencyProvider::new();
        provider.add_package_versions("root".to_string(), vec![version("1")]);
        provider.add_package_versions("a".to_string(), vec![version("1")]);
        provider.add_package_deps(
            "root".to_string(),
            version("1"),
            vec![("a".to_string(), Ranges::full())],
        );
        provider.add_package_deps("a".to_string(), version("1"), vec![]);

        let mut candidates = HashMap::from([(
            "a".to_string(),
            vec![CandidateVersion {
                jar_version: "1".to_string(),
                dependencies: Vec::new(),
                environment: Default::default(),
                provides: Vec::new(),
                language_loader: None,
                embedded_artifacts: Vec::new(),
                bundled: Vec::new(),
            }],
        )]);
        let lockfile = OrbitLockfile {
            meta: LockMeta {
                mc_version: "1".to_string(),
                modloader: "forge".to_string(),
                modloader_version: "1".to_string(),
            },
            packages: Vec::new(),
        };
        let root_version = version("1");

        let outcome = solve_with_fetch_retry(SolveRequest {
            provider: &mut provider,
            root_package: "root",
            root_version: &root_version,
            candidates: &mut candidates,
            lockfile: &lockfile,
            providers: &[],
            minecraft_version: "1",
            loader: "forge",
            exclusions: &ExclusionMap::new(),
            overrides: &OverrideMap::new(),
            target: Environment::Both,
        })
        .await
        .unwrap();

        assert_eq!(outcome.solution.get(&"a".to_string()), Some(&version("1")));
    }

    #[tokio::test]
    async fn missing_fetch_provider_returns_an_error() {
        let mut provider = OrbitDependencyProvider::new();
        let mut candidates = HashMap::from([(
            "a".to_string(),
            vec![CandidateVersion {
                jar_version: "1".to_string(),
                dependencies: vec![crate::metadata::ModDependency::required("b", "*").into()],
                environment: Default::default(),
                provides: Vec::new(),
                language_loader: None,
                embedded_artifacts: Vec::new(),
                bundled: Vec::new(),
            }],
        )]);
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

        let error = fetch_missing_candidates(FetchRequest {
            provider: &mut provider,
            candidates: &mut candidates,
            lockfile: &lockfile,
            providers: &[],
            minecraft_version: "1",
            loader: "forge",
            exclusions: &ExclusionMap::new(),
            overrides: &OverrideMap::new(),
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
