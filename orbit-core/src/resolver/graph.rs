//! Builds the complete in-memory package graph consumed by PubGrub.
//!
//! Every dependency referenced by known metadata is registered, even when it has
//! no known versions, so unsatisfied graphs become normal PubGrub derivations.

use std::collections::{HashMap, HashSet};

use pubgrub::Ranges;

use crate::lockfile::OrbitLockfile;
use crate::manifest::OrbitManifest;
use crate::resolver::provider::OrbitDependencyProvider;
use crate::resolver::types::{CandidateVersion, ImplantedCandidate};
use crate::versions::Version;

pub(crate) const ROOT_PACKAGE: &str = "___orbit_root___";
pub(crate) type ExclusionMap = HashMap<String, HashSet<String>>;
pub(crate) type OverrideMap = HashMap<String, String>;

pub(crate) struct SolverGraph {
    pub(crate) provider: OrbitDependencyProvider,
    pub(crate) root_package: String,
    pub(crate) root_version: Version,
    pub(crate) exclusions: ExclusionMap,
    pub(crate) overrides: OverrideMap,
}

pub(crate) fn build_solver_graph(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    candidates: &HashMap<String, Vec<CandidateVersion>>,
) -> SolverGraph {
    let loader = &manifest.project.modloader;
    let mut provider = OrbitDependencyProvider::new();
    let exclusions = manifest_exclusions(manifest);
    let overrides = manifest_overrides(manifest);

    register_platform_packages(&mut provider, manifest);
    register_lockfile(&mut provider, lockfile, loader, &exclusions, &overrides);
    register_candidate_map(&mut provider, candidates, loader, &exclusions, &overrides);

    let root_package = ROOT_PACKAGE.to_string();
    let root_version = Version::zero();
    let root_dependencies = root_dependencies(manifest, loader, &overrides);
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
        exclusions,
        overrides,
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
        "quilt" => "quilt_loader",
        other => other,
    };
    let loader_version = Version::parse(&manifest.project.modloader_version, loader);
    provider.add_package_versions(loader_package.to_string(), vec![loader_version.clone()]);
    provider.add_package_deps(loader_package.to_string(), loader_version.clone(), vec![]);
    if loader == "fabric" {
        provider.add_package_versions("fabric".to_string(), vec![loader_version.clone()]);
        provider.add_package_deps("fabric".to_string(), loader_version, vec![]);
    }
}

