pub mod types;
pub mod version;
pub mod provider;

use std::collections::HashMap;

use crate::lockfile::LockEntry;

use self::types::PackageId;
use crate::resolver::version::NormalizedVersion;
use crate::resolver::provider::OrbitDependencyProvider;
use pubgrub::solver::resolve;

#[derive(Debug)]
pub enum FetchRetryError {
    MissingVersions(PackageId),
    MissingDependencies(PackageId, NormalizedVersion),
}

impl std::fmt::Display for FetchRetryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchRetryError::MissingVersions(pkg) => write!(f, "Missing versions for {}", pkg),
            FetchRetryError::MissingDependencies(pkg, ver) => write!(f, "Missing dependencies for {} {}", pkg, ver),
        }
    }
}

impl std::error::Error for FetchRetryError {}

use crate::providers::ModProvider;
use crate::manifest::OrbitManifest;

/// 求解依赖图。纯函数，无副作用。
///
/// # 参数
/// - `root_deps`: orbit.toml 中的顶层依赖（包名 → 版本约束）
/// - `provider`: 预填充好的数据源
///
/// # 返回
/// - `Ok(HashMap<PackageId, NormalizedVersion>)` — 每个包被选中的版本
/// - `Err(String)` — 人类可读的冲突报告
/// 求解依赖图，带 Fetch-and-Retry 懒加载。
pub async fn resolve_manifest(
    manifest: &OrbitManifest,
    providers: &[Box<dyn ModProvider>],
) -> Result<HashMap<PackageId, crate::providers::ResolvedMod>, String> {
    let mut provider = OrbitDependencyProvider::new();
    let root_pkg = "___orbit_root___".to_string();
    let root_version = NormalizedVersion::zero();
    let loader = manifest.project.modloader.clone();

    let mut root_deps = Vec::new();
    for (name, spec) in &manifest.dependencies {
        let constraint = match spec {
            crate::manifest::DependencySpec::Short(v) => {
                if v == "*" { pubgrub::range::Range::any() } else { pubgrub::range::Range::exact(NormalizedVersion::parse(v, &loader)) }
            }
            crate::manifest::DependencySpec::Full { version, .. } => {
                let v = version.as_deref().unwrap_or("*");
                if v == "*" { pubgrub::range::Range::any() } else { pubgrub::range::Range::exact(NormalizedVersion::parse(v, &loader)) }
            }
        };
        root_deps.push((name.clone(), constraint));
    }

    provider.add_package_versions(root_pkg.clone(), vec![root_version.clone()]);
    provider.add_package_deps(root_pkg.clone(), root_version.clone(), root_deps);

    let mc_version = manifest.project.mc_version.clone();

    loop {
        match resolve(&provider, root_pkg.clone(), root_version.clone()) {
            Ok(mut solution) => {
                solution.remove(&root_pkg);
                let mut resolved = HashMap::new();
                for (pkg, ver) in solution {
                    if let Some(rm) = provider.resolved_mods.get(&(pkg.clone(), ver)) {
                        resolved.insert(pkg, rm.clone());
                    }
                }
                return Ok(resolved);
            }
            Err(pubgrub::error::PubGrubError::ErrorChoosingPackageVersion(err)) |
            Err(pubgrub::error::PubGrubError::ErrorRetrievingDependencies { source: err, .. }) => {
                if let Some(fetch_err) = err.downcast_ref::<FetchRetryError>() {
                    let missing_pkg = match fetch_err {
                        FetchRetryError::MissingVersions(pkg) => pkg.clone(),
                        FetchRetryError::MissingDependencies(pkg, _) => pkg.clone(),
                    };
                    
                    let mut fetched = false;
                    for p in providers {
                        if let Ok(versions) = p.get_versions(&missing_pkg, Some(&mc_version), Some(&loader)).await {
                            if !versions.is_empty() {
                                let mut norm_versions = Vec::new();
                                for rm in &versions {
                                    let nv = NormalizedVersion::parse(&rm.version, &loader);
                                    norm_versions.push(nv.clone());
                                    
                                    let mut deps = Vec::new();
                                    for dep in &rm.dependencies {
                                        if !dep.required { continue; } // ignore optional for now
                                        let dep_pkg = dep.slug.clone().unwrap_or(dep.name.clone());
                                        deps.push((dep_pkg, pubgrub::range::Range::any()));
                                    }
                                    provider.add_package_deps(missing_pkg.clone(), nv.clone(), deps);
                                    provider.resolved_mods.insert((missing_pkg.clone(), nv.clone()), rm.clone());
                                }
                                norm_versions.sort_by(|a: &crate::resolver::version::NormalizedVersion, b: &crate::resolver::version::NormalizedVersion| b.cmp(a));
                                provider.add_package_versions(missing_pkg.clone(), norm_versions);
                                fetched = true;
                                break;
                            }
                        }
                    }
                    if !fetched {
                        provider.add_package_versions(missing_pkg.clone(), vec![]);
                    }
                    continue;
                } else {
                    return Err(format!("Resolution error: {}", err));
                }
            }
            Err(pubgrub::error::PubGrubError::NoSolution(derivation_tree)) => {
                use pubgrub::report::{DefaultStringReporter, Reporter};
                let report = DefaultStringReporter::report(&derivation_tree);
                return Err(report);
            }
            Err(e) => return Err(format!("Resolution error: {}", e)),
        }
    }
}

