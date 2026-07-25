//! Builds the loader-independent constraint graph consumed by PubGrub.

use std::collections::{HashMap, HashSet};

use pubgrub::{IncompatibilityConstraint, IncompatibilityConstraintTerm, Ranges};

use crate::lockfile::{BundledMod, OrbitLockfile};
use crate::manifest::OrbitManifest;
use crate::metadata::{
    DependencyExpression, EmbeddedArtifact, Environment, LanguageLoaderRequirement,
    ModLoadCondition, ProvidedMod,
};
use crate::resolver::provider::{ModulePriority, OrbitDependencyProvider};
use crate::resolver::types::{
    BundledCandidate, CandidateIdentity, CandidateLocation, CandidateVersion, PlatformCandidate,
    SolverPackage, SolverVersion, solver_range,
};
use crate::versions::Version;

use super::constraints::compile_dependency_constraints;
use super::ordering::register_ordering_cycles;

pub(crate) type ExclusionMap = HashMap<String, HashSet<String>>;
pub(crate) type OverrideMap = HashMap<String, String>;

#[derive(Clone, Copy)]
struct GraphContext<'a> {
    loader: &'a str,
    exclusions: &'a ExclusionMap,
    overrides: &'a OverrideMap,
    target: Environment,
}

struct ModuleRegistration<'a> {
    package: SolverPackage,
    solver_version: SolverVersion,
    mod_id: &'a str,
    version: Version,
    dependencies: &'a [DependencyExpression],
    environment: Environment,
    provides: &'a [ProvidedMod],
    language_loader: Option<&'a LanguageLoaderRequirement>,
    embedded_artifacts: &'a [EmbeddedArtifact],
    source_artifact: Option<&'a EmbeddedArtifact>,
    owner: Option<(SolverPackage, SolverVersion)>,
    identity: CandidateIdentity,
    parent_priority: Vec<ModulePriority>,
    bundled: Vec<BundledLink>,
}

#[derive(Clone)]
struct BundledLink {
    package: SolverPackage,
    version: SolverVersion,
    mod_id: String,
    load_condition: ModLoadCondition,
    origin: crate::jar::JarModOrigin,
}

#[derive(Clone)]
struct ContainedRegistration<'a> {
    owner: &'a str,
    source: &'a str,
    path: Vec<usize>,
    installed: bool,
    parent: (SolverPackage, SolverVersion),
    parent_priority: Vec<ModulePriority>,
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
    loader_package: Option<&PlatformCandidate>,
) -> SolverGraph {
    build_solver_graph_for_target(
        manifest,
        lockfile,
        candidates,
        loader_package,
        Environment::Both,
    )
}

