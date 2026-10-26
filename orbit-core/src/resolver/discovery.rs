//! Loader discovery facts conditional on the installed package realization.
//!
//! Catalog membership is not presence. Each fact is guarded by its containing
//! top-level package, and traversal stops at an environment-disabled parent.

use std::collections::HashMap;

use pubgrub::Ranges;

use crate::loader::LoaderKind;
use crate::lockfile::{BundledMod, OrbitLockfile};
use crate::manifest::OrbitManifest;
use crate::metadata::{Environment, ModDependency, ProvidedMod};
use crate::versions::Version;

use super::constraints::Formula;
use super::graph::locked_source;
use super::types::{
    BundledCandidate, CandidateIdentity, CandidateLocation, CandidateVersion, PlatformCandidate,
    SolverPackage, SolverVersion,
};

struct DiscoveryFact {
    owner: SolverPackage,
    realization: Ranges<SolverVersion>,
    version: SolverVersion,
}

struct DiscoveryOwner {
    package: SolverPackage,
    realizations: Ranges<SolverVersion>,
}

pub(super) struct DiscoveryIndex {
    loader: LoaderKind,
    target: Environment,
    active: HashMap<String, Vec<DiscoveryFact>>,
    disabled: HashMap<String, Vec<DiscoveryFact>>,
}

impl DiscoveryIndex {
    pub(super) fn new(
        manifest: &OrbitManifest,
        lockfile: &OrbitLockfile,
        candidates: &HashMap<String, Vec<CandidateVersion>>,
        loader_package: Option<&PlatformCandidate>,
        loader: LoaderKind,
        target: Environment,
    ) -> Self {
        let mut index = Self {
            loader,
            target,
            active: HashMap::new(),
            disabled: HashMap::new(),
        };
        // This is Fabric's schema 0/1 dependency workaround, not a generic
        // relaxation of missing dependencies for other Loaders.
        if loader != LoaderKind::Fabric {
            return index;
        }
        if let Some(platform) = loader_package.filter(|metadata| {
            metadata.mod_id == crate::loader::semantics(loader).canonical_package
                && Version::parse(&metadata.version, loader)
                    == Version::parse(&manifest.project.modloader_version, loader)
        }) {
            let owner = DiscoveryOwner {
                package: SolverPackage::Platform(platform.mod_id.clone()),
                realizations: Ranges::singleton(SolverVersion::platform(Version::parse(
                    &platform.version,
                    loader,
                ))),
            };
            if index.record(
                &platform.mod_id,
                &platform.version,
                platform.environment,
                &platform.provides,
                &owner,
            ) {
                index.candidate_children(&platform.bundled, &owner);
            }
        }
        for entry in &lockfile.packages {
            if !manifest
                .packages
                .get(&entry.mod_id)
                .is_none_or(|spec| spec.enabled())
            {
                continue;
            }
            let root = installed_owner(
                &entry.mod_id,
                &entry.version,
                locked_source(entry),
                true,
                loader,
            );
            if index.record(
                &entry.mod_id,
                &entry.version,
                entry.environment,
                &entry.provides,
                &root,
            ) {
                index.lock_children(&entry.bundled, &root);
            }
        }
        for (id, versions) in candidates {
            if !manifest.packages.get(id).is_none_or(|spec| spec.enabled()) {
                continue;
            }
            for candidate in versions {
                let root = installed_owner(
                    id,
                    &candidate.jar_version,
                    candidate.id.clone(),
                    false,
                    loader,
                );
                if index.record(
                    id,
                    &candidate.jar_version,
                    candidate.environment,
                    &candidate.provides,
                    &root,
                ) {
                    index.candidate_children(&candidate.bundled, &root);
                }
            }
        }
        index
    }

    fn record(
        &mut self,
        id: &str,
        version: &str,
        environment: Environment,
        provides: &[ProvidedMod],
        root: &DiscoveryOwner,
    ) -> bool {
        let active = environment.applies_to(self.target);
        let fact = || DiscoveryFact {
            owner: root.package.clone(),
            realization: root.realizations.clone(),
            version: SolverVersion::platform(Version::parse(version, self.loader)),
        };
        if active {
            self.active.entry(id.to_string()).or_default().push(fact());
            // Fabric's active discovery lookup contains provides aliases, but
            // its environment-disabled lookup is keyed only by the own ID.
            for provided in provides {
                self.active
                    .entry(provided.id.clone())
                    .or_default()
                    .push(fact());
            }
        } else {
            self.disabled
                .entry(id.to_string())
                .or_default()
                .push(fact());
        }
        active
    }

    fn lock_children(&mut self, children: &[BundledMod], root: &DiscoveryOwner) {
        for child in children {
            if self.record(
                &child.mod_id,
                &child.version,
                child.environment,
                &child.provides,
                root,
            ) {
                self.lock_children(&child.bundled, root);
            }
        }
    }

    fn candidate_children(&mut self, children: &[BundledCandidate], root: &DiscoveryOwner) {
        for child in children {
            if self.record(
                &child.mod_id,
                &child.version,
                child.environment,
                &child.provides,
                root,
            ) {
                self.candidate_children(&child.bundled, root);
            }
        }
    }

    pub(super) fn relaxation(
        &self,
        relation: &ModDependency,
        allowed: &Ranges<SolverVersion>,
    ) -> Option<Formula> {
        let matches: Vec<_> = self
            .disabled
            .get(&relation.id)?
            .iter()
            .filter(|fact| allowed.contains(&fact.version))
            .collect();
        if matches.is_empty() {
            return None;
        }
        Some(Formula::And(vec![
            presence(matches.into_iter()),
            Formula::Not(Box::new(presence(
                self.active.get(&relation.id).into_iter().flatten(),
            ))),
        ]))
    }

    pub(super) fn relaxed_in(
        &self,
        relation: &ModDependency,
        allowed: &Ranges<SolverVersion>,
        solution: &pubgrub::SelectedDependencies<SolverPackage, SolverVersion>,
    ) -> bool {
        let present = |fact: &DiscoveryFact| {
            solution
                .get(&fact.owner)
                .is_some_and(|version| fact.realization.contains(version))
        };
        self.disabled
            .get(&relation.id)
            .into_iter()
            .flatten()
            .any(|fact| allowed.contains(&fact.version) && present(fact))
            && !self
                .active
                .get(&relation.id)
                .into_iter()
                .flatten()
                .any(present)
    }
}

fn presence<'a>(facts: impl Iterator<Item = &'a DiscoveryFact>) -> Formula {
    let mut owners: HashMap<SolverPackage, Ranges<SolverVersion>> = HashMap::new();
    for fact in facts {
        owners
            .entry(fact.owner.clone())
            .and_modify(|range| *range = range.union(&fact.realization))
            .or_insert_with(|| fact.realization.clone());
    }
    let mut owners: Vec<_> = owners.into_iter().collect();
    owners.sort_by(|left, right| left.0.cmp(&right.0));
    Formula::Or(
        owners
            .into_iter()
            .map(|(package, versions)| Formula::Atom {
                package,
                versions: Box::new(versions),
            })
            .collect(),
    )
}

fn installed_owner(
    id: &str,
    version: &str,
    source: String,
    installed: bool,
    loader: LoaderKind,
) -> DiscoveryOwner {
    let version = SolverVersion::candidate(
        Version::parse(version, loader),
        CandidateIdentity {
            owner: id.to_string(),
            source,
            path: Vec::new(),
            location: CandidateLocation::Root,
            installed,
        },
    );
    DiscoveryOwner {
        package: SolverPackage::installed(id),
        realizations: version.same_realization(),
    }
}
