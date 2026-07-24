//! Builds the loader-independent constraint graph consumed by PubGrub.

use std::collections::{BTreeSet, HashMap, HashSet};

use pubgrub::{IncompatibilityConstraint, IncompatibilityConstraintTerm, Ranges};

use crate::lockfile::{BundledMod, OrbitLockfile};
use crate::manifest::OrbitManifest;
use crate::metadata::{
    DependencyExpression, EmbeddedArtifact, Environment, LanguageLoaderRequirement, ProvidedMod,
};
use crate::resolver::provider::OrbitDependencyProvider;
use crate::resolver::types::{
    BundledCandidate, CandidateVersion, LogicalPackage, SolverPackage, SolverVersion, solver_range,
};
use crate::versions::Version;

use super::constraints::compile_dependency_constraints;
use super::ordering::register_ordering_cycles;

pub(crate) type ExclusionMap = HashMap<String, HashSet<String>>;
pub(crate) type OverrideMap = HashMap<String, String>;
type AvailabilityMap = HashMap<(SolverPackage, Version), BTreeSet<(SolverPackage, Version)>>;

#[derive(Clone, Copy)]
struct GraphContext<'a> {
    loader: &'a str,
    exclusions: &'a ExclusionMap,
    overrides: &'a OverrideMap,
    target: Environment,
}

struct ModuleRegistration<'a> {
    package: SolverPackage,
    mod_id: &'a str,
    version: Version,
    dependencies: &'a [DependencyExpression],
    environment: Environment,
    provides: &'a [ProvidedMod],
    language_loader: Option<&'a LanguageLoaderRequirement>,
    embedded_artifacts: &'a [EmbeddedArtifact],
    owner: Option<(SolverPackage, Version)>,
    bundled: Vec<(SolverPackage, Version)>,
}

pub(crate) struct SolverGraph {
    pub(crate) provider: OrbitDependencyProvider,
    pub(crate) root_package: SolverPackage,
    pub(crate) root_version: SolverVersion,
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
    let mut availability = AvailabilityMap::new();
    let context = GraphContext {
        loader,
        exclusions: &exclusions,
        overrides: &overrides,
        target,
    };

    register_platform_packages(&mut provider, manifest);
    register_lockfile(&mut provider, &mut availability, lockfile, &context);
    register_candidate_map(&mut provider, &mut availability, candidates, &context);
    register_availability(&mut provider, availability);
    register_ordering_cycles(
        &mut provider,
        lockfile,
        candidates,
        loader,
        &exclusions,
        &overrides,
        target,
    );

    let root_package = SolverPackage::Root;
    let root_version = SolverVersion::Domain(Version::zero());
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
    let package = SolverPackage::Platform(package.to_string());
    add_version(provider, package.clone(), version.clone());
    provider.add_package_deps(package.clone(), version.clone().into(), Vec::new());
    provider.add_package_incompatibilities(package, version.into(), Vec::new());
}

fn register_candidate_versions(
    provider: &mut OrbitDependencyProvider,
    availability: &mut AvailabilityMap,
    package: &str,
    candidates: &[CandidateVersion],
    context: &GraphContext<'_>,
) {
    for candidate in candidates {
        let version = Version::parse(&candidate.jar_version, context.loader);
        let physical_package = SolverPackage::top_level(package);
        let bundled =
            candidate_bundled_links(&candidate.bundled, package, &version, context.loader);
        register_module(
            provider,
            availability,
            ModuleRegistration {
                package: physical_package,
                mod_id: package,
                version: version.clone(),
                dependencies: &candidate.dependencies,
                environment: candidate.environment,
                provides: &candidate.provides,
                language_loader: candidate.language_loader.as_ref(),
                embedded_artifacts: &candidate.embedded_artifacts,
                owner: None,
                bundled,
            },
            context,
        );
        for (index, bundled) in candidate.bundled.iter().enumerate() {
            register_bundled_candidate(
                provider,
                availability,
                bundled,
                package,
                &version,
                vec![index],
                context,
            );
        }
    }
}

