//! Builds the loader-independent constraint graph consumed by PubGrub.

use std::collections::{HashMap, HashSet};

use pubgrub::{IncompatibilityConstraint, IncompatibilityConstraintTerm, Ranges};

use crate::lockfile::{BundledMod, OrbitLockfile};
use crate::manifest::OrbitManifest;
use crate::metadata::{DependencyExpression, EmbeddedArtifact, Environment, ProvidedMod};
use crate::resolver::provider::OrbitDependencyProvider;
use crate::resolver::types::{BundledCandidate, CandidateVersion};
use crate::versions::Version;

use super::constraints::compile_dependency_constraints;
use super::ordering::register_ordering_cycles;

pub(crate) const ROOT_PACKAGE: &str = "___orbit_root___";
const CAPABILITY_PREFIX: &str = "___orbit_capability___";
pub(crate) type ExclusionMap = HashMap<String, HashSet<String>>;
pub(crate) type OverrideMap = HashMap<String, String>;

pub(crate) struct SolverGraph {
    pub(crate) provider: OrbitDependencyProvider,
    pub(crate) root_package: String,
    pub(crate) root_version: Version,
    pub(crate) exclusions: ExclusionMap,
    pub(crate) overrides: OverrideMap,
    pub(crate) target: Environment,
}

pub(crate) fn build_solver_graph(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    candidates: &HashMap<String, Vec<CandidateVersion>>,
) -> SolverGraph {
    build_solver_graph_for_target(manifest, lockfile, candidates, Environment::Both)
}

pub(crate) fn build_solver_graph_for_target(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    candidates: &HashMap<String, Vec<CandidateVersion>>,
    target: Environment,
) -> SolverGraph {
    let loader = &manifest.project.modloader;
    let exclusions = manifest_exclusions(manifest);
    let overrides = manifest_overrides(manifest);
    let mut provider = OrbitDependencyProvider::new();

    register_platform_packages(&mut provider, manifest);
    register_lockfile(
        &mut provider,
        lockfile,
        loader,
        &exclusions,
        &overrides,
        target,
    );
    register_candidate_map(
        &mut provider,
        candidates,
        loader,
        &exclusions,
        &overrides,
        target,
    );
    register_ordering_cycles(
        &mut provider,
        lockfile,
        candidates,
        loader,
        &exclusions,
        &overrides,
        target,
    );

    let root_package = ROOT_PACKAGE.to_string();
    let root_version = Version::zero();
    provider.add_package_versions(root_package.clone(), vec![root_version.clone()]);
    provider.add_package_deps(
        root_package.clone(),
        root_version.clone(),
        root_dependencies(manifest, loader, &overrides, target),
    );
    provider.add_package_incompatibilities(root_package.clone(), root_version.clone(), Vec::new());
    ensure_referenced_packages(&mut provider);

    SolverGraph {
        provider,
        root_package,
        root_version,
        exclusions,
        overrides,
        target,
    }
}

pub(crate) fn register_platform_packages(
    provider: &mut OrbitDependencyProvider,
    manifest: &OrbitManifest,
) {
    let loader = &manifest.project.modloader;
    register_leaf(
        provider,
        "minecraft",
        Version::parse(&manifest.project.mc_version, loader),
    );

    let loader_package = match loader.as_str() {
        "fabric" => "fabricloader",
        "quilt" => "quilt_loader",
        other => other,
    };
    let loader_version = Version::parse(&manifest.project.modloader_version, loader);
    register_leaf(provider, loader_package, loader_version.clone());

    match loader.as_str() {
        "fabric" => register_leaf(provider, "fabric", loader_version),
        "quilt" => register_leaf(
            provider,
            "quiltloader",
            Version::parse(&manifest.project.modloader_version, loader),
        ),
        "forge" => {
            let major = manifest
                .project
                .modloader_version
                .split('.')
                .next()
                .unwrap_or(&manifest.project.modloader_version);
            register_leaf(provider, "javafml", Version::parse(major, loader));
            register_leaf(provider, "lowcodefml", Version::parse(major, loader));
        }
        "neoforge" => {
            register_leaf(provider, "javafml", Version::parse("1", loader));
            register_leaf(provider, "lowcodefml", Version::parse("1", loader));
        }
        _ => {}
    }

    register_leaf(
        provider,
        "java",
        Version::parse(
            &minecraft_java_version(&manifest.project.mc_version),
            loader,
        ),
    );
}

