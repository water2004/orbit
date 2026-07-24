//! Builds the complete in-memory package graph consumed by PubGrub.
//!
//! Every dependency referenced by known metadata is registered, even when it has
//! no known versions, so unsatisfied graphs become normal PubGrub derivations.

use std::collections::{HashMap, HashSet};

use pubgrub::Ranges;

use crate::lockfile::OrbitLockfile;
use crate::manifest::{DependencySpec, OrbitManifest};
use crate::resolver::provider::OrbitDependencyProvider;
use crate::resolver::types::{CandidateVersion, ImplantedCandidate};
use crate::versions::Version;

pub(crate) const ROOT_PACKAGE: &str = "___orbit_root___";

pub(crate) struct SolverGraph {
    pub(crate) provider: OrbitDependencyProvider,
    pub(crate) root_package: String,
    pub(crate) root_version: Version,
}

pub(crate) fn build_solver_graph(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    candidates: &HashMap<String, Vec<CandidateVersion>>,
) -> SolverGraph {
    let loader = &manifest.project.modloader;
    let mut provider = OrbitDependencyProvider::new();

    register_platform_packages(&mut provider, manifest);
    register_lockfile(&mut provider, lockfile, loader);
    register_candidate_map(&mut provider, candidates, loader);

    let root_package = ROOT_PACKAGE.to_string();
    let root_version = Version::zero();
    let root_dependencies = root_dependencies(manifest, candidates, loader);
    provider.add_package_versions(root_package.clone(), vec![root_version.clone()]);
    provider.add_package_deps(
        root_package.clone(),
        root_version.clone(),
        root_dependencies,
    );
    ensure_referenced_packages(&mut provider);

    SolverGraph {
        provider,
        root_package,
        root_version,
    }
}

pub(crate) fn register_platform_packages(
    provider: &mut OrbitDependencyProvider,
    manifest: &OrbitManifest,
) {
    let loader = &manifest.project.modloader;
    let minecraft = Version::parse(&manifest.project.mc_version, loader);
    provider.add_package_versions("minecraft".to_string(), vec![minecraft.clone()]);
    provider.add_package_deps("minecraft".to_string(), minecraft, vec![]);

    let loader_package = match loader.as_str() {
        "fabric" => "fabricloader",
        "quilt" => "quiltloader",
        other => other,
    };
    let loader_version = Version::parse(&manifest.project.modloader_version, loader);
    provider.add_package_versions(loader_package.to_string(), vec![loader_version.clone()]);
    provider.add_package_deps(loader_package.to_string(), loader_version.clone(), vec![]);
    if loader == "fabric" {
        provider.add_package_versions("fabric".to_string(), vec![loader_version.clone()]);
        provider.add_package_deps("fabric".to_string(), loader_version, vec![]);
    }

    let zero = Version::zero();
    provider.add_package_versions("java".to_string(), vec![zero.clone()]);
    provider.add_package_deps("java".to_string(), zero.clone(), vec![]);
    if loader == "fabric" {
        provider.add_package_versions("mixinextras".to_string(), vec![zero.clone()]);
        provider.add_package_deps("mixinextras".to_string(), zero, vec![]);
    }
}

pub(crate) fn register_candidate_versions(
    provider: &mut OrbitDependencyProvider,
    package: &str,
    candidates: &[CandidateVersion],
    loader: &str,
) {
    let existing = provider.versions.get(package).cloned().unwrap_or_default();
    let mut versions = Vec::new();

    for candidate in candidates {
        let version = Version::parse(&candidate.jar_version, loader);
        push_unique(&mut versions, version.clone());
        provider.add_package_deps(
            package.to_string(),
            version,
            parse_required_dependencies(&candidate.deps, loader),
        );

        for implanted in &candidate.implanted {
            register_implanted_candidate(provider, implanted, loader);
        }
    }

    for version in existing {
        push_unique(&mut versions, version);
    }
    provider.add_package_versions(package.to_string(), versions);
    ensure_referenced_packages(provider);
}

pub(crate) fn required_candidate_packages(
    candidates: &HashMap<String, Vec<CandidateVersion>>,
) -> Vec<String> {
    let mut required = HashSet::new();
    for versions in candidates.values() {
        for candidate in versions {
            collect_required_names(&candidate.deps, &mut required);
            for implanted in &candidate.implanted {
                collect_required_names(&implanted.deps, &mut required);
            }
        }
    }
    let mut required: Vec<_> = required.into_iter().collect();
    required.sort();
    required
}