fn register_lockfile(
    provider: &mut OrbitDependencyProvider,
    availability: &mut AvailabilityMap,
    lockfile: &OrbitLockfile,
    context: &GraphContext<'_>,
) {
    for entry in &lockfile.packages {
        let version = Version::parse(&entry.version, context.loader);
        let bundled_links = bundled_links(&entry.bundled, &entry.mod_id, &version, context.loader);
        register_module(
            provider,
            availability,
            ModuleRegistration {
                package: SolverPackage::top_level(&entry.mod_id),
                mod_id: &entry.mod_id,
                version: version.clone(),
                dependencies: &entry.dependencies,
                environment: entry.environment,
                provides: &entry.provides,
                language_loader: entry.language_loader.as_ref(),
                embedded_artifacts: &entry.embedded_artifacts,
                owner: None,
                bundled: bundled_links,
            },
            context,
        );
        for (index, bundled) in entry.bundled.iter().enumerate() {
            register_bundled_lock(
                provider,
                availability,
                bundled,
                &entry.mod_id,
                &version,
                vec![index],
                context,
            );
        }
    }
}

fn register_candidate_map(
    provider: &mut OrbitDependencyProvider,
    availability: &mut AvailabilityMap,
    candidates: &HashMap<String, Vec<CandidateVersion>>,
    context: &GraphContext<'_>,
) {
    let mut packages: Vec<_> = candidates.iter().collect();
    packages.sort_by_key(|(package, _)| *package);
    for (package, versions) in packages {
        register_candidate_versions(provider, availability, package, versions, context);
    }
}

fn register_bundled_candidate(
    provider: &mut OrbitDependencyProvider,
    availability: &mut AvailabilityMap,
    bundled: &BundledCandidate,
    owner: &str,
    owner_version: &Version,
    path: Vec<usize>,
    context: &GraphContext<'_>,
) {
    let package = SolverPackage::Bundled {
        owner: owner.to_string(),
        owner_version: owner_version.clone(),
        path: path.clone(),
        mod_id: bundled.mod_id.clone(),
    };
    register_module(
        provider,
        availability,
        ModuleRegistration {
            package,
            mod_id: &bundled.mod_id,
            version: Version::parse(&bundled.version, context.loader),
            dependencies: &bundled.dependencies,
            environment: bundled.environment,
            provides: &bundled.provides,
            language_loader: bundled.language_loader.as_ref(),
            embedded_artifacts: &bundled.embedded_artifacts,
            owner: Some((SolverPackage::top_level(owner), owner_version.clone())),
            bundled: Vec::new(),
        },
        context,
    );
    for (index, child) in bundled.bundled.iter().enumerate() {
        let mut child_path = path.clone();
        child_path.push(index);
        register_bundled_candidate(
            provider,
            availability,
            child,
            owner,
            owner_version,
            child_path,
            context,
        );
    }
}

fn register_bundled_lock(
    provider: &mut OrbitDependencyProvider,
    availability: &mut AvailabilityMap,
    bundled: &BundledMod,
    owner: &str,
    owner_version: &Version,
    path: Vec<usize>,
    context: &GraphContext<'_>,
) {
    let version = Version::parse(&bundled.version, context.loader);
    let package = SolverPackage::Bundled {
        owner: owner.to_string(),
        owner_version: owner_version.clone(),
        path: path.clone(),
        mod_id: bundled.mod_id.clone(),
    };
    register_module(
        provider,
        availability,
        ModuleRegistration {
            package,
            mod_id: &bundled.mod_id,
            version,
            dependencies: &bundled.dependencies,
            environment: bundled.environment,
            provides: &bundled.provides,
            language_loader: bundled.language_loader.as_ref(),
            embedded_artifacts: &bundled.embedded_artifacts,
            owner: Some((SolverPackage::top_level(owner), owner_version.clone())),
            bundled: Vec::new(),
        },
        context,
    );
    for (index, child) in bundled.bundled.iter().enumerate() {
        let mut child_path = path.clone();
        child_path.push(index);
        register_bundled_lock(
            provider,
            availability,
            child,
            owner,
            owner_version,
            child_path,
            context,
        );
    }
}

