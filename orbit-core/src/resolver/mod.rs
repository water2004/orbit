pub mod types;
pub mod provider;
pub mod provider_version;
pub mod modrinth_version;

use std::collections::HashMap;

use crate::lockfile::PackageEntry;

use self::types::PackageId;
use self::provider_version::{ProviderVersionResolver, FallbackResolver};
use self::modrinth_version::ModrinthVersionResolver;
use crate::versions::Version;
use crate::resolver::provider::OrbitDependencyProvider;
use pubgrub::solver::resolve;

#[derive(Debug)]
pub enum FetchRetryError {
    MissingVersions(PackageId),
    MissingDependencies(PackageId, Version),
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
/// - `Ok(HashMap<PackageId, Version>)` — 每个包被选中的版本
/// - `Err(String)` — 人类可读的冲突报告
/// 求解依赖图，带 Fetch-and-Retry 懒加载。
pub async fn resolve_manifest(
    manifest: &OrbitManifest,
    lockfile: &crate::lockfile::OrbitLockfile,
    providers: &[Box<dyn ModProvider>],
) -> Result<HashMap<PackageId, crate::providers::ResolvedMod>, String> {
    let mut provider = OrbitDependencyProvider::new();
    let root_pkg = "___orbit_root___".to_string();
    let root_version = Version::zero();
    let loader = manifest.project.modloader.clone();

    // 注入 lockfile 中已有条目作为本地可用版本。
    // 这些 mod 已安装，依赖已满足——只注册为"可用版本"不携带它们的依赖，
    // 这样 PubGrub 只需解析新添加的包的依赖，不会因已安装 mod 的内部依赖链报错。
    for entry in &lockfile.packages {
        let ver = Version::parse(&entry.version, &loader);
        provider.add_package_versions(entry.mod_id.clone(), vec![ver.clone()]);
        provider.add_package_deps(entry.mod_id.clone(), ver, vec![]);
        for imp in &entry.implanted {
            let iver = Version::parse(&imp.version, &loader);
            provider.add_package_versions(imp.name.clone(), vec![iver.clone()]);
            provider.add_package_deps(imp.name.clone(), iver, vec![]);
        }
    }

    let mut root_deps = Vec::new();
    for (name, spec) in &manifest.dependencies {
        let constraint = match spec {
            crate::manifest::DependencySpec::Short(v) => {
                Version::parse_constraint(v, &loader)
            }
            crate::manifest::DependencySpec::Full { version, .. } => {
                let v = version.as_deref().unwrap_or("*");
                Version::parse_constraint(v, &loader)
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
                        if let Ok(mut versions) = p.get_versions(&missing_pkg, Some(&mc_version), Some(&loader)).await {
                            if !versions.is_empty() {
                                // 使用 provider 特定的版本排序（Modrinth → date_published，fallback → SemVer）
                                let pvr: &dyn ProviderVersionResolver = if p.name() == "modrinth" {
                                    &ModrinthVersionResolver
                                } else {
                                    &FallbackResolver
                                };
                                pvr.sort_newest_first(&mut versions);

                                let mut norm_versions = Vec::new();
                                for rm in &versions {
                                    let nv = Version::parse(&rm.version, &loader);
                                    norm_versions.push(nv.clone());

                                    let mut deps = Vec::new();
                                    for dep in &rm.dependencies {
                                        if !dep.required { continue; }
                                        let Some(ref dep_pkg) = dep.slug else { continue; };
                                        if dep_pkg.is_empty() { continue; }
                                        deps.push((dep_pkg.clone(), pubgrub::range::Range::any()));
                                    }
                                    provider.add_package_deps(missing_pkg.clone(), nv.clone(), deps);
                                    provider.resolved_mods.insert((missing_pkg.clone(), nv.clone()), rm.clone());
                                }
                                // 按 provider 顺序注入 PubGrub（用于 choose_package_version 选第一个）
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
    
    let mc_ver = Version::parse(&manifest.project.mc_version, &loader);
    provider.add_package_versions("minecraft".to_string(), vec![mc_ver.clone()]);
    provider.add_package_deps("minecraft".to_string(), mc_ver, vec![]);

    let loader_pkg = if loader == "fabric" {
        "fabricloader".to_string()
    } else if loader == "quilt" {
        "quiltloader".to_string()
    } else {
        loader.clone()
    };
    let loader_ver = Version::parse(&manifest.project.modloader_version, &loader);
    provider.add_package_versions(loader_pkg.clone(), vec![loader_ver.clone()]);
    provider.add_package_deps(loader_pkg, loader_ver, vec![]);

    // key: pkg id → local_version（用于 root_deps 的 exact 约束）
    let mut pkg_local_versions: std::collections::HashMap<String, Version> = Default::default();

    for m in local_mods {
        let pkg = if !m.mod_id.is_empty() { m.mod_id.clone() } else if !m.mod_name.is_empty() { m.mod_name.clone() } else { m.filename.clone() };
        // 使用 fabric.mod.json 里的自声明版本，与 Fabric Loader 行为一致
        let nv = Version::parse(&m.version, &loader);
        provider.add_package_versions(pkg.clone(), vec![nv.clone()]);
        pkg_local_versions.insert(pkg.clone(), nv.clone());

        let mut deps = Vec::new();
        for (dep_id, constraint, req) in &m.deps {
            if *req && dep_id != "java" && dep_id != "mixinextras" {
                deps.push((dep_id.clone(), Version::parse_constraint(constraint, &loader)));
            }
        }
        provider.add_package_deps(pkg, nv, deps);
    }

    // 收集所有被依赖但未安装的包，给它们注册一个空版本列表。
    // 这样 PubGrub 在报错时就能给出完整的依赖链，而不是直接报错包丢失。
    let mut missing_deps = std::collections::HashSet::new();
    for m in local_mods {
        for (dep_id, _, req) in &m.deps {
            if *req && !provider.versions.contains_key(dep_id) && dep_id != "java" && dep_id != "mixinextras" {
                missing_deps.insert(dep_id.clone());
            }
        }
    }
    for (name, _) in &manifest.dependencies {
        if !provider.versions.contains_key(name) {
            missing_deps.insert(name.clone());
        }
    }
    for dep in missing_deps {
        provider.add_package_versions(dep, vec![]);
    }

    let root_pkg = "___orbit_root___".to_string();
    let root_version = Version::zero();
    let mut root_deps = Vec::new();
    for name in manifest.dependencies.keys() {
        // root 依赖使用本地已安装的 local_version 做 exact 约束
        // 这样 root 和 mod 自己声明的版本永远一致，不受 Modrinth 版本名影响
        if let Some(installed_ver) = pkg_local_versions.get(name) {
            root_deps.push((name.clone(), pubgrub::range::Range::exact(installed_ver.clone())));
        }
        // 如果本地没有安装，missing_deps 已经把它注册为空版本列表，PubGrub 会自动报错
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
            Err(format!("Internal resolver error: {}", err))
        }
        Err(e) => Err(e.to_string()),
    }
}

pub fn dependents<'a>(slug: &str, entries: &'a [PackageEntry]) -> Vec<&'a str> {
    entries
        .iter()
        .filter(|e| e.dependencies.iter().any(|d| d.name == slug))
        .map(|e| e.mod_id.as_str())
        .collect()
}

pub fn find_entry<'a>(slug: &str, entries: &'a [PackageEntry]) -> Option<&'a PackageEntry> {
    entries.iter().find(|e| {
        e.mod_id == slug
        || e.modrinth.as_ref().map(|m| m.slug.as_str()) == Some(slug)
    })
}

pub fn check_version_conflict(slug: &str, new_version: &str, entries: &[PackageEntry]) -> Result<(), String> {
    if let Some(entry) = find_entry(slug, entries) {
        if entry.version != new_version {
            return Err(format!(
                "'{}' version conflict: lock has '{}', resolved '{}'",
                entry.mod_id, entry.version, new_version
            ));
        }
    }
    Ok(())
}