fn register_lockfile(
    provider: &mut OrbitDependencyProvider,
    lockfile: &OrbitLockfile,
    loader: &str,
) {
    for entry in &lockfile.packages {
        let version = Version::parse(&entry.version, loader);
        let dependencies = entry
            .dependencies
            .iter()
            .map(|dependency| {
                (
                    dependency.name.clone(),
                    Version::parse_constraint(&dependency.version, loader),
                )
            })
            .collect();
        provider.add_package_versions(entry.mod_id.clone(), vec![version.clone()]);
        provider.add_package_deps(entry.mod_id.clone(), version, dependencies);

        for implanted in &entry.implanted {
            let version = Version::parse(&implanted.version, loader);
            let dependencies = implanted
                .dependencies
                .iter()
                .map(|dependency| {
                    (
                        dependency.name.clone(),
                        Version::parse_constraint(&dependency.version, loader),
                    )
                })
                .collect();
            provider.add_package_versions(implanted.name.clone(), vec![version.clone()]);
            provider.add_package_deps(implanted.name.clone(), version, dependencies);
        }
    }
}

fn register_candidate_map(
    provider: &mut OrbitDependencyProvider,
    candidates: &HashMap<String, Vec<CandidateVersion>>,
    loader: &str,
) {
    for (package, versions) in candidates {
        register_candidate_versions(provider, package, versions, loader);
    }
}

fn register_implanted_candidate(
    provider: &mut OrbitDependencyProvider,
    implanted: &ImplantedCandidate,
    loader: &str,
) {
    let version = Version::parse(&implanted.version, loader);
    let mut versions = provider
        .versions
        .get(&implanted.mod_id)
        .cloned()
        .unwrap_or_default();
    push_unique(&mut versions, version.clone());
    provider.add_package_versions(implanted.mod_id.clone(), versions);
    provider.add_package_deps(
        implanted.mod_id.clone(),
        version,
        parse_required_dependencies(&implanted.deps, loader),
    );
}

fn parse_required_dependencies(
    dependencies: &[(String, String, bool)],
    loader: &str,
) -> Vec<(String, Ranges<Version>)> {
    dependencies
        .iter()
        .filter(|(_, _, required)| *required)
        .map(|(name, constraint, _)| (name.clone(), Version::parse_constraint(constraint, loader)))
        .collect()
}

fn collect_required_names(dependencies: &[(String, String, bool)], required: &mut HashSet<String>) {
    for (name, _, is_required) in dependencies {
        if *is_required && !is_builtin_package(name) {
            required.insert(name.clone());
        }
    }
}

fn root_dependencies(
    manifest: &OrbitManifest,
    candidates: &HashMap<String, Vec<CandidateVersion>>,
    loader: &str,
) -> Vec<(String, Ranges<Version>)> {
    let mut dependencies = Vec::new();
    for (name, spec) in &manifest.dependencies {
        let constraint = if candidates.contains_key(name) {
            Ranges::full()
        } else {
            match spec {
                DependencySpec::Short(version) => Version::parse_constraint(version, loader),
                DependencySpec::Full { version, .. } => {
                    Version::parse_constraint(version.as_deref().unwrap_or("*"), loader)
                }
            }
        };
        dependencies.push((name.clone(), constraint));
    }
    for package in candidates.keys() {
        if !manifest.dependencies.contains_key(package) {
            dependencies.push((package.clone(), Ranges::full()));
        }
    }
    dependencies
}

fn ensure_referenced_packages(provider: &mut OrbitDependencyProvider) {
    let referenced: HashSet<_> = provider
        .dependencies
        .values()
        .flat_map(|dependencies| dependencies.iter().map(|(package, _)| package.clone()))
        .collect();
    for package in referenced {
        provider.versions.entry(package).or_default();
    }
}

fn push_unique(versions: &mut Vec<Version>, version: Version) {
    if !versions.contains(&version) {
        versions.push(version);
    }
}

fn is_builtin_package(package: &str) -> bool {
    matches!(
        package,
        "java" | "mixinextras" | "minecraft" | "fabric" | "fabricloader" | "quiltloader"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(version: &str, dependencies: Vec<(String, String, bool)>) -> CandidateVersion {
        CandidateVersion {
            jar_version: version.to_string(),
            deps: dependencies,
            implanted: Vec::new(),
        }
    }

    #[test]
    fn candidate_versions_precede_and_deduplicate_existing_versions() {
        let mut provider = OrbitDependencyProvider::new();
        provider.add_package_versions(
            "example".to_string(),
            vec![Version::Generic("1".to_string())],
        );
        let candidates = vec![candidate("2", Vec::new()), candidate("1", Vec::new())];

        register_candidate_versions(&mut provider, "example", &candidates, "forge");

        assert_eq!(
            provider.versions["example"],
            vec![
                Version::Generic("2".to_string()),
                Version::Generic("1".to_string())
            ]
        );
    }

    #[test]
    fn registering_new_candidates_prepares_their_unknown_dependencies() {
        let mut provider = OrbitDependencyProvider::new();
        let candidates = vec![candidate(
            "2",
            vec![("transitive".to_string(), "*".to_string(), true)],
        )];

        register_candidate_versions(&mut provider, "example", &candidates, "forge");

        assert_eq!(provider.versions["transitive"], Vec::<Version>::new());
    }
}