fn bundled_links(
    mods: &[BundledMod],
    owner: &str,
    owner_version: &Version,
    loader: &str,
) -> Vec<(SolverPackage, Version)> {
    fn collect(
        mods: &[BundledMod],
        owner: &str,
        owner_version: &Version,
        loader: &str,
        prefix: &[usize],
        output: &mut Vec<(SolverPackage, Version)>,
    ) {
        for (index, metadata) in mods.iter().enumerate() {
            let mut path = prefix.to_vec();
            path.push(index);
            output.push((
                SolverPackage::Bundled {
                    owner: owner.to_string(),
                    owner_version: owner_version.clone(),
                    path: path.clone(),
                    mod_id: metadata.mod_id.clone(),
                },
                Version::parse(&metadata.version, loader),
            ));
            collect(
                &metadata.bundled,
                owner,
                owner_version,
                loader,
                &path,
                output,
            );
        }
    }

    let mut output = Vec::new();
    collect(mods, owner, owner_version, loader, &[], &mut output);
    output
}

fn candidate_bundled_links(
    mods: &[BundledCandidate],
    owner: &str,
    owner_version: &Version,
    loader: &str,
) -> Vec<(SolverPackage, Version)> {
    fn collect(
        mods: &[BundledCandidate],
        owner: &str,
        owner_version: &Version,
        loader: &str,
        prefix: &[usize],
        output: &mut Vec<(SolverPackage, Version)>,
    ) {
        for (index, metadata) in mods.iter().enumerate() {
            let mut path = prefix.to_vec();
            path.push(index);
            output.push((
                SolverPackage::Bundled {
                    owner: owner.to_string(),
                    owner_version: owner_version.clone(),
                    path: path.clone(),
                    mod_id: metadata.mod_id.clone(),
                },
                Version::parse(&metadata.version, loader),
            ));
            collect(
                &metadata.bundled,
                owner,
                owner_version,
                loader,
                &path,
                output,
            );
        }
    }

    let mut output = Vec::new();
    collect(mods, owner, owner_version, loader, &[], &mut output);
    output
}

fn register_module(
    provider: &mut OrbitDependencyProvider,
    availability: &mut AvailabilityMap,
    registration: ModuleRegistration<'_>,
    context: &GraphContext<'_>,
) {
    let ModuleRegistration {
        package: physical_package,
        mod_id,
        version,
        dependencies: expressions,
        environment,
        provides,
        language_loader,
        embedded_artifacts,
        owner,
        bundled,
    } = registration;
    let GraphContext {
        loader,
        exclusions,
        overrides,
        target,
    } = *context;
    add_version(provider, physical_package.clone(), version.clone());
    let mut dependencies = Vec::new();
    let mut incompatibilities =
        compile_dependency_constraints(expressions, mod_id, loader, exclusions, overrides, target);
    if target != Environment::Both && !environment.applies_to(target) {
        incompatibilities.push(IncompatibilityConstraint {
            terms: Vec::new(),
            reason: format!(
                "{mod_id} is declared for {} but the selected target is {}",
                environment.as_str(),
                target.as_str()
            ),
        });
    }

    if let Some(language_loader) = language_loader {
        dependencies.push((
            logical_package(&language_loader.id),
            dependency_constraint(
                &language_loader.id,
                &language_loader.requirement,
                loader,
                overrides,
            ),
        ));
    }
    for artifact in embedded_artifacts {
        let artifact_package =
            SolverPackage::Logical(LogicalPackage::EmbeddedArtifact(artifact.id.clone()));
        let artifact_version = Version::parse(&artifact.version, "forge");
        add_availability(
            provider,
            availability,
            artifact_package.clone(),
            artifact_version,
            physical_package.clone(),
            version.clone(),
        );
        dependencies.push((
            artifact_package,
            solver_range(Version::parse_constraint(&artifact.requirement, "forge")),
        ));
    }
    if let Some((owner, owner_version)) = owner {
        dependencies.push((owner, Ranges::singleton(owner_version)));
    }
    for (bundled_id, bundled_version) in bundled {
        dependencies.push((bundled_id, Ranges::singleton(bundled_version)));
    }
    add_availability(
        provider,
        availability,
        SolverPackage::Logical(LogicalPackage::Capability(mod_id.to_string())),
        version.clone(),
        physical_package.clone(),
        version.clone(),
    );
    for provided in provides {
        let provided_version = Version::parse(
            provided.version.as_deref().unwrap_or(&version.to_string()),
            loader,
        );
        add_availability(
            provider,
            availability,
            SolverPackage::Logical(LogicalPackage::Capability(provided.id.clone())),
            provided_version,
            physical_package.clone(),
            version.clone(),
        );
    }

    provider.add_package_deps(
        physical_package.clone(),
        version.clone().into(),
        dependencies,
    );
    provider.add_package_incompatibilities(
        physical_package,
        version.into(),
        std::mem::take(&mut incompatibilities),
    );
}