pub(crate) fn build_solver_graph_for_target(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    candidates: &HashMap<String, Vec<CandidateVersion>>,
    loader_package: Option<&PlatformCandidate>,
    target: Environment,
) -> SolverGraph {
    let loader = &manifest.project.modloader;
    let exclusions = manifest_exclusions(manifest);
    let overrides = manifest_overrides(manifest);
    let mut provider = OrbitDependencyProvider::new();
    let context = GraphContext {
        loader,
        exclusions: &exclusions,
        overrides: &overrides,
        target,
    };

    register_platform_packages(&mut provider, manifest, loader_package, &context);
    register_lockfile(&mut provider, lockfile, &context);
    register_candidate_map(&mut provider, candidates, &context);
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
    let root_version = SolverVersion::platform(Version::zero());
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

fn register_platform_packages(
    provider: &mut OrbitDependencyProvider,
    manifest: &OrbitManifest,
    loader_package: Option<&PlatformCandidate>,
    context: &GraphContext<'_>,
) {
    let loader = &manifest.project.modloader;
    register_leaf(
        provider,
        "minecraft",
        Version::parse(&manifest.project.mc_version, loader),
    );

    let canonical_loader = match loader.as_str() {
        "fabric" => "fabricloader",
        "quilt" => "quilt_loader",
        other => other,
    };
    let loader_version = Version::parse(&manifest.project.modloader_version, loader);
    if let Some(metadata) = loader_package
        .filter(|metadata| metadata.mod_id == canonical_loader)
        .filter(|metadata| Version::parse(&metadata.version, loader) == loader_version)
    {
        register_platform_candidate(provider, metadata, context);
    } else {
        register_leaf(provider, canonical_loader, loader_version.clone());
    }

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

fn register_platform_candidate(
    provider: &mut OrbitDependencyProvider,
    candidate: &PlatformCandidate,
    context: &GraphContext<'_>,
) {
    let version = Version::parse(&candidate.version, context.loader);
    let source = format!("platform:{}:{}", candidate.mod_id, candidate.version);
    let identity = CandidateIdentity {
        owner: candidate.mod_id.clone(),
        source: source.clone(),
        path: Vec::new(),
        location: CandidateLocation::Root,
        installed: true,
    };
    let package = SolverPackage::Platform(candidate.mod_id.clone());
    let solver_version = SolverVersion::platform(version.clone());
    register_module(
        provider,
        ModuleRegistration {
            package: package.clone(),
            solver_version: solver_version.clone(),
            mod_id: &candidate.mod_id,
            version: version.clone(),
            dependencies: &candidate.dependencies,
            environment: candidate.environment,
            provides: &candidate.provides,
            language_loader: candidate.language_loader.as_ref(),
            embedded_artifacts: &candidate.embedded_artifacts,
            source_artifact: None,
            owner: None,
            identity: identity.clone(),
            parent_priority: Vec::new(),
            bundled: candidate_bundled_links(&candidate.bundled, &identity, context.loader),
        },
        context,
    );
    for (index, bundled) in candidate.bundled.iter().enumerate() {
        register_bundled_candidate(
            provider,
            bundled,
            ContainedRegistration {
                owner: &candidate.mod_id,
                source: &source,
                path: vec![index],
                installed: true,
                parent: (package.clone(), solver_version.clone()),
                parent_priority: vec![ModulePriority {
                    mod_id: candidate.mod_id.clone(),
                    version: version.clone(),
                }],
            },
            context,
        );
    }
}

fn register_leaf(provider: &mut OrbitDependencyProvider, package: &str, version: Version) {
    let package = SolverPackage::Platform(package.to_string());
    add_version(provider, package.clone(), version.clone());
    provider.add_package_deps(package.clone(), version.clone().into(), Vec::new());
    provider.add_package_incompatibilities(package, version.into(), Vec::new());
}

fn register_candidate_versions(
    provider: &mut OrbitDependencyProvider,
    package: &str,
    candidates: &[CandidateVersion],
    context: &GraphContext<'_>,
) {
    for candidate in candidates {
        let version = Version::parse(&candidate.jar_version, context.loader);
        let source = candidate.id.clone();
        let identity = CandidateIdentity {
            owner: package.to_string(),
            source: source.clone(),
            path: Vec::new(),
            location: CandidateLocation::Root,
            installed: false,
        };
        let solver_version = SolverVersion::candidate(version.clone(), identity.clone());
        let solver_package = SolverPackage::Mod(package.to_string());
        let bundled = candidate_bundled_links(&candidate.bundled, &identity, context.loader);
        register_module(
            provider,
            ModuleRegistration {
                package: solver_package.clone(),
                solver_version: solver_version.clone(),
                mod_id: package,
                version: version.clone(),
                dependencies: &candidate.dependencies,
                environment: candidate.environment,
                provides: &candidate.provides,
                language_loader: candidate.language_loader.as_ref(),
                embedded_artifacts: &candidate.embedded_artifacts,
                source_artifact: None,
                owner: None,
                identity: identity.clone(),
                parent_priority: Vec::new(),
                bundled,
            },
            context,
        );
        for (index, bundled) in candidate.bundled.iter().enumerate() {
            register_bundled_candidate(
                provider,
                bundled,
                ContainedRegistration {
                    owner: package,
                    source: &source,
                    path: vec![index],
                    installed: false,
                    parent: (solver_package.clone(), solver_version.clone()),
                    parent_priority: vec![ModulePriority {
                        mod_id: package.to_string(),
                        version: version.clone(),
                    }],
                },
                context,
            );
        }
    }
}

fn register_lockfile(
    provider: &mut OrbitDependencyProvider,
    lockfile: &OrbitLockfile,
    context: &GraphContext<'_>,
) {
    for entry in &lockfile.packages {
        let version = Version::parse(&entry.version, context.loader);
        let source = locked_source(entry);
        let identity = CandidateIdentity {
            owner: entry.mod_id.clone(),
            source: source.clone(),
            path: Vec::new(),
            location: CandidateLocation::Root,
            installed: true,
        };
        let solver_version = SolverVersion::candidate(version.clone(), identity.clone());
        let solver_package = SolverPackage::Mod(entry.mod_id.clone());
        let bundled_links = bundled_links(&entry.bundled, &identity, context.loader);
        register_module(
            provider,
            ModuleRegistration {
                package: solver_package.clone(),
                solver_version: solver_version.clone(),
                mod_id: &entry.mod_id,
                version: version.clone(),
                dependencies: &entry.dependencies,
                environment: entry.environment,
                provides: &entry.provides,
                language_loader: entry.language_loader.as_ref(),
                embedded_artifacts: &entry.embedded_artifacts,
                source_artifact: None,
                owner: None,
                identity: identity.clone(),
                parent_priority: Vec::new(),
                bundled: bundled_links,
            },
            context,
        );
        for (index, bundled) in entry.bundled.iter().enumerate() {
            register_bundled_lock(
                provider,
                bundled,
                ContainedRegistration {
                    owner: &entry.mod_id,
                    source: &source,
                    path: vec![index],
                    installed: true,
                    parent: (solver_package.clone(), solver_version.clone()),
                    parent_priority: vec![ModulePriority {
                        mod_id: entry.mod_id.clone(),
                        version: version.clone(),
                    }],
                },
                context,
            );
        }
    }
}

fn register_candidate_map(
    provider: &mut OrbitDependencyProvider,
    candidates: &HashMap<String, Vec<CandidateVersion>>,
    context: &GraphContext<'_>,
) {
    let mut packages: Vec<_> = candidates.iter().collect();
    packages.sort_by_key(|(package, _)| *package);
    for (package, versions) in packages {
        register_candidate_versions(provider, package, versions, context);
    }
}

fn register_bundled_candidate(
    provider: &mut OrbitDependencyProvider,
    bundled: &BundledCandidate,
    contained: ContainedRegistration<'_>,
    context: &GraphContext<'_>,
) {
    let ContainedRegistration {
        owner,
        source,
        path,
        installed,
        parent,
        parent_priority,
    } = contained;
    let version = Version::parse(&bundled.version, context.loader);
    let identity = CandidateIdentity {
        owner: owner.to_string(),
        source: source.to_string(),
        path: path.clone(),
        location: candidate_location(&bundled.origin),
        installed,
    };
    let package = SolverPackage::Mod(bundled.mod_id.clone());
    let solver_version = SolverVersion::candidate(version.clone(), identity.clone());
    register_module(
        provider,
        ModuleRegistration {
            package: package.clone(),
            solver_version: solver_version.clone(),
            mod_id: &bundled.mod_id,
            version: version.clone(),
            dependencies: &bundled.dependencies,
            environment: bundled.environment,
            provides: &bundled.provides,
            language_loader: bundled.language_loader.as_ref(),
            embedded_artifacts: &bundled.embedded_artifacts,
            source_artifact: nested_artifact(&bundled.origin),
            owner: Some(parent),
            identity: identity.clone(),
            parent_priority: parent_priority.clone(),
            bundled: candidate_bundled_links(&bundled.bundled, &identity, context.loader),
        },
        context,
    );
    for (index, child) in bundled.bundled.iter().enumerate() {
        let mut child_path = path.clone();
        child_path.push(index);
        let mut child_parent_priority = vec![ModulePriority {
            mod_id: bundled.mod_id.clone(),
            version: version.clone(),
        }];
        child_parent_priority.extend(parent_priority.iter().cloned());
        register_bundled_candidate(
            provider,
            child,
            ContainedRegistration {
                owner,
                source,
                path: child_path,
                installed,
                parent: (package.clone(), solver_version.clone()),
                parent_priority: child_parent_priority,
            },
            context,
        );
    }
}

fn register_bundled_lock(
    provider: &mut OrbitDependencyProvider,
    bundled: &BundledMod,
    contained: ContainedRegistration<'_>,
    context: &GraphContext<'_>,
) {
    let ContainedRegistration {
        owner,
        source,
        path,
        installed,
        parent,
        parent_priority,
    } = contained;
    let version = Version::parse(&bundled.version, context.loader);
    let identity = CandidateIdentity {
        owner: owner.to_string(),
        source: source.to_string(),
        path: path.clone(),
        location: candidate_location(&bundled.origin),
        installed,
    };
    let package = SolverPackage::Mod(bundled.mod_id.clone());
    let solver_version = SolverVersion::candidate(version.clone(), identity.clone());
    register_module(
        provider,
        ModuleRegistration {
            package: package.clone(),
            solver_version: solver_version.clone(),
            mod_id: &bundled.mod_id,
            version: version.clone(),
            dependencies: &bundled.dependencies,
            environment: bundled.environment,
            provides: &bundled.provides,
            language_loader: bundled.language_loader.as_ref(),
            embedded_artifacts: &bundled.embedded_artifacts,
            source_artifact: nested_artifact(&bundled.origin),
            owner: Some(parent),
            identity: identity.clone(),
            parent_priority: parent_priority.clone(),
            bundled: bundled_links(&bundled.bundled, &identity, context.loader),
        },
        context,
    );
    for (index, child) in bundled.bundled.iter().enumerate() {
        let mut child_path = path.clone();
        child_path.push(index);
        let mut child_parent_priority = vec![ModulePriority {
            mod_id: bundled.mod_id.clone(),
            version: version.clone(),
        }];
        child_parent_priority.extend(parent_priority.iter().cloned());
        register_bundled_lock(
            provider,
            child,
            ContainedRegistration {
                owner,
                source,
                path: child_path,
                installed,
                parent: (package.clone(), solver_version.clone()),
                parent_priority: child_parent_priority,
            },
            context,
        );
    }
}

fn bundled_links(
    mods: &[BundledMod],
    parent: &CandidateIdentity,
    loader: &str,
) -> Vec<BundledLink> {
    mods.iter()
        .enumerate()
        .map(|(index, metadata)| {
            bundled_link(
                &metadata.mod_id,
                &metadata.version,
                metadata.load_condition,
                &metadata.origin,
                parent,
                index,
                loader,
            )
        })
        .collect()
}

fn candidate_bundled_links(
    mods: &[BundledCandidate],
    parent: &CandidateIdentity,
    loader: &str,
) -> Vec<BundledLink> {
    mods.iter()
        .enumerate()
        .map(|(index, metadata)| {
            bundled_link(
                &metadata.mod_id,
                &metadata.version,
                metadata.load_condition,
                &metadata.origin,
                parent,
                index,
                loader,
            )
        })
        .collect()
}

fn bundled_link(
    mod_id: &str,
    version: &str,
    load_condition: ModLoadCondition,
    origin: &crate::jar::JarModOrigin,
    parent: &CandidateIdentity,
    index: usize,
    loader: &str,
) -> BundledLink {
    let mut path = parent.path.clone();
    path.push(index);
    let identity = CandidateIdentity {
        owner: parent.owner.clone(),
        source: parent.source.clone(),
        path,
        location: candidate_location(origin),
        installed: parent.installed,
    };
    BundledLink {
        package: SolverPackage::Mod(mod_id.to_string()),
        version: SolverVersion::candidate(Version::parse(version, loader), identity),
        mod_id: mod_id.to_string(),
        load_condition,
        origin: origin.clone(),
    }
}

fn candidate_location(origin: &crate::jar::JarModOrigin) -> CandidateLocation {
    match origin {
        crate::jar::JarModOrigin::Root => CandidateLocation::Root,
        crate::jar::JarModOrigin::SameFile => CandidateLocation::SameFile,
        crate::jar::JarModOrigin::Nested { .. } => CandidateLocation::Nested,
    }
}

fn nested_artifact(origin: &crate::jar::JarModOrigin) -> Option<&EmbeddedArtifact> {
    match origin {
        crate::jar::JarModOrigin::Nested {
            artifact: Some(artifact),
            ..
        } => Some(artifact),
        _ => None,
    }
}

pub(crate) fn locked_source(entry: &crate::lockfile::PackageEntry) -> String {
    format!(
        "lock:{}:{}:{}:{}",
        entry.provider,
        entry.source_version_id().unwrap_or_default(),
        if entry.sha512.is_empty() {
            &entry.sha256
        } else {
            &entry.sha512
        },
        entry.filename
    )
}

fn register_module(
    provider: &mut OrbitDependencyProvider,
    registration: ModuleRegistration<'_>,
    context: &GraphContext<'_>,
) {
    let ModuleRegistration {
        package,
        solver_version,
        mod_id,
        version,
        dependencies: expressions,
        environment,
        provides,
        language_loader,
        embedded_artifacts,
        source_artifact,
        owner,
        identity,
        parent_priority,
        bundled,
    } = registration;
    let GraphContext {
        loader,
        exclusions,
        overrides,
        target,
    } = *context;
    add_version(provider, package.clone(), solver_version.clone());
    provider.add_candidate_priority(
        identity.clone(),
        if loader == "fabric" {
            parent_priority
        } else {
            Vec::new()
        },
    );
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
    for provided in provides {
        let provided_version = Version::parse(
            provided.version.as_deref().unwrap_or(&version.to_string()),
            loader,
        );
        register_proxy_candidate(
            provider,
            &mut dependencies,
            SolverPackage::Mod(provided.id.clone()),
            provided_version,
            identity.clone(),
            package.clone(),
            solver_version.clone(),
        );
    }

    if let Some(artifact) = source_artifact {
        let artifact_package = SolverPackage::EmbeddedArtifact(artifact.id.clone());
        let artifact_version = Version::parse(&artifact.version, "forge");
        register_proxy_candidate(
            provider,
            &mut dependencies,
            artifact_package.clone(),
            artifact_version,
            identity.clone(),
            package.clone(),
            solver_version.clone(),
        );
    }

    for artifact in embedded_artifacts {
        let artifact_package = SolverPackage::EmbeddedArtifact(artifact.id.clone());
        let artifact_version = Version::parse(&artifact.version, "forge");
        let has_mod_provider = bundled.iter().any(|link| {
            nested_artifact(&link.origin).is_some_and(|source| {
                source.id == artifact.id
                    && source.path == artifact.path
                    && source.version == artifact.version
            })
        });
        if !has_mod_provider {
            register_proxy_candidate(
                provider,
                &mut dependencies,
                artifact_package.clone(),
                artifact_version,
                identity.clone(),
                package.clone(),
                solver_version.clone(),
            );
        }
        dependencies.push((
            artifact_package,
            solver_range(Version::parse_constraint(&artifact.requirement, "forge")),
        ));
    }
    if let Some((owner_package, owner_version)) = owner {
        dependencies.push((owner_package, Ranges::singleton(owner_version)));
    }

    let mut nested_groups: HashMap<String, (bool, bool)> = HashMap::new();
    for link in &bundled {
        match &link.origin {
            crate::jar::JarModOrigin::SameFile | crate::jar::JarModOrigin::Root => {
                dependencies.push((
                    link.package.clone(),
                    Ranges::singleton(link.version.clone()),
                ));
            }
            crate::jar::JarModOrigin::Nested {
                artifact: Some(_), ..
            } => {}
            crate::jar::JarModOrigin::Nested { artifact: None, .. } => {
                let group = nested_groups.entry(link.mod_id.clone()).or_default();
                match link.load_condition {
                    ModLoadCondition::Always => group.0 = true,
                    ModLoadCondition::IfPossible => group.1 = true,
                    ModLoadCondition::IfRequired => {}
                }
            }
        }
    }
    for (nested_mod_id, (always, if_possible)) in nested_groups {
        let nested_package = logical_package(&nested_mod_id);
        if always {
            dependencies.push((nested_package, Ranges::full()));
        } else if if_possible {
            let choice = SolverPackage::LoadPreference {
                parent: identity.clone(),
                mod_id: nested_mod_id,
            };
            let omitted = SolverVersion::LoadPreference(false);
            let loaded = SolverVersion::LoadPreference(true);
            provider.add_package_versions(choice.clone(), vec![omitted.clone(), loaded.clone()]);
            provider.add_package_deps(choice.clone(), omitted.clone(), Vec::new());
            provider.add_package_incompatibilities(choice.clone(), omitted, Vec::new());
            provider.add_package_deps(
                choice.clone(),
                loaded.clone(),
                vec![(nested_package, Ranges::full())],
            );
            provider.add_package_incompatibilities(choice.clone(), loaded, Vec::new());
            dependencies.push((choice, Ranges::full()));
        }
    }

    provider.add_package_deps(package.clone(), solver_version.clone(), dependencies);
    provider.add_package_incompatibilities(
        package,
        solver_version,
        std::mem::take(&mut incompatibilities),
    );
}

fn register_proxy_candidate(
    provider: &mut OrbitDependencyProvider,
    dependencies: &mut Vec<(SolverPackage, Ranges<SolverVersion>)>,
    proxy_package: SolverPackage,
    semantic_version: Version,
    identity: CandidateIdentity,
    package: SolverPackage,
    version: SolverVersion,
) {
    let proxy_version = SolverVersion::candidate(semantic_version, identity);
    add_version(provider, proxy_package.clone(), proxy_version.clone());
    provider.add_package_deps(
        proxy_package.clone(),
        proxy_version.clone(),
        vec![(package, Ranges::singleton(version))],
    );
    provider.add_package_incompatibilities(
        proxy_package.clone(),
        proxy_version.clone(),
        Vec::new(),
    );
    dependencies.push((proxy_package, Ranges::singleton(proxy_version)));
}

fn root_dependencies(
    manifest: &OrbitManifest,
    loader: &str,
    overrides: &OverrideMap,
    target: Environment,
) -> Vec<(SolverPackage, Ranges<SolverVersion>)> {
    let mut dependencies = manifest
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
        .collect::<Vec<_>>();
    // Minecraft and the selected Loader are not optional leaves: their actual
    // JARs are part of every launch. Keeping them at the root also makes the
    // Loader's own dependencies and contained-module load conditions use the
    // same solver path as ordinary packages.
    dependencies.push((
        SolverPackage::Platform("minecraft".to_string()),
        solver_range(Ranges::singleton(Version::parse(
            &manifest.project.mc_version,
            loader,
        ))),
    ));
    let canonical_loader = match loader {
        "fabric" => "fabricloader",
        "quilt" => "quilt_loader",
        other => other,
    };
    dependencies.push((
        SolverPackage::Platform(canonical_loader.to_string()),
        solver_range(Ranges::singleton(Version::parse(
            &manifest.project.modloader_version,
            loader,
        ))),
    ));
    dependencies
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
[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }
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
            id: format!("candidate-{version}"),
            filename: format!("candidate-{version}.jar"),
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
    fn loader_bundled_module_satisfies_regular_mod_dependency() {
        let manifest: OrbitManifest = toml::from_str(
            r#"
[project]
name = "test"
mc_version = "1.21.1"
modloader = "fabric"
modloader_version = "0.19.2"
[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }
[dependencies]
carpet_tis = "*"
"#,
        )
        .unwrap();
        let lockfile = OrbitLockfile {
            meta: LockMeta {
                mc_version: "1.21.1".to_string(),
                modloader: "fabric".to_string(),
                modloader_version: "0.19.2".to_string(),
            },
            packages: vec![package(
                "carpet_tis",
                "1",
                vec![dependency(
                    "mixinextras",
                    ">=0.3.0",
                    DependencyKind::Required,
                )],
            )],
        };
        let loader_package = PlatformCandidate {
            mod_id: "fabricloader".to_string(),
            version: "0.19.2".to_string(),
            dependencies: Vec::new(),
            environment: Environment::Both,
            provides: Vec::new(),
            language_loader: None,
            embedded_artifacts: Vec::new(),
            bundled: vec![BundledCandidate {
                mod_id: "mixinextras".to_string(),
                version: "0.5.4".to_string(),
                load_condition: ModLoadCondition::IfPossible,
                origin: crate::jar::JarModOrigin::Nested {
                    path: "META-INF/jars/mixinextras-fabric-0.5.4.jar".to_string(),
                    artifact: None,
                },
                environment: Environment::Both,
                dependencies: Vec::new(),
                provides: Vec::new(),
                language_loader: None,
                embedded_artifacts: Vec::new(),
                bundled: Vec::new(),
            }],
        };

        let graph =
            build_solver_graph(&manifest, &lockfile, &HashMap::new(), Some(&loader_package));
        let solution =
            pubgrub::resolve(&graph.provider, graph.root_package, graph.root_version).unwrap();

        assert!(
            solution
                .get(&SolverPackage::Mod("mixinextras".to_string()))
                .is_some(),
            "{solution:?}"
        );
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

        let graph = build_solver_graph(&manifest, &lockfile, &HashMap::new(), None);
        let solution =
            pubgrub::resolve(&graph.provider, graph.root_package, graph.root_version).unwrap();
        let selected_providers = ["provider_one", "provider_two"]
            .into_iter()
            .filter(|provider| {
                solution
                    .iter()
                    .map(|(package, _)| package)
                    .any(|package| package.top_level_mod_id() == Some(*provider))
            })
            .count();

        assert_eq!(selected_providers, 1, "{solution:?}");
        assert!(
            solution
                .get(&SolverPackage::Mod("virtual_api".to_string()))
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
        let graph = build_solver_graph(&manifest, &lockfile, &HashMap::new(), None);
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
        let graph = build_solver_graph(&manifest, &lockfile, &HashMap::new(), None);
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
        let graph = build_solver_graph(&manifest, &lockfile, &HashMap::new(), None);
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
        let graph = build_solver_graph(&manifest, &lockfile, &HashMap::new(), None);
        assert!(pubgrub::resolve(&graph.provider, graph.root_package, graph.root_version).is_err());

        manifest.dependencies.shift_remove("c");
        let graph = build_solver_graph(&manifest, &lockfile, &HashMap::new(), None);
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
            None,
            Environment::Server,
        );
        assert!(
            pubgrub::resolve(&server.provider, server.root_package, server.root_version).is_ok()
        );
        let client = build_solver_graph_for_target(
            &manifest,
            &lockfile,
            &HashMap::new(),
            None,
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

        let graph = build_solver_graph(&manifest, &lockfile, &candidates, None);

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
            load_condition: ModLoadCondition::Always,
            origin: crate::jar::JarModOrigin::SameFile,
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

        let graph = build_solver_graph(&manifest, &lockfile, &HashMap::new(), None);
        let occurrences = graph
            .provider
            .versions
            .get(&SolverPackage::Mod("shared".to_string()))
            .map_or(0, Vec::len);
        let solution =
            pubgrub::resolve(&graph.provider, graph.root_package, graph.root_version).unwrap();
        let selected_owners = ["a", "b"]
            .into_iter()
            .filter(|owner| {
                solution
                    .iter()
                    .map(|(package, _)| package)
                    .any(|package| package.top_level_mod_id() == Some(*owner))
            })
            .count();

        assert_eq!(occurrences, 2);
        assert_eq!(selected_owners, 1, "{solution:?}");
    }

    #[test]
    fn duplicate_package_versions_select_the_candidate_whose_metadata_is_compatible() {
        let mut manifest = manifest();
        manifest.dependencies.clear();
        manifest.dependencies.insert(
            "a".to_string(),
            crate::manifest::DependencySpec::Short("*".to_string()),
        );
        let mut incompatible = candidate("1", Vec::new());
        incompatible.id = "incompatible".to_string();
        incompatible.dependencies =
            vec![dependency("minecraft", "[1.21]", DependencyKind::Required)];
        let mut compatible = candidate("1", Vec::new());
        compatible.id = "compatible".to_string();
        compatible.dependencies = vec![dependency(
            "minecraft",
            "[1.20.1]",
            DependencyKind::Required,
        )];
        let candidates = HashMap::from([("a".to_string(), vec![incompatible, compatible])]);

        let graph = build_solver_graph(
            &manifest,
            &OrbitLockfile {
                meta: LockMeta {
                    mc_version: "1.20.1".to_string(),
                    modloader: "forge".to_string(),
                    modloader_version: "47.2.0".to_string(),
                },
                packages: Vec::new(),
            },
            &candidates,
            None,
        );
        let solution =
            pubgrub::resolve(&graph.provider, graph.root_package, graph.root_version).unwrap();
        let selected = solution
            .get(&SolverPackage::Mod("a".to_string()))
            .and_then(SolverVersion::candidate_identity)
            .unwrap();

        assert_eq!(selected.source, "compatible");
        assert_eq!(
            graph.provider.versions[&SolverPackage::Mod("a".to_string())].len(),
            2
        );
    }

    #[test]
    fn fabric_multi_version_nested_mod_selects_only_the_compatible_candidate() {
        let manifest: OrbitManifest = toml::from_str(
            r#"
[project]
name = "test"
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.16.10"
[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }
[dependencies]
wrapper = "*"
"#,
        )
        .unwrap();
        let nested = |path: &str, minecraft: &str| BundledCandidate {
            mod_id: "actual".to_string(),
            version: "1".to_string(),
            load_condition: ModLoadCondition::IfPossible,
            origin: crate::jar::JarModOrigin::Nested {
                path: path.to_string(),
                artifact: None,
            },
            environment: Environment::Both,
            dependencies: vec![dependency("minecraft", minecraft, DependencyKind::Required)],
            provides: Vec::new(),
            language_loader: None,
            embedded_artifacts: Vec::new(),
            bundled: Vec::new(),
        };
        let mut wrapper = candidate("1", Vec::new());
        wrapper.id = "wrapper-source".to_string();
        wrapper.bundled = vec![
            nested("META-INF/jars/actual-1.19.jar", "=1.19"),
            nested("META-INF/jars/actual-1.20.1.jar", "=1.20.1"),
        ];
        let candidates = HashMap::from([("wrapper".to_string(), vec![wrapper])]);
        let graph = build_solver_graph(
            &manifest,
            &OrbitLockfile {
                meta: LockMeta {
                    mc_version: "1.20.1".to_string(),
                    modloader: "fabric".to_string(),
                    modloader_version: "0.16.10".to_string(),
                },
                packages: Vec::new(),
            },
            &candidates,
            None,
        );

        let solution =
            pubgrub::resolve(&graph.provider, graph.root_package, graph.root_version).unwrap();
        let selected = solution
            .get(&SolverPackage::Mod("actual".to_string()))
            .and_then(SolverVersion::candidate_identity)
            .unwrap();

        assert_eq!(selected.path, vec![1]);
        assert!(
            solution
                .get(&SolverPackage::Mod("wrapper".to_string()))
                .is_some()
        );
    }

    #[test]
    fn fabric_if_possible_nested_mod_is_omitted_when_no_candidate_is_compatible() {
        let manifest: OrbitManifest = toml::from_str(
            r#"
[project]
name = "test"
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.16.10"
[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }
[dependencies]
wrapper = "*"
"#,
        )
        .unwrap();
        let nested = BundledCandidate {
            mod_id: "actual".to_string(),
            version: "1".to_string(),
            load_condition: ModLoadCondition::IfPossible,
            origin: crate::jar::JarModOrigin::Nested {
                path: "META-INF/jars/actual-1.19.jar".to_string(),
                artifact: None,
            },
            environment: Environment::Both,
            dependencies: vec![dependency("minecraft", "=1.19", DependencyKind::Required)],
            provides: Vec::new(),
            language_loader: None,
            embedded_artifacts: Vec::new(),
            bundled: Vec::new(),
        };
        let mut wrapper = candidate("1", Vec::new());
        wrapper.bundled = vec![nested];
        let graph = build_solver_graph(
            &manifest,
            &OrbitLockfile {
                meta: LockMeta {
                    mc_version: "1.20.1".to_string(),
                    modloader: "fabric".to_string(),
                    modloader_version: "0.16.10".to_string(),
                },
                packages: Vec::new(),
            },
            &HashMap::from([("wrapper".to_string(), vec![wrapper])]),
            None,
        );

        let solution =
            pubgrub::resolve(&graph.provider, graph.root_package, graph.root_version).unwrap();

        assert!(
            solution
                .get(&SolverPackage::Mod("wrapper".to_string()))
                .is_some()
        );
        assert!(
            solution
                .get(&SolverPackage::Mod("actual".to_string()))
                .is_none()
        );
    }

    #[test]
    fn fabric_equal_nested_candidates_follow_their_parent_priority() {
        let manifest: OrbitManifest = toml::from_str(
            r#"
[project]
name = "test"
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.16.10"
[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }
[dependencies]
a_parent = "*"
z_parent = "*"
"#,
        )
        .unwrap();
        let nested = || BundledCandidate {
            mod_id: "shared".to_string(),
            version: "1".to_string(),
            load_condition: ModLoadCondition::IfPossible,
            origin: crate::jar::JarModOrigin::Nested {
                path: "META-INF/jars/shared.jar".to_string(),
                artifact: None,
            },
            environment: Environment::Both,
            dependencies: Vec::new(),
            provides: Vec::new(),
            language_loader: None,
            embedded_artifacts: Vec::new(),
            bundled: Vec::new(),
        };
        let mut a_parent = candidate("1", Vec::new());
        a_parent.id = "a-parent-source".to_string();
        a_parent.bundled = vec![nested()];
        let mut z_parent = candidate("2", Vec::new());
        z_parent.id = "z-parent-source".to_string();
        z_parent.bundled = vec![nested()];
        let graph = build_solver_graph(
            &manifest,
            &OrbitLockfile {
                meta: LockMeta {
                    mc_version: "1.20.1".to_string(),
                    modloader: "fabric".to_string(),
                    modloader_version: "0.16.10".to_string(),
                },
                packages: Vec::new(),
            },
            &HashMap::from([
                ("a_parent".to_string(), vec![a_parent]),
                ("z_parent".to_string(), vec![z_parent]),
            ]),
            None,
        );

        let solution =
            pubgrub::resolve(&graph.provider, graph.root_package, graph.root_version).unwrap();
        let selected = solution
            .get(&SolverPackage::Mod("shared".to_string()))
            .and_then(SolverVersion::candidate_identity)
            .unwrap();

        assert_eq!(selected.owner, "a_parent");
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
        let graph = build_solver_graph(&manifest, &lockfile, &HashMap::new(), None);
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
