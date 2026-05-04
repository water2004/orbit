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

    inject_lockfile(&mut provider, lockfile, &loader);

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
    provider.add_package_deps(loader_pkg.to_string(), loader_ver, vec![]);

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

/// 将 lockfile 条目注入 PubGrub provider。
/// 条目不携带依赖（已安装的 mod 视为已满足），仅标记版本存在。
fn inject_lockfile(
    provider: &mut OrbitDependencyProvider,
    lockfile: &crate::lockfile::OrbitLockfile,
    loader: &str,
) {
    for entry in &lockfile.packages {
        let ver = Version::parse(&entry.version, loader);
        provider.add_package_versions(entry.mod_id.clone(), vec![ver.clone()]);
        provider.add_package_deps(entry.mod_id.clone(), ver, vec![]);
        for imp in &entry.implanted {
            let iver = Version::parse(&imp.version, loader);
            provider.add_package_versions(imp.name.clone(), vec![iver.clone()]);
            provider.add_package_deps(imp.name.clone(), iver, vec![]);
        }
    }
}

/// 离线 PubGrub 求解：给定候选版本（已从 JAR 解析出真实版本号和依赖约束），
/// 判断哪些 mod 可安全升级。返回 `mod_id → new_version`。
///
/// `candidates`: mod_id → [(jar_version, deps)]，每个元素对应一个候选版本（从新到旧排序）
pub fn resolve_with_candidates(
    manifest: &OrbitManifest,
    lockfile: &crate::lockfile::OrbitLockfile,
    candidates: &HashMap<String, Vec<(String, Vec<(String, String, bool)>)>>,
) -> Result<HashMap<String, String>, String> {
    let loader = &manifest.project.modloader;

    let mut provider = OrbitDependencyProvider::new();

    // 注入 minecraft 和 loader 作为内置包
    let mc_ver = Version::parse(&manifest.project.mc_version, loader);
    provider.add_package_versions("minecraft".to_string(), vec![mc_ver.clone()]);
    provider.add_package_deps("minecraft".to_string(), mc_ver, vec![]);

    let loader_pkg = if loader == "fabric" { "fabricloader" }
        else if loader == "quilt" { "quiltloader" }
        else { loader };
    let loader_ver = Version::parse(&manifest.project.modloader_version, loader);
    provider.add_package_versions(loader_pkg.to_string(), vec![loader_ver.clone()]);
    provider.add_package_deps(loader_pkg.to_string(), loader_ver, vec![]);

    let zero = Version::zero();
    provider.add_package_versions("java".to_string(), vec![zero.clone()]);
    provider.add_package_deps("java".to_string(), zero.clone(), vec![]);
    if loader == "fabric" {
        provider.add_package_versions("mixinextras".to_string(), vec![zero.clone()]);
        provider.add_package_deps("mixinextras".to_string(), zero.clone(), vec![]);
    }

    // 注入 lockfile 所有条目（含实际依赖）
    for entry in &lockfile.packages {
        let ver = Version::parse(&entry.version, loader);
        let deps: Vec<_> = entry.dependencies.iter().map(|d| {
            (d.name.clone(), Version::parse_constraint(&d.version, loader))
        }).collect();
        provider.add_package_versions(entry.mod_id.clone(), vec![ver.clone()]);
        provider.add_package_deps(entry.mod_id.clone(), ver, deps);
        for imp in &entry.implanted {
            let iver = Version::parse(&imp.version, loader);
            let ideps: Vec<_> = imp.dependencies.iter().map(|d| {
                (d.name.clone(), Version::parse_constraint(&d.version, loader))
            }).collect();
            provider.add_package_versions(imp.name.clone(), vec![iver.clone()]);
            provider.add_package_deps(imp.name.clone(), iver, ideps);
        }
    }

    // 追加候选版本到已有列表（不替换），PubGrub 按序优先选取
    for (mod_id, versions) in candidates {
        let mut all_vers: Vec<Version> = provider.versions.get(mod_id.as_str())
            .cloned()
            .unwrap_or_default();
        for (jar_ver, deps) in versions {
            let v = Version::parse(jar_ver, loader);
            let d: Vec<_> = deps.iter()
                .filter(|(_, _, req)| *req)
                .map(|(name, constraint, _)| (name.clone(), Version::parse_constraint(constraint, loader)))
                .collect();
            all_vers.push(v.clone());
            provider.add_package_deps(mod_id.clone(), v, d);
        }
        provider.add_package_versions(mod_id.clone(), all_vers);
    }

    // 收集所有被依赖但不在 provider 中的包，注册空版本（使 PubGrub 报清晰错误）
    let mut referenced: std::collections::HashSet<String> = std::collections::HashSet::new();
    for entry in &lockfile.packages {
        for d in &entry.dependencies { referenced.insert(d.name.clone()); }
        for imp in &entry.implanted {
            for d in &imp.dependencies { referenced.insert(d.name.clone()); }
        }
    }
    for (_, versions) in candidates.iter() {
        for (_, deps) in versions {
            for (name, _, req) in deps { if *req { referenced.insert(name.clone()); } }
        }
    }
    for dep in referenced {
        if !provider.versions.contains_key(&dep) {
            provider.add_package_versions(dep, vec![]);
        }
    }

    // Root deps — 有候选版本的 mod 放宽为 any，让 PubGrub 自由选择
    let root_pkg = "___orbit_root___".to_string();
    let root_version = Version::zero();
    let mut root_deps = Vec::new();
    for (name, _spec) in &manifest.dependencies {
        let constraint = if candidates.contains_key(name) {
            pubgrub::range::Range::any()
        } else {
            match _spec {
                crate::manifest::DependencySpec::Short(v) => Version::parse_constraint(v, loader),
                crate::manifest::DependencySpec::Full { version, .. } => {
                    let v = version.as_deref().unwrap_or("*");
                    Version::parse_constraint(v, loader)
                }
            }
        };
        root_deps.push((name.clone(), constraint));
    }
    provider.add_package_versions(root_pkg.clone(), vec![root_version.clone()]);
    provider.add_package_deps(root_pkg.clone(), root_version.clone(), root_deps);

    let solution = match pubgrub::solver::resolve(&provider, root_pkg, root_version) {
        Ok(s) => s,
        Err(pubgrub::error::PubGrubError::NoSolution(tree)) => {
            use pubgrub::report::{DefaultStringReporter, Reporter};
            return Err(DefaultStringReporter::report(&tree));
        }
        Err(pubgrub::error::PubGrubError::ErrorChoosingPackageVersion(err)) => {
            if let Some(fetch) = err.downcast_ref::<FetchRetryError>() {
                return Err(format!("internal error: package '{}' is missing from resolver", fetch));
            }
            return Err(format!("internal error: {}", err));
        }
        Err(pubgrub::error::PubGrubError::ErrorRetrievingDependencies { package, version, source }) => {
            if let Some(fetch) = source.downcast_ref::<FetchRetryError>() {
                return Err(format!("internal error: deps of '{}' v{} are missing: {}", package, version, fetch));
            }
            return Err(format!("internal error: {} v{} deps: {}", package, version, source));
        }
        Err(e) => return Err(e.to_string()),
    };

    // 对比 lockfile，找出升级
    let mut upgrades = HashMap::new();
    for (mod_id, _) in candidates {
        if let Some(ver) = solution.get(mod_id.as_str()) {
            let entry = lockfile.find(mod_id);
            let current = entry.map(|e| e.version.as_str()).unwrap_or("?");
            let new_ver = ver.to_string();
            if new_ver != current {
                upgrades.insert(mod_id.clone(), new_ver);
            }
        }
    }

    Ok(upgrades)
}