pub(crate) fn register_candidate_versions(
    provider: &mut OrbitDependencyProvider,
    package: &str,
    candidates: &[CandidateVersion],
    loader: &str,
    exclusions: &ExclusionMap,
    overrides: &OverrideMap,
) {
    let existing = provider.versions.get(package).cloned().unwrap_or_default();
    let mut versions = Vec::new();

    for candidate in candidates {
        let version = Version::parse(&candidate.jar_version, loader);
        push_unique(&mut versions, version.clone());
        provider.add_package_deps(
            package.to_string(),
            version,
            parse_required_dependencies(&candidate.deps, package, loader, exclusions, overrides),
        );

        for implanted in &candidate.implanted {
            register_implanted_candidate(provider, implanted, loader, exclusions, overrides);
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
    exclusions: &ExclusionMap,
) -> Vec<String> {
    let mut required = HashSet::new();
    for (package, versions) in candidates {
        for candidate in versions {
            collect_required_names(&candidate.deps, exclusions.get(package), &mut required);
            for implanted in &candidate.implanted {
                collect_required_names(
                    &implanted.deps,
                    exclusions.get(&implanted.mod_id),
                    &mut required,
                );
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
    exclusions: &ExclusionMap,
    overrides: &OverrideMap,
) {
    for entry in &lockfile.packages {
        let version = Version::parse(&entry.version, loader);
        let dependencies = entry
            .dependencies
            .iter()
            .filter(|dependency| {
                !is_ignored_runtime_dependency(&dependency.name)
                    && !is_excluded(exclusions, &entry.mod_id, &dependency.name)
            })
            .map(|dependency| {
                (
                    dependency.name.clone(),
                    dependency_constraint(&dependency.name, &dependency.version, loader, overrides),
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
                .filter(|dependency| {
                    !is_ignored_runtime_dependency(&dependency.name)
                        && !is_excluded(exclusions, &implanted.name, &dependency.name)
                })
                .map(|dependency| {
                    (
                        dependency.name.clone(),
                        dependency_constraint(
                            &dependency.name,
                            &dependency.version,
                            loader,
                            overrides,
                        ),
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
    exclusions: &ExclusionMap,
    overrides: &OverrideMap,
) {
    for (package, versions) in candidates {
        register_candidate_versions(provider, package, versions, loader, exclusions, overrides);
    }
}

fn register_implanted_candidate(
    provider: &mut OrbitDependencyProvider,
    implanted: &ImplantedCandidate,
    loader: &str,
    exclusions: &ExclusionMap,
    overrides: &OverrideMap,
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
        parse_required_dependencies(
            &implanted.deps,
            &implanted.mod_id,
            loader,
            exclusions,
            overrides,
        ),
    );
}

pub(crate) fn parse_required_dependencies(
    dependencies: &[(String, String, bool)],
    package: &str,
    loader: &str,
    exclusions: &ExclusionMap,
    overrides: &OverrideMap,
) -> Vec<(String, Ranges<Version>)> {
    dependencies
        .iter()
        .filter(|(name, _, required)| {
            *required
                && !is_ignored_runtime_dependency(name)
                && !is_excluded(exclusions, package, name)
        })
        .map(|(name, constraint, _)| {
            (
                name.clone(),
                dependency_constraint(name, constraint, loader, overrides),
            )
        })
        .collect()
}

fn collect_required_names(
    dependencies: &[(String, String, bool)],
    excluded: Option<&HashSet<String>>,
    required: &mut HashSet<String>,
) {
    for (name, _, is_required) in dependencies {
        if *is_required
            && !is_builtin_package(name)
            && !excluded.is_some_and(|names| names.contains(name))
        {
            required.insert(name.clone());
        }
    }
}

fn root_dependencies(
    manifest: &OrbitManifest,
    loader: &str,
    overrides: &OverrideMap,
) -> Vec<(String, Ranges<Version>)> {
    let mut dependencies = Vec::new();
    for (name, spec) in &manifest.dependencies {
        let constraint = dependency_constraint(
            name,
            spec.version_constraint().unwrap_or("*"),
            loader,
            overrides,
        );
        dependencies.push((name.clone(), constraint));
    }
    dependencies
}

pub(crate) fn manifest_exclusions(manifest: &OrbitManifest) -> ExclusionMap {
    let mut exclusions = ExclusionMap::new();
    for (package, spec) in manifest
        .dependencies
        .iter()
        .chain(manifest.overrides.iter())
    {
        exclusions
            .entry(package.clone())
            .or_default()
            .extend(spec.exclusions().iter().cloned());
    }
    exclusions.retain(|_, names| !names.is_empty());
    exclusions
}

pub(crate) fn manifest_overrides(manifest: &OrbitManifest) -> OverrideMap {
    manifest
        .overrides
        .iter()
        .map(|(package, spec)| {
            (
                package.clone(),
                spec.version_constraint().unwrap_or("*").to_string(),
            )
        })
        .collect()
}

pub(crate) fn dependency_constraint(
    package: &str,
    constraint: &str,
    loader: &str,
    overrides: &OverrideMap,
) -> Ranges<Version> {
    Version::parse_constraint(
        overrides
            .get(package)
            .map(String::as_str)
            .unwrap_or(constraint),
        loader,
    )
}

pub(crate) fn is_ignored_runtime_dependency(package: &str) -> bool {
    matches!(package, "java" | "mixinextras")
}

fn is_excluded(exclusions: &ExclusionMap, package: &str, dependency: &str) -> bool {
    exclusions
        .get(package)
        .is_some_and(|names| names.contains(dependency))
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

pub(crate) fn is_builtin_package(package: &str) -> bool {
    matches!(
        package,
        "java"
            | "mixinextras"
            | "minecraft"
            | "fabric"
            | "fabricloader"
            | "quilt_loader"
            | "quiltloader"
            | "forge"
            | "neoforge"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockfile::{LockDependency, LockMeta, PackageEntry};

    fn candidate(version: &str, dependencies: Vec<(String, String, bool)>) -> CandidateVersion {
        CandidateVersion {
            jar_version: version.to_string(),
            deps: dependencies,
            implanted: Vec::new(),
        }
    }

    fn empty_lockfile(mc_version: &str, loader: &str, loader_version: &str) -> OrbitLockfile {
        OrbitLockfile {
            meta: LockMeta {
                mc_version: mc_version.to_string(),
                modloader: loader.to_string(),
                modloader_version: loader_version.to_string(),
            },
            packages: Vec::new(),
        }
    }

    #[test]
    fn candidate_versions_precede_and_deduplicate_existing_versions() {
        let mut provider = OrbitDependencyProvider::new();
        provider.add_package_versions("example".to_string(), vec![Version::parse("1", "forge")]);
        let candidates = vec![candidate("2", Vec::new()), candidate("1", Vec::new())];

        register_candidate_versions(
            &mut provider,
            "example",
            &candidates,
            "forge",
            &ExclusionMap::new(),
            &OverrideMap::new(),
        );

        assert_eq!(
            provider.versions["example"],
            vec![Version::parse("2", "forge"), Version::parse("1", "forge")]
        );
    }

    #[test]
    fn registering_new_candidates_prepares_their_unknown_dependencies() {
        let mut provider = OrbitDependencyProvider::new();
        let candidates = vec![candidate(
            "2",
            vec![("transitive".to_string(), "*".to_string(), true)],
        )];

        register_candidate_versions(
            &mut provider,
            "example",
            &candidates,
            "forge",
            &ExclusionMap::new(),
            &OverrideMap::new(),
        );

        assert_eq!(provider.versions["transitive"], Vec::<Version>::new());
    }

    #[test]
    fn manifest_constraint_still_applies_when_candidates_exist() {
        let manifest: OrbitManifest = toml::from_str(
            r#"
[project]
name = "test"
mc_version = "1"
modloader = "forge"
modloader_version = "1"

[dependencies]
example = "1"
"#,
        )
        .unwrap();
        let lockfile: OrbitLockfile = toml::from_str(
            r#"
[meta]
mc_version = "1"
modloader = "forge"
modloader_version = "1"

[[package]]
mod_id = "example"
version = "1"
sha256 = "unused"
provider = "file"
"#,
        )
        .unwrap();
        let candidates = HashMap::from([("example".to_string(), vec![candidate("2", Vec::new())])]);

        let graph = build_solver_graph(&manifest, &lockfile, &candidates);
        let solution =
            pubgrub::resolve(&graph.provider, graph.root_package, graph.root_version).unwrap();

        assert_eq!(
            solution.get(&"example".to_string()),
            Some(&Version::parse("1", "forge"))
        );
    }

    #[test]
    fn override_replaces_a_transitive_constraint_without_adding_a_package() {
        let manifest: OrbitManifest = toml::from_str(
            r#"
[project]
name = "test"
mc_version = "1"
modloader = "forge"
modloader_version = "1"

[dependencies]
a = "*"

[overrides]
b = "1"
unused = "1"
"#,
        )
        .unwrap();
        let lockfile = empty_lockfile("1", "forge", "1");
        let candidates = HashMap::from([
            (
                "a".to_string(),
                vec![candidate(
                    "1",
                    vec![("b".to_string(), "2".to_string(), true)],
                )],
            ),
            (
                "b".to_string(),
                vec![candidate("2", Vec::new()), candidate("1", Vec::new())],
            ),
        ]);

        let graph = build_solver_graph(&manifest, &lockfile, &candidates);
        let solution =
            pubgrub::resolve(&graph.provider, graph.root_package, graph.root_version).unwrap();

        assert_eq!(
            solution.get(&"b".to_string()),
            Some(&Version::parse("1", "forge"))
        );
        assert!(solution.get(&"unused".to_string()).is_none());
    }

    #[test]
    fn exclude_removes_only_the_declaring_packages_dependency_edge() {
        let manifest: OrbitManifest = toml::from_str(
            r#"
[project]
name = "test"
mc_version = "1"
modloader = "forge"
modloader_version = "1"

[dependencies]
a = { version = "*", exclude = ["b"] }
"#,
        )
        .unwrap();
        let lockfile = empty_lockfile("1", "forge", "1");
        let candidates = HashMap::from([
            (
                "a".to_string(),
                vec![candidate(
                    "1",
                    vec![("b".to_string(), "*".to_string(), true)],
                )],
            ),
            ("b".to_string(), vec![candidate("1", Vec::new())]),
        ]);

        let graph = build_solver_graph(&manifest, &lockfile, &candidates);
        let solution =
            pubgrub::resolve(&graph.provider, graph.root_package, graph.root_version).unwrap();

        assert_eq!(
            solution.get(&"a".to_string()),
            Some(&Version::parse("1", "forge"))
        );
        assert!(solution.get(&"b".to_string()).is_none());
    }

    #[test]
    fn runtime_dependencies_are_consistently_ignored() {
        let manifest: OrbitManifest = toml::from_str(
            r#"
[project]
name = "test"
mc_version = "1.21.1"
modloader = "fabric"
modloader_version = "0.16.0"

[dependencies]
a = "*"
"#,
        )
        .unwrap();
        let lockfile = empty_lockfile("1.21.1", "fabric", "0.16.0");
        let candidates = HashMap::from([(
            "a".to_string(),
            vec![candidate(
                "1",
                vec![
                    ("java".to_string(), ">=21".to_string(), true),
                    ("mixinextras".to_string(), ">=0.4".to_string(), true),
                ],
            )],
        )]);

        let graph = build_solver_graph(&manifest, &lockfile, &candidates);
        let solution =
            pubgrub::resolve(&graph.provider, graph.root_package, graph.root_version).unwrap();

        assert_eq!(
            solution.get(&"a".to_string()),
            Some(&Version::Fabric(
                crate::versions::fabric::SemanticVersion::parse("1", true).unwrap()
            ))
        );
        assert!(!graph.provider.versions.contains_key("java"));
        assert!(!graph.provider.versions.contains_key("mixinextras"));
    }

    #[test]
    fn forge_loader_satisfies_maven_version_ranges() {
        let manifest: OrbitManifest = toml::from_str(
            r#"
[project]
name = "test"
mc_version = "1.20.1"
modloader = "forge"
modloader_version = "47.2.0"
[dependencies]
example = "*"
"#,
        )
        .unwrap();
        let mut lockfile = empty_lockfile("1.20.1", "forge", "47.2.0");
        lockfile.packages.push(PackageEntry {
            mod_id: "example".to_string(),
            version: "1".to_string(),
            sha1: String::new(),
            sha256: String::new(),
            sha512: String::new(),
            filename: "example.jar".to_string(),
            provider: "file".to_string(),
            modrinth: None,
            file: None,
            dependencies: vec![LockDependency {
                name: "forge".to_string(),
                version: "[47,48)".to_string(),
            }],
            implanted: Vec::new(),
        });

        assert!(crate::resolver::check_lockfile_graph(&manifest, &lockfile).is_ok());

        lockfile.meta.modloader_version = "46.0.0".to_string();
        let mut incompatible = manifest;
        incompatible.project.modloader_version = "46.0.0".to_string();
        assert!(crate::resolver::check_lockfile_graph(&incompatible, &lockfile).is_err());
    }
}
