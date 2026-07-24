//! Validates loader ordering and emits non-fatal dependency warnings.

use std::collections::{HashMap, HashSet};

use pubgrub::{IncompatibilityConstraint, IncompatibilityConstraintTerm, Ranges};

use crate::lockfile::{BundledMod, OrbitLockfile};
use crate::metadata::{
    DependencyExpression, DependencyKind, DependencyOrdering, Environment, ProvidedMod,
};
use crate::resolver::provider::OrbitDependencyProvider;
use crate::resolver::types::{
    BundledCandidate, CandidateIdentity, CandidateLocation, CandidateVersion, SolverPackage,
    SolverVersion,
};
use crate::versions::Version;

use super::constraints::relation_reason;
use super::graph::{
    ExclusionMap, OverrideMap, dependency_constraint, is_excluded, logical_package,
};

#[derive(Clone)]
struct ModuleRecord {
    package: SolverPackage,
    solver_version: SolverVersion,
    mod_id: String,
    version: Version,
    dependencies: Vec<DependencyExpression>,
    provides: Vec<ProvidedMod>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn register_ordering_cycles(
    provider: &mut OrbitDependencyProvider,
    lockfile: &OrbitLockfile,
    candidates: &HashMap<String, Vec<CandidateVersion>>,
    loader: &str,
    exclusions: &ExclusionMap,
    overrides: &OverrideMap,
    target: Environment,
) {
    let records = module_records(lockfile, candidates, loader);
    let mut adjacency = vec![Vec::new(); records.len()];
    for (source_index, source) in records.iter().enumerate() {
        for relation in source
            .dependencies
            .iter()
            .flat_map(DependencyExpression::relations)
        {
            if relation.ordering == DependencyOrdering::None
                || !relation.environment.applies_to(target)
                || is_excluded(exclusions, &source.mod_id, &relation.id)
            {
                continue;
            }
            let range =
                dependency_constraint(&relation.id, &relation.requirement, loader, overrides);
            for (target_index, dependency) in records.iter().enumerate() {
                if !provides_matching_version(dependency, &relation.id, &range, loader) {
                    continue;
                }
                let edge = match relation.ordering {
                    DependencyOrdering::Before => (source_index, target_index),
                    DependencyOrdering::After => (target_index, source_index),
                    DependencyOrdering::None => unreachable!(),
                };
                if !adjacency[edge.0].contains(&edge.1) {
                    adjacency[edge.0].push(edge.1);
                }
            }
        }
    }

    let mut cycles = Vec::new();
    let mut seen = HashSet::new();
    for start in 0..records.len() {
        let mut path = Vec::new();
        find_ordering_cycles(
            start,
            start,
            &adjacency,
            &records,
            &mut path,
            &mut seen,
            &mut cycles,
        );
    }
    for cycle in cycles {
        let first = &records[cycle[0]];
        let terms = cycle[1..]
            .iter()
            .map(|index| {
                let record = &records[*index];
                IncompatibilityConstraintTerm::Positive(
                    record.package.clone(),
                    Ranges::singleton(record.solver_version.clone()),
                )
            })
            .collect();
        let mut route: Vec<_> = cycle
            .iter()
            .map(|index| records[*index].mod_id.as_str())
            .collect();
        route.push(first.mod_id.as_str());
        provider.extend_package_incompatibilities(
            first.package.clone(),
            first.solver_version.clone(),
            vec![IncompatibilityConstraint {
                terms,
                reason: format!("load ordering cycle: {}", route.join(" -> ")),
            }],
        );
    }
}

fn module_records(
    lockfile: &OrbitLockfile,
    candidates: &HashMap<String, Vec<CandidateVersion>>,
    loader: &str,
) -> Vec<ModuleRecord> {
    fn insert_bundled_lock(
        records: &mut HashMap<(SolverPackage, SolverVersion), ModuleRecord>,
        bundled: &[BundledMod],
        owner: &str,
        source: &str,
        prefix: &[usize],
        loader: &str,
    ) {
        for (index, metadata) in bundled.iter().enumerate() {
            let mut path = prefix.to_vec();
            path.push(index);
            let identity = CandidateIdentity {
                owner: owner.to_string(),
                source: source.to_string(),
                path: path.clone(),
                location: candidate_location(&metadata.origin),
                installed: true,
            };
            let version = Version::parse(&metadata.version, loader);
            let record = ModuleRecord {
                package: SolverPackage::Mod(metadata.mod_id.clone()),
                solver_version: SolverVersion::candidate(version.clone(), identity),
                mod_id: metadata.mod_id.clone(),
                version,
                dependencies: metadata.dependencies.clone(),
                provides: metadata.provides.clone(),
            };
            records.insert(
                (record.package.clone(), record.solver_version.clone()),
                record,
            );
            insert_bundled_lock(records, &metadata.bundled, owner, source, &path, loader);
        }
    }

    fn insert_bundled_candidate(
        records: &mut HashMap<(SolverPackage, SolverVersion), ModuleRecord>,
        bundled: &[BundledCandidate],
        owner: &str,
        source: &str,
        prefix: &[usize],
        loader: &str,
    ) {
        for (index, metadata) in bundled.iter().enumerate() {
            let mut path = prefix.to_vec();
            path.push(index);
            let identity = CandidateIdentity {
                owner: owner.to_string(),
                source: source.to_string(),
                path: path.clone(),
                location: candidate_location(&metadata.origin),
                installed: false,
            };
            let version = Version::parse(&metadata.version, loader);
            let record = ModuleRecord {
                package: SolverPackage::Mod(metadata.mod_id.clone()),
                solver_version: SolverVersion::candidate(version.clone(), identity),
                mod_id: metadata.mod_id.clone(),
                version,
                dependencies: metadata.dependencies.clone(),
                provides: metadata.provides.clone(),
            };
            records.insert(
                (record.package.clone(), record.solver_version.clone()),
                record,
            );
            insert_bundled_candidate(records, &metadata.bundled, owner, source, &path, loader);
        }
    }

    let mut records = HashMap::new();
    for entry in &lockfile.packages {
        let source = super::graph::locked_source(entry);
        let identity = CandidateIdentity {
            owner: entry.mod_id.clone(),
            source: source.clone(),
            path: Vec::new(),
            location: CandidateLocation::Root,
            installed: true,
        };
        let version = Version::parse(&entry.version, loader);
        let record = ModuleRecord {
            package: SolverPackage::Mod(entry.mod_id.clone()),
            solver_version: SolverVersion::candidate(version.clone(), identity),
            mod_id: entry.mod_id.clone(),
            version,
            dependencies: entry.dependencies.clone(),
            provides: entry.provides.clone(),
        };
        records.insert(
            (record.package.clone(), record.solver_version.clone()),
            record,
        );
        insert_bundled_lock(
            &mut records,
            &entry.bundled,
            &entry.mod_id,
            &source,
            &[],
            loader,
        );
    }
    for (package, versions) in candidates {
        for candidate in versions {
            let identity = CandidateIdentity {
                owner: package.clone(),
                source: candidate.id.clone(),
                path: Vec::new(),
                location: CandidateLocation::Root,
                installed: false,
            };
            let version = Version::parse(&candidate.jar_version, loader);
            let record = ModuleRecord {
                package: SolverPackage::Mod(package.clone()),
                solver_version: SolverVersion::candidate(version.clone(), identity),
                mod_id: package.clone(),
                version,
                dependencies: candidate.dependencies.clone(),
                provides: candidate.provides.clone(),
            };
            records.insert(
                (record.package.clone(), record.solver_version.clone()),
                record,
            );
            insert_bundled_candidate(
                &mut records,
                &candidate.bundled,
                package,
                &candidate.id,
                &[],
                loader,
            );
        }
    }
    let mut records: Vec<_> = records.into_values().collect();
    records.sort_by(|left, right| {
        left.package
            .cmp(&right.package)
            .then_with(|| left.version.cmp(&right.version))
    });
    records
}

pub(crate) fn resolution_warnings(
    lockfile: &OrbitLockfile,
    candidates: &HashMap<String, Vec<CandidateVersion>>,
    solution: &pubgrub::SelectedDependencies<SolverPackage, SolverVersion>,
    loader: &str,
    exclusions: &ExclusionMap,
    overrides: &OverrideMap,
    target: Environment,
) -> Vec<String> {
    let records = module_records(lockfile, candidates, loader);
    let mut warnings = Vec::new();
    for record in records {
        if solution.get(&record.package) != Some(&record.solver_version) {
            continue;
        }
        for relation in record
            .dependencies
            .iter()
            .flat_map(DependencyExpression::relations)
        {
            if !relation.environment.applies_to(target)
                || is_excluded(exclusions, &record.mod_id, &relation.id)
            {
                continue;
            }
            let range =
                dependency_constraint(&relation.id, &relation.requirement, loader, overrides);
            let selected = solution.get(&logical_package(&relation.id));
            let warn = match relation.kind {
                DependencyKind::Recommended => {
                    selected.is_none_or(|version| !range.contains(version))
                }
                DependencyKind::Discouraged => {
                    selected.is_some_and(|version| range.contains(version))
                }
                _ => false,
            };
            if warn {
                warnings.push(relation_reason(
                    &record.mod_id,
                    relation,
                    match relation.kind {
                        DependencyKind::Recommended => "recommends",
                        DependencyKind::Discouraged => "discourages",
                        _ => unreachable!(),
                    },
                ));
            }
        }
    }
    warnings.sort();
    warnings.dedup();
    warnings
}

fn provides_matching_version(
    record: &ModuleRecord,
    id: &str,
    range: &Ranges<SolverVersion>,
    loader: &str,
) -> bool {
    if record.mod_id == id && range.contains(&record.solver_version) {
        return true;
    }
    record.provides.iter().any(|provided| {
        let Some(identity) = record.solver_version.candidate_identity() else {
            return false;
        };
        provided.id == id
            && range.contains(&SolverVersion::candidate(
                Version::parse(
                    provided
                        .version
                        .as_deref()
                        .unwrap_or(&record.version.to_string()),
                    loader,
                ),
                identity.clone(),
            ))
    })
}

fn candidate_location(origin: &crate::jar::JarModOrigin) -> CandidateLocation {
    match origin {
        crate::jar::JarModOrigin::Root => CandidateLocation::Root,
        crate::jar::JarModOrigin::SameFile => CandidateLocation::SameFile,
        crate::jar::JarModOrigin::Nested { .. } => CandidateLocation::Nested,
    }
}

#[allow(clippy::too_many_arguments)]
fn find_ordering_cycles(
    start: usize,
    current: usize,
    adjacency: &[Vec<usize>],
    records: &[ModuleRecord],
    path: &mut Vec<usize>,
    seen: &mut HashSet<Vec<(SolverPackage, SolverVersion)>>,
    output: &mut Vec<Vec<usize>>,
) {
    if path.contains(&current) {
        return;
    }
    if path
        .iter()
        .any(|index| records[*index].package == records[current].package)
    {
        return;
    }
    path.push(current);
    for next in &adjacency[current] {
        if *next < start {
            continue;
        }
        if *next == start {
            let mut packages = HashSet::new();
            if path
                .iter()
                .all(|index| packages.insert(&records[*index].package))
            {
                let mut key: Vec<_> = path
                    .iter()
                    .map(|index| {
                        (
                            records[*index].package.clone(),
                            records[*index].solver_version.clone(),
                        )
                    })
                    .collect();
                key.sort();
                if seen.insert(key) {
                    output.push(path.clone());
                }
            }
        } else {
            find_ordering_cycles(start, *next, adjacency, records, path, seen, output);
        }
    }
    path.pop();
}
