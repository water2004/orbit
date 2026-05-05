pub mod types;
pub mod provider;

use std::collections::HashMap;

use crate::lockfile::PackageEntry;
use crate::manifest::OrbitManifest;
use crate::providers::ModProvider;

use self::types::PackageId;
use crate::versions::Version;
use crate::resolver::provider::OrbitDependencyProvider;

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

/// 带 Fetch-and-Retry 的离线求解。
/// 缺依赖时上 provider 下载 JAR 加入候选，重试直到求解成功或无更多候选。
///
/// `candidates`: mod_id → [CandidateVersion]（会被追加修改）
pub async fn resolve_with_candidates(
    manifest: &OrbitManifest,
    lockfile: &crate::lockfile::OrbitLockfile,
    candidates: &mut HashMap<String, Vec<crate::resolver::types::CandidateVersion>>,
    providers: &[Box<dyn ModProvider>],
) -> Result<HashMap<String, String>, String> {
    let loader = &manifest.project.modloader;
    let mc_version = &manifest.project.mc_version;

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

    // 候选版本放前面（已从新到旧），lockfile 版本垫底，PubGrub 优先选新版本
    for (mod_id, versions) in &*candidates {
        let existing = provider.versions.get(mod_id.as_str()).cloned().unwrap_or_default();
        let mut all_vers = Vec::new();
        for cand in versions {
            let v = Version::parse(&cand.jar_version, loader);
            let mut d: Vec<_> = cand.deps.iter()
                .filter(|(_, _, req)| *req)
                .map(|(name, constraint, _)| (name.clone(), Version::parse_constraint(constraint, loader)))
                .collect();
            
            // Register implanted mod versions (constraints already in cand.deps)
            for imp in &cand.implanted {
                let imp_ver = Version::parse(&imp.version, loader);

                // Register implanted mod in provider
                let imp_d: Vec<_> = imp.deps.iter()
                    .filter(|(_, _, req)| *req)
                    .map(|(name, constraint, _)| (name.clone(), Version::parse_constraint(constraint, loader)))
                    .collect();
                
                let mut imp_existing = provider.versions.get(&imp.mod_id).cloned().unwrap_or_default();
                if !imp_existing.contains(&imp_ver) {
                    imp_existing.push(imp_ver.clone());
                    provider.add_package_versions(imp.mod_id.clone(), imp_existing);
                }
                provider.add_package_deps(imp.mod_id.clone(), imp_ver, imp_d);
            }
            
            all_vers.push(v.clone());
            provider.add_package_deps(mod_id.clone(), v, d);
        }
        all_vers.extend(existing);
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
        for cand in versions {
            for (name, _, req) in &cand.deps { if *req { referenced.insert(name.clone()); } }
            for imp in &cand.implanted {
                for (name, _, req) in &imp.deps { if *req { referenced.insert(name.clone()); } }
            }
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
    // 候选 mod 不在 manifest 中的也加入 root deps（如 `orbit add` 的新 mod）
    for mod_id in candidates.keys() {
        if !manifest.dependencies.contains_key(mod_id) {
            root_deps.push((mod_id.clone(), pubgrub::range::Range::any()));
        }
    }
    provider.add_package_versions(root_pkg.clone(), vec![root_version.clone()]);
    provider.add_package_deps(root_pkg.clone(), root_version.clone(), root_deps);

    let solution = loop {
        match pubgrub::solver::resolve(&provider, root_pkg.clone(), root_version.clone()) {
            Ok(s) => break s,
            Err(pubgrub::error::PubGrubError::NoSolution(_tree)) => {
                // 缺依赖 → 从 lockfile 找到 project_id → provider 下载 JAR → 加入 candidates 重试
                // 从已注册的包中找出版本列表为空的（即缺失的）
                // 收集候选版本引用的依赖，只下载这些依赖的 JAR
                let mut needed_deps: std::collections::HashSet<String> = std::collections::HashSet::new();
                for (_, versions) in candidates.iter() {
                    for cand in versions {
                        for (name, _, req) in &cand.deps {
                            if *req && name != "java" && name != "mixinextras"
                                && name != "minecraft" && name != "fabricloader"
                            {
                                needed_deps.insert(name.clone());
                            }
                        }
                        for imp in &cand.implanted {
                            for (name, _, req) in &imp.deps {
                                if *req && name != "java" && name != "mixinextras"
                                    && name != "minecraft" && name != "fabricloader"
                                {
                                    needed_deps.insert(name.clone());
                                }
                            }
                        }
                    }
                }
                eprintln!("    needed deps from candidates: {:?}", needed_deps.iter().collect::<Vec<_>>());
                let mut added = false;
                for dep in &needed_deps {
                    if candidates.contains_key(dep) { eprintln!("    {} already in candidates, skip", dep); continue; }
                    let entry = lockfile.find(dep);
                    if entry.is_none() { eprintln!("    {} not in lockfile, skip", dep); continue; }
                    let Some(mr) = entry.and_then(|e| e.modrinth.as_ref()) else { eprintln!("    {} no modrinth info, skip", dep); continue; };
                    eprintln!("    fetching dep {} versions (project={})...", dep, mr.project_id);
                    let versions = match providers[0].get_versions(&mr.project_id, Some(mc_version), Some(loader)).await {
                        Ok(v) => v,
                        Err(e) => { eprintln!("    ! API error for {}: {}", dep, e); continue; }
                    };
                    let mut new_candidates = Vec::new();
                    for v in &versions {
                        if let Ok(meta) = crate::jar::download_and_parse(&v.download_url, &v.sha512, loader).await {
                            let imp_cands = meta.implanted_mods.into_iter().map(|im| {
                                crate::resolver::types::ImplantedCandidate {
                                    mod_id: im.mod_id,
                                    version: im.version,
                                    deps: im.dependencies,
                                }
                            }).collect();
                            new_candidates.push(crate::resolver::types::CandidateVersion {
                                jar_version: meta.version,
                                deps: meta.dependencies,
                                implanted: imp_cands,
                            });
                        }
                    }
                    if new_candidates.is_empty() { continue; }
                    eprintln!("    downloaded {} versions for {}", new_candidates.len(), dep);
                    let existing = provider.versions.get(dep).cloned().unwrap_or_default();
                    let mut all_vers: Vec<Version> = new_candidates.iter()
                        .map(|cand| Version::parse(&cand.jar_version, loader))
                        .collect();
                    all_vers.extend(existing);
                    for cand in &new_candidates {
                        let v = Version::parse(&cand.jar_version, loader);
                        let mut d: Vec<_> = cand.deps.iter()
                            .filter(|(n, _, req)| *req && n != "java" && n != "mixinextras")
                            .map(|(n, c, _)| (n.clone(), Version::parse_constraint(c, loader)))
                            .collect();
                        
                        for imp in &cand.implanted {
                            let imp_ver = Version::parse(&imp.version, loader);

                            let imp_d: Vec<_> = imp.deps.iter()
                                .filter(|(n, _, req)| *req && n != "java" && n != "mixinextras")
                                .map(|(n, c, _)| (n.clone(), Version::parse_constraint(c, loader)))
                                .collect();

                            let mut imp_existing = provider.versions.get(&imp.mod_id).cloned().unwrap_or_default();
                            if !imp_existing.contains(&imp_ver) {
                                imp_existing.push(imp_ver.clone());
                                provider.add_package_versions(imp.mod_id.clone(), imp_existing);
                            }
                            provider.add_package_deps(imp.mod_id.clone(), imp_ver, imp_d);
                        }
                        
                        provider.add_package_deps(dep.clone(), v, d);
                    }
                    provider.add_package_versions(dep.clone(), all_vers);
                    candidates.entry(dep.clone()).or_default().extend(new_candidates);
                    added = true;
                }
                if !added {
                    use pubgrub::report::{DefaultStringReporter, Reporter};
                    return Err(DefaultStringReporter::report(&_tree));
                }
            }
            Err(pubgrub::error::PubGrubError::ErrorChoosingPackageVersion(err)) => {
                if let Some(fetch) = err.downcast_ref::<FetchRetryError>() {
                    return Err(format!("internal error: package '{}' is missing from resolver", fetch));
                }
                return Err(format!("internal error: {}", err));
            }
            Err(e) => return Err(e.to_string()),
        }
    };

    eprintln!("    solution: {:?}", solution.keys().collect::<Vec<_>>());
    let mut upgrades = HashMap::new();
    for (mod_id, _) in candidates {
        eprintln!("    check {}: in_solution={}", mod_id, solution.contains_key(mod_id.as_str()));
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
