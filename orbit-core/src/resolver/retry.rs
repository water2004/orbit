//! Runs PubGrub and expands the graph from remote metadata between failed attempts.

use std::collections::HashMap;

use pubgrub::{DefaultStringReporter, Reporter, SelectedDependencies};

use crate::lockfile::OrbitLockfile;
use crate::providers::ModProvider;
use crate::resolver::diagnostics::ResolutionTrace;
use crate::resolver::graph::{register_candidate_versions, required_candidate_packages};
use crate::resolver::provider::OrbitDependencyProvider;
use crate::resolver::types::{CandidateVersion, ImplantedCandidate};
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
                let added = fetch_missing_candidates(
                    provider,
                    candidates,
                    lockfile,
                    providers,
                    minecraft_version,
                    loader,
                )
                .await?;
                if !added {
                    return Err(DefaultStringReporter::report(&derivation_tree));
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

async fn fetch_missing_candidates(
    provider: &mut OrbitDependencyProvider,
    candidates: &mut HashMap<String, Vec<CandidateVersion>>,
    lockfile: &OrbitLockfile,
    providers: &[Box<dyn ModProvider>],
    minecraft_version: &str,
    loader: &str,
) -> Result<bool, String> {
    let needed = required_candidate_packages(candidates);
    eprintln!("    needed deps from candidates: {needed:?}");

    let mut added = false;
    for package in needed {
        if candidates.contains_key(&package) || is_implanted(lockfile, &package) {
            continue;
        }
        let Some(entry) = lockfile.find(&package) else {
            eprintln!("    {package} not in lockfile, skip");
            continue;
        };
        let Some(modrinth) = entry.modrinth.as_ref() else {
            eprintln!("    {package} has no Modrinth metadata, skip");
            continue;
        };
        let Some(mod_provider) = providers.first() else {
            return Err(
                "cannot fetch missing dependencies: no mod provider configured".to_string(),
            );
        };

        eprintln!(
            "    fetching dep {package} versions (project={})...",
            modrinth.project_id
        );
        let versions = match mod_provider
            .get_versions(&modrinth.project_id, Some(minecraft_version), Some(loader))
            .await
        {
            Ok(versions) => versions,
            Err(error) => {
                eprintln!("    ! API error for {package}: {error}");
                continue;
            }
        };

        let mut downloaded = Vec::new();
        for version in versions {
            let Ok(metadata) = crate::jar::download_and_parse(
                &version.download_url,
                &version.filename,
                &version.sha512,
                loader,
            )
            .await
            else {
                continue;
            };
            downloaded.push(CandidateVersion {
                jar_version: metadata.version,
                deps: metadata.dependencies,
                implanted: metadata
                    .implanted_mods
                    .into_iter()
                    .map(|implanted| ImplantedCandidate {
                        mod_id: implanted.mod_id,
                        version: implanted.version,
                        deps: implanted.dependencies,
                    })
                    .collect(),
            });
        }
        if downloaded.is_empty() {
            continue;
        }

        eprintln!("    downloaded {} versions for {package}", downloaded.len());
        register_candidate_versions(provider, &package, &downloaded, loader);
        candidates.entry(package).or_default().extend(downloaded);
        added = true;
    }
    Ok(added)
}

fn is_implanted(lockfile: &OrbitLockfile, package: &str) -> bool {
    lockfile.packages.iter().any(|entry| {
        entry
            .implanted
            .iter()
            .any(|implanted| implanted.name == package)
    })
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
                deps: Vec::new(),
                implanted: Vec::new(),
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
                deps: vec![("b".to_string(), "*".to_string(), true)],
                implanted: Vec::new(),
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

        let error =
            fetch_missing_candidates(&mut provider, &mut candidates, &lockfile, &[], "1", "forge")
                .await
                .unwrap_err();

        assert_eq!(
            error,
            "cannot fetch missing dependencies: no mod provider configured"
        );
    }
}