fn add_availability(
    provider: &mut OrbitDependencyProvider,
    availability: &mut AvailabilityMap,
    logical_package: SolverPackage,
    logical_version: Version,
    provider_package: SolverPackage,
    provider_version: Version,
) {
    add_version(provider, logical_package.clone(), logical_version.clone());
    availability
        .entry((logical_package, logical_version))
        .or_default()
        .insert((provider_package, provider_version));
}

fn register_availability(provider: &mut OrbitDependencyProvider, availability: AvailabilityMap) {
    let mut entries: Vec<_> = availability.into_iter().collect();
    entries.sort_by(|(left, _), (right, _)| left.cmp(right));
    for ((logical_package, logical_version), providers) in entries {
        let SolverPackage::Logical(logical) = logical_package.clone() else {
            unreachable!("availability is only registered for logical packages");
        };
        let choice_package = SolverPackage::ProviderChoice {
            logical,
            logical_version: logical_version.clone(),
        };
        for (choice, (physical_package, physical_version)) in providers.into_iter().enumerate() {
            let choice_version = SolverVersion::ProviderChoice(
                u32::try_from(choice).expect("a logical package has fewer than 2^32 providers"),
            );
            add_version(provider, choice_package.clone(), choice_version.clone());
            provider.add_package_deps(
                choice_package.clone(),
                choice_version.clone(),
                vec![(physical_package, Ranges::singleton(physical_version))],
            );
            provider.add_package_incompatibilities(
                choice_package.clone(),
                choice_version,
                Vec::new(),
            );
        }
        provider.add_package_deps(
            logical_package.clone(),
            logical_version.clone().into(),
            vec![(choice_package, Ranges::full())],
        );
        provider.add_package_incompatibilities(logical_package, logical_version.into(), Vec::new());
    }
}