pub fn check_local_graph(
    manifest: &OrbitManifest,
    local_mods: &[crate::identification::IdentifiedMod],
) -> Result<(), String> {
    let mut provider = OrbitDependencyProvider::new();
    let loader = manifest.project.modloader.clone();
    
    let mc_ver = NormalizedVersion::parse(&manifest.project.mc_version, &loader);
    provider.add_package_versions("minecraft".to_string(), vec![mc_ver.clone()]);
    provider.add_package_deps("minecraft".to_string(), mc_ver, vec![]);

    for m in local_mods {
        let pkg = if !m.mod_id.is_empty() { m.mod_id.clone() } else if !m.mod_name.is_empty() { m.mod_name.clone() } else { m.filename.clone() };
        let nv = NormalizedVersion::parse(&m.version, &loader);
        provider.add_package_versions(pkg.clone(), vec![nv.clone()]);
        
        let mut deps = Vec::new();
        for (dep_id, constraint, req) in &m.deps {
            if *req && dep_id != "java" && dep_id != "mixinextras" {
                let range = if constraint.is_empty() || constraint == "*" {
                    pubgrub::range::Range::any()
                } else {
                    pubgrub::range::Range::exact(NormalizedVersion::parse(constraint, &loader))
                };
                deps.push((dep_id.clone(), range));
            }
        }
        provider.add_package_deps(pkg, nv, deps);
    }

    let root_pkg = "___orbit_root___".to_string();
    let root_version = NormalizedVersion::zero();
    let mut root_deps = Vec::new();
    for (name, spec) in &manifest.dependencies {
        let constraint = match spec {
            crate::manifest::DependencySpec::Short(v) => {
                if v == "*" { pubgrub::range::Range::any() } else { pubgrub::range::Range::exact(NormalizedVersion::parse(v, &loader)) }
            }
            crate::manifest::DependencySpec::Full { version, .. } => {
                let v = version.as_deref().unwrap_or("*");
                if v == "*" { pubgrub::range::Range::any() } else { pubgrub::range::Range::exact(NormalizedVersion::parse(v, &loader)) }
            }
        };
        root_deps.push((name.clone(), constraint));
    }
    provider.add_package_versions(root_pkg.clone(), vec![root_version.clone()]);
    provider.add_package_deps(root_pkg.clone(), root_version.clone(), root_deps);

    match pubgrub::solver::resolve(&provider, root_pkg, root_version) {
        Ok(_) => Ok(()),
        Err(pubgrub::error::PubGrubError::NoSolution(tree)) => {
            use pubgrub::report::{DefaultStringReporter, Reporter};
            Err(DefaultStringReporter::report(&tree))
        }
        Err(pubgrub::error::PubGrubError::ErrorChoosingPackageVersion(err)) |
        Err(pubgrub::error::PubGrubError::ErrorRetrievingDependencies { source: err, .. }) => {
            if let Some(fetch_err) = err.downcast_ref::<FetchRetryError>() {
                match fetch_err {
                    FetchRetryError::MissingVersions(pkg) => Err(format!("Missing dependency package: {}", pkg)),
                    FetchRetryError::MissingDependencies(pkg, ver) => Err(format!("Missing dependency data for: {} {}", pkg, ver)),
                }
            } else {
                Err(err.to_string())
            }
        }
        Err(e) => Err(e.to_string()),
    }
}

pub fn dependents<'a>(slug: &str, entries: &'a [LockEntry]) -> Vec<&'a str> {
    entries
        .iter()
        .filter(|e| e.dependencies.iter().any(|d| d.name == slug))
        .map(|e| e.name.as_str())
        .collect()
}

pub fn find_entry<'a>(slug: &str, entries: &'a [LockEntry]) -> Option<&'a LockEntry> {
    entries.iter().find(|e| {
        e.name == slug 
        || e.mod_id.as_deref() == Some(slug) 
        || e.slug.as_deref() == Some(slug)
    })
}

pub fn check_version_conflict(slug: &str, new_version: &str, entries: &[LockEntry]) -> Result<(), String> {
    if let Some(entry) = find_entry(slug, entries) {
        if entry.version != new_version {
            return Err(format!(
                "'{}' version conflict: lock has '{}', resolved '{}'",
                entry.name, entry.version, new_version
            ));
        }
    }
    Ok(())
}