fn register_leaf(provider: &mut OrbitDependencyProvider, package: &str, version: Version) {
    add_version(provider, package, version.clone());
    provider.add_package_deps(package.to_string(), version.clone(), Vec::new());
    provider.add_package_incompatibilities(package.to_string(), version, Vec::new());
}

pub(crate) fn register_candidate_versions(
    provider: &mut OrbitDependencyProvider,
    package: &str,
    candidates: &[CandidateVersion],
    loader: &str,
    exclusions: &ExclusionMap,
    overrides: &OverrideMap,
    target: Environment,
) {
    for candidate in candidates {
        let version = Version::parse(&candidate.jar_version, loader);
        register_module(
            provider,
            package,
            version.clone(),
            &candidate.dependencies,
            candidate.environment,
            &candidate.provides,
            candidate.language_loader.as_ref(),
            &candidate.embedded_artifacts,
            loader,
            exclusions,
            overrides,
            target,
            None,
            candidate_bundled_links(&candidate.bundled, loader),
        );
        for bundled in &candidate.bundled {
            register_bundled_candidate(
                provider, bundled, package, &version, loader, exclusions, overrides, target,
            );
        }
    }
    ensure_referenced_packages(provider);
}

pub(crate) fn required_candidate_packages(
    candidates: &HashMap<String, Vec<CandidateVersion>>,
    exclusions: &ExclusionMap,
    target: Environment,
) -> Vec<String> {
    let mut required = HashSet::new();
    for (package, versions) in candidates {
        for candidate in versions {
            collect_required_names(
                &candidate.dependencies,
                exclusions.get(package),
                target,
                &mut required,
            );
            collect_bundled_required_names(&candidate.bundled, exclusions, target, &mut required);
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
    target: Environment,
) {
    for entry in &lockfile.packages {
        let version = Version::parse(&entry.version, loader);
        let bundled_links = bundled_links(&entry.bundled, loader);
        register_module(
            provider,
            &entry.mod_id,
            version.clone(),
            &entry.dependencies,
            entry.environment,
            &entry.provides,
            entry.language_loader.as_ref(),
            &entry.embedded_artifacts,
            loader,
            exclusions,
            overrides,
            target,
            None,
            bundled_links,
        );
        for bundled in &entry.bundled {
            register_bundled_lock(
                provider,
                bundled,
                &entry.mod_id,
                &version,
                loader,
                exclusions,
                overrides,
                target,
            );
        }
    }
}

fn register_candidate_map(
    provider: &mut OrbitDependencyProvider,
    candidates: &HashMap<String, Vec<CandidateVersion>>,
    loader: &str,
    exclusions: &ExclusionMap,
    overrides: &OverrideMap,
    target: Environment,
) {
    for (package, versions) in candidates {
        register_candidate_versions(
            provider, package, versions, loader, exclusions, overrides, target,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn register_bundled_candidate(
    provider: &mut OrbitDependencyProvider,
    bundled: &BundledCandidate,
    owner: &str,
    owner_version: &Version,
    loader: &str,
    exclusions: &ExclusionMap,
    overrides: &OverrideMap,
    target: Environment,
) {
    register_module(
        provider,
        &bundled.mod_id,
        Version::parse(&bundled.version, loader),
        &bundled.dependencies,
        bundled.environment,
        &bundled.provides,
        bundled.language_loader.as_ref(),
        &bundled.embedded_artifacts,
        loader,
        exclusions,
        overrides,
        target,
        Some((owner, owner_version.clone())),
        candidate_bundled_links(&bundled.bundled, loader),
    );
    for child in &bundled.bundled {
        register_bundled_candidate(
            provider,
            child,
            owner,
            owner_version,
            loader,
            exclusions,
            overrides,
            target,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn register_bundled_lock(
    provider: &mut OrbitDependencyProvider,
    bundled: &BundledMod,
    owner: &str,
    owner_version: &Version,
    loader: &str,
    exclusions: &ExclusionMap,
    overrides: &OverrideMap,
    target: Environment,
) {
    let version = Version::parse(&bundled.version, loader);
    let child_links = bundled_links(&bundled.bundled, loader);
    register_module(
        provider,
        &bundled.mod_id,
        version,
        &bundled.dependencies,
        bundled.environment,
        &bundled.provides,
        bundled.language_loader.as_ref(),
        &bundled.embedded_artifacts,
        loader,
        exclusions,
        overrides,
        target,
        Some((owner, owner_version.clone())),
        child_links,
    );
    for child in &bundled.bundled {
        register_bundled_lock(
            provider,
            child,
            owner,
            owner_version,
            loader,
            exclusions,
            overrides,
            target,
        );
    }
}

fn bundled_links(mods: &[BundledMod], loader: &str) -> Vec<(String, Version)> {
    fn collect(mods: &[BundledMod], loader: &str, output: &mut Vec<(String, Version)>) {
        for metadata in mods {
            output.push((
                metadata.mod_id.clone(),
                Version::parse(&metadata.version, loader),
            ));
            collect(&metadata.bundled, loader, output);
        }
    }

    let mut output = Vec::new();
    collect(mods, loader, &mut output);
    output
}

fn candidate_bundled_links(mods: &[BundledCandidate], loader: &str) -> Vec<(String, Version)> {
    fn collect(mods: &[BundledCandidate], loader: &str, output: &mut Vec<(String, Version)>) {
        for metadata in mods {
            output.push((
                metadata.mod_id.clone(),
                Version::parse(&metadata.version, loader),
            ));
            collect(&metadata.bundled, loader, output);
        }
    }

    let mut output = Vec::new();
    collect(mods, loader, &mut output);
    output
}

#[allow(clippy::too_many_arguments)]
fn register_module(
    provider: &mut OrbitDependencyProvider,
    package: &str,
    version: Version,
    expressions: &[DependencyExpression],
    environment: Environment,
    provides: &[ProvidedMod],
    language_loader: Option<&crate::metadata::LanguageLoaderRequirement>,
    embedded_artifacts: &[EmbeddedArtifact],
    loader: &str,
    exclusions: &ExclusionMap,
    overrides: &OverrideMap,
    target: Environment,
    owner: Option<(&str, Version)>,
    bundled: Vec<(String, Version)>,
) {
    add_version(provider, package, version.clone());
    let mut dependencies = Vec::new();
    let mut incompatibilities =
        compile_dependency_constraints(expressions, package, loader, exclusions, overrides, target);
    if target != Environment::Both && !environment.applies_to(target) {
        incompatibilities.push(IncompatibilityConstraint {
            terms: Vec::new(),
            reason: format!(
                "{package} is declared for {} but the selected target is {}",
                environment.as_str(),
                target.as_str()
            ),
        });
    }

    if let Some(language_loader) = language_loader {
        dependencies.push((
            language_loader.id.clone(),
            dependency_constraint(
                &language_loader.id,
                &language_loader.requirement,
                loader,
                overrides,
            ),
        ));
    }
    for artifact in embedded_artifacts {
        let artifact_package = embedded_artifact_package(&artifact.id);
        let artifact_version = Version::parse(&artifact.version, "forge");
        add_version(provider, &artifact_package, artifact_version.clone());
        provider.add_package_deps(
            artifact_package.clone(),
            artifact_version.clone(),
            Vec::new(),
        );
        provider.add_package_incompatibilities(
            artifact_package.clone(),
            artifact_version,
            Vec::new(),
        );
        dependencies.push((
            artifact_package,
            Version::parse_constraint(&artifact.requirement, "forge"),
        ));
    }
    if let Some((owner, owner_version)) = owner {
        dependencies.push((owner.to_string(), Ranges::singleton(owner_version)));
    }
    for (bundled_id, bundled_version) in bundled {
        dependencies.push((bundled_id, Ranges::singleton(bundled_version)));
    }
    dependencies.extend(register_capability(
        provider,
        package,
        version.clone(),
        package,
        version.clone(),
    ));
    for provided in provides {
        let provided_version = Version::parse(
            provided.version.as_deref().unwrap_or(&version.to_string()),
            loader,
        );
        dependencies.extend(register_capability(
            provider,
            &provided.id,
            provided_version,
            package,
            version.clone(),
        ));
    }

    provider.add_package_deps(package.to_string(), version.clone(), dependencies);
    provider.add_package_incompatibilities(
        package.to_string(),
        version,
        std::mem::take(&mut incompatibilities),
    );
}

fn collect_required_names(
    dependencies: &[DependencyExpression],
    excluded: Option<&HashSet<String>>,
    target: Environment,
    required: &mut HashSet<String>,
) {
    for dependency in dependencies {
        for relation in dependency.relations() {
            if relation.kind.installs_target()
                && relation.environment.applies_to(target)
                && !is_builtin_package(&relation.id)
                && !excluded.is_some_and(|names| names.contains(&relation.id))
            {
                required.insert(relation.id.clone());
            }
        }
    }
}

fn collect_bundled_required_names(
    bundled: &[BundledCandidate],
    exclusions: &ExclusionMap,
    target: Environment,
    required: &mut HashSet<String>,
) {
    for metadata in bundled {
        collect_required_names(
            &metadata.dependencies,
            exclusions.get(&metadata.mod_id),
            target,
            required,
        );
        collect_bundled_required_names(&metadata.bundled, exclusions, target, required);
    }
}

fn root_dependencies(
    manifest: &OrbitManifest,
    loader: &str,
    overrides: &OverrideMap,
    target: Environment,
) -> Vec<(String, Ranges<Version>)> {
    manifest
        .dependencies
        .iter()
        .filter(|(_, spec)| manifest_environment(spec.env()).applies_to(target))
        .map(|(name, spec)| {
            (
                constraint_package(name),
                dependency_constraint(
                    name,
                    spec.version_constraint().unwrap_or("*"),
                    loader,
                    overrides,
                ),
            )
        })
        .collect()
}

fn manifest_environment(environment: Option<&str>) -> Environment {
    match environment {
        Some("client") => Environment::Client,
        Some("server") => Environment::Server,
        _ => Environment::Both,
    }
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

pub(super) fn is_excluded(exclusions: &ExclusionMap, package: &str, dependency: &str) -> bool {
    exclusions
        .get(package)
        .is_some_and(|names| names.contains(dependency))
}

fn ensure_referenced_packages(provider: &mut OrbitDependencyProvider) {
    let dependency_packages = provider
        .dependencies
        .values()
        .flat_map(|dependencies| dependencies.iter().map(|(package, _)| package.clone()));
    let incompatibility_packages = provider
        .incompatibilities
        .values()
        .flat_map(|clauses| clauses.iter())
        .flat_map(|clause| clause.terms.iter())
        .map(|term| match term {
            IncompatibilityConstraintTerm::Positive(package, _)
            | IncompatibilityConstraintTerm::Negative(package, _) => package.clone(),
        });
    let referenced: HashSet<_> = dependency_packages
        .chain(incompatibility_packages)
        .collect();
    for package in referenced {
        provider.versions.entry(package).or_default();
    }
}

fn add_version(provider: &mut OrbitDependencyProvider, package: &str, version: Version) {
    let versions = provider.versions.entry(package.to_string()).or_default();
    if !versions.contains(&version) {
        versions.push(version);
    }
}

fn register_capability(
    provider: &mut OrbitDependencyProvider,
    id: &str,
    provided_version: Version,
    owner: &str,
    owner_version: Version,
) -> [(String, Ranges<Version>); 2] {
    let capability = constraint_package(id);
    let choice_package = format!("___orbit_provider_choice___{id}___{provided_version}");
    let choice_version = Version::Generic(format!("{owner}@{owner_version}"));

    add_version(provider, &capability, provided_version.clone());
    provider.add_package_deps(
        capability.clone(),
        provided_version.clone(),
        vec![(choice_package.clone(), Ranges::full())],
    );
    provider.add_package_incompatibilities(
        capability.clone(),
        provided_version.clone(),
        Vec::new(),
    );

    add_version(provider, &choice_package, choice_version.clone());
    provider.add_package_deps(
        choice_package.clone(),
        choice_version.clone(),
        vec![(owner.to_string(), Ranges::singleton(owner_version))],
    );
    provider.add_package_incompatibilities(
        choice_package.clone(),
        choice_version.clone(),
        Vec::new(),
    );

    [
        (capability, Ranges::singleton(provided_version)),
        (choice_package, Ranges::singleton(choice_version)),
    ]
}

pub(super) fn constraint_package(package: &str) -> String {
    if is_builtin_package(package) {
        package.to_string()
    } else {
        format!("{CAPABILITY_PREFIX}{package}")
    }
}

pub(crate) fn is_builtin_package(package: &str) -> bool {
    package.starts_with("___orbit_jarjar___")
        || package.starts_with(CAPABILITY_PREFIX)
        || package.starts_with("___orbit_provider_choice___")
        || matches!(
            package,
            "java"
                | "minecraft"
                | "fabric"
                | "fabricloader"
                | "quilt_loader"
                | "quiltloader"
                | "forge"
                | "neoforge"
                | "javafml"
                | "lowcodefml"
        )
}

fn embedded_artifact_package(id: &str) -> String {
    format!("___orbit_jarjar___{id}")
}

fn minecraft_java_version(minecraft: &str) -> String {
    let numeric = minecraft
        .split(['-', '+'])
        .next()
        .unwrap_or(minecraft)
        .split('.')
        .filter_map(|part| part.parse::<u32>().ok())
        .collect::<Vec<_>>();
    match numeric.as_slice() {
        [major, ..] if *major >= 26 => "25",
        [major, minor, patch, ..] if (*major, *minor, *patch) >= (1, 20, 5) => "21",
        [major, minor, ..] if (*major, *minor) >= (1, 18) => "17",
        [1, 17, ..] => "16",
        _ => "8",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockfile::{LockMeta, PackageEntry};
    use crate::metadata::{DependencyKind, DependencyOrdering, ModDependency};
    use crate::resolver::ordering::resolution_warnings;

    fn dependency(id: &str, requirement: &str, kind: DependencyKind) -> DependencyExpression {
        ModDependency {
            id: id.to_string(),
            requirement: requirement.to_string(),
            kind,
            environment: Environment::Both,
            ordering: Default::default(),
            reason: None,
            unless: None,
        }
        .into()
    }

    fn manifest() -> OrbitManifest {
        toml::from_str(
            r#"
[project]
name = "test"
mc_version = "1.20.1"
modloader = "forge"
modloader_version = "47.2.0"
[dependencies]
a = "*"
b = "*"
"#,
        )
        .unwrap()
    }

    fn package(id: &str, version: &str, dependencies: Vec<DependencyExpression>) -> PackageEntry {
        PackageEntry {
            mod_id: id.to_string(),
            version: version.to_string(),
            sha1: String::new(),
            sha256: String::new(),
            sha512: String::new(),
            filename: format!("{id}.jar"),
            provider: "file".to_string(),
            modrinth: None,
            curseforge: None,
            file: None,
            dependencies,
            environment: Environment::Both,
            provides: Vec::new(),
            language_loader: None,
            embedded_artifacts: Vec::new(),
            bundled: Vec::new(),
        }
    }

    #[test]
    fn provided_capability_selects_one_of_multiple_providers() {
        let mut manifest = manifest();
        manifest.dependencies.clear();
        manifest.dependencies.insert(
            "virtual_api".to_string(),
            crate::manifest::DependencySpec::Short("*".to_string()),
        );
        let mut first = package("provider_one", "1", Vec::new());
        first.provides.push(ProvidedMod {
            id: "virtual_api".to_string(),
            version: Some("1".to_string()),
        });
        let mut second = package("provider_two", "1", Vec::new());
        second.provides.push(ProvidedMod {
            id: "virtual_api".to_string(),
            version: Some("1".to_string()),
        });
        let lockfile = OrbitLockfile {
            meta: LockMeta {
                mc_version: "1.20.1".to_string(),
                modloader: "forge".to_string(),
                modloader_version: "47.2.0".to_string(),
            },
            packages: vec![first, second],
        };

        let graph = build_solver_graph(&manifest, &lockfile, &HashMap::new());
        let solution =
            pubgrub::resolve(&graph.provider, graph.root_package, graph.root_version).unwrap();
        let selected_providers = ["provider_one", "provider_two"]
            .into_iter()
            .filter(|provider| solution.get(&provider.to_string()).is_some())
            .count();

        assert_eq!(selected_providers, 1);
        assert!(
            solution
                .get(&"___orbit_capability___virtual_api".to_string())
                .is_some()
        );
    }

    #[test]
    fn optional_dependencies_validate_only_when_present() {
        let manifest = manifest();
        let lockfile = OrbitLockfile {
            meta: LockMeta {
                mc_version: "1.20.1".to_string(),
                modloader: "forge".to_string(),
                modloader_version: "47.2.0".to_string(),
            },
            packages: vec![
                package(
                    "a",
                    "1",
                    vec![dependency("b", "[2,)", DependencyKind::Optional)],
                ),
                package("b", "1", Vec::new()),
            ],
        };
        let graph = build_solver_graph(&manifest, &lockfile, &HashMap::new());
        assert!(pubgrub::resolve(&graph.provider, graph.root_package, graph.root_version).is_err());
    }

    #[test]
    fn incompatible_dependencies_are_solver_constraints() {
        let manifest = manifest();
        let lockfile = OrbitLockfile {
            meta: LockMeta {
                mc_version: "1.20.1".to_string(),
                modloader: "forge".to_string(),
                modloader_version: "47.2.0".to_string(),
            },
            packages: vec![
                package(
                    "a",
                    "1",
                    vec![dependency("b", "[1,2)", DependencyKind::Incompatible)],
                ),
                package("b", "1", Vec::new()),
            ],
        };
        let graph = build_solver_graph(&manifest, &lockfile, &HashMap::new());
        assert!(pubgrub::resolve(&graph.provider, graph.root_package, graph.root_version).is_err());
    }

    #[test]
    fn any_group_accepts_one_compatible_dependency() {
        let mut manifest = manifest();
        manifest.dependencies.shift_remove("b");
        let lockfile = OrbitLockfile {
            meta: LockMeta {
                mc_version: "1.20.1".to_string(),
                modloader: "forge".to_string(),
                modloader_version: "47.2.0".to_string(),
            },
            packages: vec![package(
                "a",
                "1",
                vec![DependencyExpression::Any(vec![
                    dependency("missing", "*", DependencyKind::Required),
                    dependency("forge", "[47,)", DependencyKind::Required),
                ])],
            )],
        };
        let graph = build_solver_graph(&manifest, &lockfile, &HashMap::new());
        assert!(pubgrub::resolve(&graph.provider, graph.root_package, graph.root_version).is_ok());
    }

    #[test]
    fn all_group_conflict_requires_every_member() {
        let mut manifest = manifest();
        manifest.dependencies.insert(
            "c".to_string(),
            crate::manifest::DependencySpec::Short("*".to_string()),
        );
        let conflict = DependencyExpression::All(vec![
            dependency("b", "*", DependencyKind::Incompatible),
            dependency("c", "*", DependencyKind::Incompatible),
        ]);
        let lockfile = OrbitLockfile {
            meta: LockMeta {
                mc_version: "1.20.1".to_string(),
                modloader: "forge".to_string(),
                modloader_version: "47.2.0".to_string(),
            },
            packages: vec![
                package("a", "1", vec![conflict]),
                package("b", "1", Vec::new()),
                package("c", "1", Vec::new()),
            ],
        };
        let graph = build_solver_graph(&manifest, &lockfile, &HashMap::new());
        assert!(pubgrub::resolve(&graph.provider, graph.root_package, graph.root_version).is_err());

        manifest.dependencies.shift_remove("c");
        let graph = build_solver_graph(&manifest, &lockfile, &HashMap::new());
        assert!(pubgrub::resolve(&graph.provider, graph.root_package, graph.root_version).is_ok());
    }

    #[test]
    fn dependency_sides_are_evaluated_for_the_selected_target() {
        let mut manifest = manifest();
        manifest.dependencies.shift_remove("b");
        let client_dependency = ModDependency {
            environment: Environment::Client,
            ..ModDependency::required("b", "*")
        };
        let lockfile = OrbitLockfile {
            meta: LockMeta {
                mc_version: "1.20.1".to_string(),
                modloader: "forge".to_string(),
                modloader_version: "47.2.0".to_string(),
            },
            packages: vec![package("a", "1", vec![client_dependency.into()])],
        };

        let server = build_solver_graph_for_target(
            &manifest,
            &lockfile,
            &HashMap::new(),
            Environment::Server,
        );
        assert!(
            pubgrub::resolve(&server.provider, server.root_package, server.root_version).is_ok()
        );
        let client = build_solver_graph_for_target(
            &manifest,
            &lockfile,
            &HashMap::new(),
            Environment::Client,
        );
        assert!(
            pubgrub::resolve(&client.provider, client.root_package, client.root_version).is_err()
        );
    }

    #[test]
    fn ordering_cycles_are_reported_by_pubgrub() {
        let mut a_before_b = ModDependency::required("b", "*");
        a_before_b.ordering = DependencyOrdering::Before;
        let mut b_before_a = ModDependency::required("a", "*");
        b_before_a.ordering = DependencyOrdering::Before;
        let manifest = manifest();
        let lockfile = OrbitLockfile {
            meta: LockMeta {
                mc_version: "1.20.1".to_string(),
                modloader: "forge".to_string(),
                modloader_version: "47.2.0".to_string(),
            },
            packages: vec![
                package("a", "1", vec![a_before_b.into()]),
                package("b", "1", vec![b_before_a.into()]),
            ],
        };

        let error = crate::resolver::check_lockfile_graph(&manifest, &lockfile).unwrap_err();

        assert!(error.contains("load ordering cycle: a -> b -> a"));
    }

    #[test]
    fn jarjar_artifacts_share_one_solver_selected_version() {
        let manifest = manifest();
        let artifact = |version: &str, requirement: &str| EmbeddedArtifact {
            id: "org.example:shared".to_string(),
            requirement: requirement.to_string(),
            version: version.to_string(),
            path: format!("META-INF/jarjar/shared-{version}.jar"),
            obfuscated: false,
        };
        let mut a = package("a", "1", Vec::new());
        a.embedded_artifacts = vec![artifact("1", "[1,2)")];
        let mut b = package("b", "1", Vec::new());
        b.embedded_artifacts = vec![artifact("2", "[2,3)")];
        let lockfile = OrbitLockfile {
            meta: LockMeta {
                mc_version: "1.20.1".to_string(),
                modloader: "forge".to_string(),
                modloader_version: "47.2.0".to_string(),
            },
            packages: vec![a, b],
        };

        let error = crate::resolver::check_lockfile_graph(&manifest, &lockfile).unwrap_err();

        assert!(error.contains("org.example:shared"));
    }

    #[test]
    fn soft_dependency_semantics_are_reported_without_rejecting_solution() {
        let manifest = manifest();
        let lockfile = OrbitLockfile {
            meta: LockMeta {
                mc_version: "1.20.1".to_string(),
                modloader: "forge".to_string(),
                modloader_version: "47.2.0".to_string(),
            },
            packages: vec![
                package(
                    "a",
                    "1",
                    vec![
                        dependency("missing", "*", DependencyKind::Recommended),
                        dependency("b", "*", DependencyKind::Discouraged),
                    ],
                ),
                package("b", "1", Vec::new()),
            ],
        };
        let graph = build_solver_graph(&manifest, &lockfile, &HashMap::new());
        let solution = pubgrub::resolve(
            &graph.provider,
            graph.root_package.clone(),
            graph.root_version.clone(),
        )
        .unwrap();
        let warnings = resolution_warnings(
            &lockfile,
            &HashMap::new(),
            &solution,
            &manifest.project.modloader,
            &graph.exclusions,
            &graph.overrides,
            graph.target,
        );

        assert_eq!(warnings.len(), 2);
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("recommends missing"))
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("discourages b"))
        );
    }
}