fn root_dependencies(
    manifest: &OrbitManifest,
    loader: &str,
    overrides: &OverrideMap,
    target: Environment,
) -> Vec<(SolverPackage, Ranges<SolverVersion>)> {
    manifest
        .dependencies
        .iter()
        .filter(|(_, spec)| manifest_environment(spec.env()).applies_to(target))
        .map(|(name, spec)| {
            (
                logical_package(name),
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
) -> Ranges<SolverVersion> {
    solver_range(Version::parse_constraint(
        overrides
            .get(package)
            .map(String::as_str)
            .unwrap_or(constraint),
        loader,
    ))
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

fn add_version(
    provider: &mut OrbitDependencyProvider,
    package: SolverPackage,
    version: impl Into<SolverVersion>,
) {
    let version = version.into();
    let versions = provider.versions.entry(package).or_default();
    if !versions.contains(&version) {
        versions.push(version);
    }
}

pub(super) fn logical_package(package: &str) -> SolverPackage {
    SolverPackage::logical(package)
}

pub(crate) fn is_platform_package(package: &str) -> bool {
    matches!(
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

    fn candidate(version: &str, embedded_artifacts: Vec<EmbeddedArtifact>) -> CandidateVersion {
        CandidateVersion {
            jar_version: version.to_string(),
            dependencies: Vec::new(),
            environment: Environment::Both,
            provides: Vec::new(),
            language_loader: None,
            embedded_artifacts,
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
            .filter(|provider| solution.get(&SolverPackage::top_level(*provider)).is_some())
            .count();

        assert_eq!(selected_providers, 1, "{solution:?}");
        assert!(
            solution
                .get(&SolverPackage::Logical(LogicalPackage::Capability(
                    "virtual_api".to_string(),
                )))
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

        assert!(
            error.contains("a 1 requires org.example:shared >=1, <2"),
            "{error}"
        );
        assert!(
            error.contains("b 1 requires org.example:shared >=2, <3"),
            "{error}"
        );
        assert!(!error.contains("provider choice"), "{error}");
        assert!(!error.contains("capability"), "{error}");
    }

    #[test]
    fn jarjar_artifact_cannot_come_from_an_unselected_candidate() {
        let mut manifest = manifest();
        manifest.dependencies.shift_remove("b");
        manifest.dependencies.insert(
            "a".to_string(),
            crate::manifest::DependencySpec::Short("[1]".to_string()),
        );
        let artifact = |version: &str, requirement: &str| EmbeddedArtifact {
            id: "org.example:shared".to_string(),
            requirement: requirement.to_string(),
            version: version.to_string(),
            path: format!("META-INF/jarjar/shared-{version}.jar"),
            obfuscated: false,
        };
        let mut locked = package("a", "1", Vec::new());
        locked.embedded_artifacts = vec![artifact("1", "[2]")];
        let lockfile = OrbitLockfile {
            meta: LockMeta {
                mc_version: "1.20.1".to_string(),
                modloader: "forge".to_string(),
                modloader_version: "47.2.0".to_string(),
            },
            packages: vec![locked],
        };
        let candidates = HashMap::from([(
            "a".to_string(),
            vec![candidate("2", vec![artifact("2", "[2]")])],
        )]);

        let graph = build_solver_graph(&manifest, &lockfile, &candidates);

        assert!(
            pubgrub::resolve(&graph.provider, graph.root_package, graph.root_version).is_err(),
            "a selected a@1 must not use the artifact physically bundled in unselected a@2"
        );
    }

    #[test]
    fn equal_bundled_mods_keep_distinct_owner_occurrences() {
        let mut manifest = manifest();
        manifest.dependencies.clear();
        manifest.dependencies.insert(
            "shared".to_string(),
            crate::manifest::DependencySpec::Short("*".to_string()),
        );
        let bundled = || BundledMod {
            mod_id: "shared".to_string(),
            version: "1".to_string(),
            environment: Environment::Both,
            dependencies: Vec::new(),
            provides: Vec::new(),
            language_loader: None,
            embedded_artifacts: Vec::new(),
            bundled: Vec::new(),
        };
        let mut a = package("a", "1", Vec::new());
        a.bundled.push(bundled());
        let mut b = package("b", "1", Vec::new());
        b.bundled.push(bundled());
        let lockfile = OrbitLockfile {
            meta: LockMeta {
                mc_version: "1.20.1".to_string(),
                modloader: "forge".to_string(),
                modloader_version: "47.2.0".to_string(),
            },
            packages: vec![a, b],
        };

        let graph = build_solver_graph(&manifest, &lockfile, &HashMap::new());
        let occurrences = graph
            .provider
            .versions
            .keys()
            .filter(|package| {
                matches!(
                    package,
                    SolverPackage::Bundled { mod_id, .. } if mod_id == "shared"
                )
            })
            .count();
        let solution =
            pubgrub::resolve(&graph.provider, graph.root_package, graph.root_version).unwrap();
        let selected_owners = ["a", "b"]
            .into_iter()
            .filter(|owner| solution.get(&SolverPackage::top_level(*owner)).is_some())
            .count();

        assert_eq!(occurrences, 2);
        assert_eq!(selected_owners, 1, "{solution:?}");
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
