//! 模组安装 / 卸载逻辑。
//!
//! 提供顶层 API 供 CLI 调用。CLI 层不直接操作 TOML / 文件。

use std::path::{Path, PathBuf};

use crate::error::OrbitError;
use crate::lockfile::{
    FileInfo, LockDependency, LockMeta, ModrinthInfo, OrbitLockfile, PackageEntry,
};
use crate::manifest::{DependencySpec, OrbitManifest};
use crate::providers::{ModProvider, ResolvedMod};
use crate::resolver::types::CandidateDiagnostic;
use crate::workspace::{Lockfile, ManifestFile};

pub type InstallPrompt = Box<dyn FnOnce(&InstallReport) -> bool + Send>;

#[derive(Debug, Clone, Default)]
pub struct InstallOptions {
    pub no_deps: bool,
    pub dry_run: bool,
    pub existing_ok: bool,
    pub optional: bool,
    pub env: Option<String>,
}

fn download_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(format!("orbit/{}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("failed to build download client")
}

/// 单次 install 报告
#[derive(Debug, Clone)]
pub struct InstallReport {
    pub installed: Vec<InstalledMod>,
    pub already_satisfied: Vec<String>,
    pub skipped_optional: Vec<String>,
    pub diagnostics: Vec<CandidateDiagnostic>,
}

#[derive(Debug, Clone, Default)]
pub struct RestoreOptions {
    pub target: Option<String>,
    pub group: Option<String>,
    pub no_optional: bool,
    pub locked: bool,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Default)]
pub struct RestoreReport {
    pub restored: Vec<String>,
    pub already_present: Vec<String>,
    pub skipped: Vec<String>,
    pub diagnostics: Vec<CandidateDiagnostic>,
}

#[derive(Debug, Clone)]
pub struct InstalledMod {
    pub slug: String,
    pub mod_id: String,
    /// fabric.mod.json 的 version
    pub version: String,
    pub filename: String,
    pub provider: String,
    pub project_id: String,
    pub version_id: String,
    /// Modrinth version_number（写入 [package.modrinth].version）
    pub modrinth_version: String,
    pub download_url: String,
    /// 从 JAR 提取的真实依赖: (mod_id, version_constraint, required)
    pub jar_deps: Vec<(String, String, bool)>,
    pub implanted: Vec<crate::lockfile::ImplantedMod>,
}

/// 顶层 API：在指定实例目录安装模组。
///
/// 接收 `instance_dir`，内部完成 orbit.toml / orbit.lock 的读写和 mods/ 目录管理。
/// `constraint` 会绑定到候选 JAR 自声明的 `mod_id`，并作为 manifest 根约束保留。
/// `dry_run` 为 true 时仅解析不下载不写文件。
pub async fn install_to_instance(
    slug: &str,
    constraint: &str,
    instance_dir: &Path,
    providers: &[Box<dyn ModProvider>],
    options: InstallOptions,
    prompt_fn: Option<InstallPrompt>,
) -> Result<InstallReport, OrbitError> {
    let dry_run = options.dry_run;
    let mut manifest_file = ManifestFile::open(instance_dir)?;
    let mut lock = Lockfile::open_or_default(
        instance_dir,
        LockMeta {
            mc_version: manifest_file.inner.project.mc_version.clone(),
            modloader: manifest_file.inner.project.modloader.clone(),
            modloader_version: manifest_file.inner.project.modloader_version.clone(),
        },
    );

    let mods_dir = instance_dir.join("mods");
    if !mods_dir.exists() && !dry_run {
        std::fs::create_dir_all(&mods_dir).map_err(OrbitError::Io)?;
    }

    let report = install_mod(InstallModInput {
        slug,
        constraint,
        providers,
        manifest: &mut manifest_file.inner,
        lockfile: &mut lock.inner,
        mods_dir: &mods_dir,
        options,
        prompt_fn,
    })
    .await?;

    if !dry_run && !report.installed.is_empty() {
        manifest_file.save()?;
        lock.save()?;
    }

    Ok(report)
}

/// Restore the selected environment from `orbit.toml` and `orbit.lock`.
///
/// Non-locked mode resolves a complete fat lockfile when manifest entries are
/// missing from the lock. Locked mode never performs metadata resolution.
pub async fn restore_instance(
    instance_dir: &Path,
    providers: &[Box<dyn ModProvider>],
    options: RestoreOptions,
) -> Result<RestoreReport, OrbitError> {
    validate_restore_options(&options)?;
    let manifest = ManifestFile::open(instance_dir)?;
    let lock_path = instance_dir.join("orbit.lock");
    if options.locked && !lock_path.exists() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "--locked requires orbit.lock, but it does not exist"
        )));
    }
    let mut lock = Lockfile::open_or_default(
        instance_dir,
        LockMeta {
            mc_version: manifest.inner.project.mc_version.clone(),
            modloader: manifest.inner.project.modloader.clone(),
            modloader_version: manifest.inner.project.modloader_version.clone(),
        },
    );
    reconcile_lock_metadata(&manifest.inner, &mut lock.inner, options.locked)?;

    let mut report = RestoreReport::default();
    let missing_roots: Vec<_> = manifest
        .inner
        .dependencies
        .keys()
        .filter(|package| lock.inner.find(package).is_none())
        .cloned()
        .collect();
    let has_dangling_lock_edges = lock.inner.packages.iter().any(|entry| {
        entry
            .dependencies
            .iter()
            .any(|dependency| lock.inner.find(&dependency.name).is_none())
    });
    let lock_graph_error =
        crate::resolver::check_lockfile_graph(&manifest.inner, &lock.inner).err();
    if !missing_roots.is_empty() || has_dangling_lock_edges || lock_graph_error.is_some() {
        if options.locked {
            let package = missing_roots
                .first()
                .cloned()
                .unwrap_or_else(|| "transitive dependency".to_string());
            return Err(OrbitError::Other(anyhow::anyhow!(
                "--locked: orbit.toml and orbit.lock are inconsistent near '{package}': {}",
                lock_graph_error.unwrap_or_else(|| "missing lock entry".to_string())
            )));
        }
        if options.dry_run {
            if missing_roots.is_empty() {
                report
                    .restored
                    .extend(manifest.inner.dependencies.keys().cloned());
            } else {
                report.restored.extend(missing_roots);
            }
            report.restored.sort();
            return Ok(report);
        }
        let resolution =
            resolve_missing_lock_entries(&manifest.inner, &mut lock.inner, providers).await?;
        report.diagnostics = resolution.diagnostics;
        lock.save()?;
    }

    let (selected, skipped) = selected_packages(&manifest.inner, &lock.inner, &options)?;
    report.skipped = skipped;
    let mods_dir = instance_dir.join("mods");
    if !options.dry_run {
        std::fs::create_dir_all(&mods_dir)?;
    }

    let mut lock_changed = false;
    for package in selected {
        let Some(index) = lock
            .inner
            .packages
            .iter()
            .position(|entry| entry.mod_id == package)
        else {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "orbit.lock is missing selected package '{package}'"
            )));
        };
        if package_is_present(&lock.inner.packages[index], &mods_dir)? {
            report.already_present.push(package);
            continue;
        }
        if options.dry_run {
            report.restored.push(package);
            continue;
        }
        restore_package(
            &mut lock.inner.packages[index],
            instance_dir,
            &mods_dir,
            providers,
            options.locked,
        )
        .await?;
        lock_changed = true;
        report.restored.push(package);
    }
    if lock_changed {
        lock.save()?;
    }
    report.restored.sort();
    report.already_present.sort();
    report.skipped.sort();
    Ok(report)
}

/// 升级实例中所有过期模组
pub async fn upgrade_all_in_instance(
    instance_dir: &Path,
    providers: &[Box<dyn ModProvider>],
    dry_run: bool,
    prompt_fn: Option<InstallPrompt>,
) -> Result<InstallReport, OrbitError> {
    let manifest_file = ManifestFile::open(instance_dir)?;
    let mut lock = Lockfile::open_or_default(
        instance_dir,
        LockMeta {
            mc_version: manifest_file.inner.project.mc_version.clone(),
            modloader: manifest_file.inner.project.modloader.clone(),
            modloader_version: manifest_file.inner.project.modloader_version.clone(),
        },
    );

    let crate::outdated::OutdatedReport {
        updates: outdated,
        resolved: resolved_candidates,
        diagnostics,
    } = crate::outdated::check_all_outdated(&manifest_file.inner, &lock.inner, providers).await?;

    if outdated.is_empty() {
        return Ok(InstallReport {
            installed: vec![],
            already_satisfied: vec![],
            skipped_optional: vec![],
            diagnostics,
        });
    }

    let mods_dir = instance_dir.join("mods");
    if !mods_dir.exists() && !dry_run {
        std::fs::create_dir_all(&mods_dir).map_err(OrbitError::Io)?;
    }

    let loader = &manifest_file.inner.project.modloader;
    let mut planned = Vec::new();

    for o in &outdated {
        let key = (o.mod_id.clone(), o.new_version.clone());
        let Some(resolved) = resolved_candidates.get(&key) else {
            continue;
        };
        planned.push(plan_from_resolved(&o.mod_id, &o.new_version, resolved));
    }

    let report = InstallReport {
        installed: planned.clone(),
        already_satisfied: vec![],
        skipped_optional: vec![],
        diagnostics: diagnostics.clone(),
    };

    if let Some(prompt) = prompt_fn
        && !prompt(&report)
    {
        return Ok(InstallReport {
            installed: vec![],
            already_satisfied: vec![],
            skipped_optional: vec![],
            diagnostics,
        }); // aborted
    }

    if dry_run {
        return Ok(report);
    }

    let installed = materialize_plans(planned, &resolved_candidates, &mods_dir, loader).await?;

    apply_to_lockfile(&mut lock.inner, &installed, &mods_dir);

    if !installed.is_empty() {
        manifest_file.save()?;
        lock.save()?;
    }

    Ok(InstallReport {
        installed,
        already_satisfied: vec![],
        skipped_optional: vec![],
        diagnostics,
    })
}

/// 顶层 API：从指定实例目录移除模组。
///
/// `input` 可以是 mod_id（JAR 内 fabric.mod.json 的 `id`）或 slug。
/// 先从 lockfile 查找（同时匹配 mod_id 和 modrinth.slug），
/// 再同步更新 manifest、lockfile、JAR 文件。
pub fn remove_from_instance(
    input: &str,
    instance_dir: &Path,
    dry_run: bool,
) -> Result<RemoveReport, OrbitError> {
    let mut manifest_file = ManifestFile::open(instance_dir)?;
    let mut lock = Lockfile::open(instance_dir)?;

    let entry = crate::resolver::find_entry(input, &lock.inner.packages)
        .ok_or_else(|| OrbitError::ModNotFound(input.to_string()))?;
    let key = entry.mod_id.clone();

    if !manifest_file.inner.dependencies.contains_key(&key) {
        return Err(OrbitError::ModNotFound(input.to_string()));
    }
    manifest_file
        .inner
        .dependencies
        .swap_remove(&key)
        .expect("dependency entry should exist");

    let dependents = crate::resolver::dependents(&key, &lock.inner.packages);
    if !dependents.is_empty() {
        return Err(OrbitError::Conflict(format!(
            "'{key}' is required by: {}\nRemove those mods first.",
            dependents.join(", ")
        )));
    }

    let mods_dir = instance_dir.join("mods");
    let jar_deleted = !dry_run && lock.remove_jar(&key, &mods_dir).is_ok();
    lock.inner.packages.retain(|e| e.mod_id != key);

    if !dry_run {
        manifest_file.save()?;
        lock.save()?;
    }
    Ok(RemoveReport {
        mod_id: key,
        jar_deleted,
    })
}

#[derive(Debug, Clone)]
pub struct RemoveReport {
    pub mod_id: String,
    pub jar_deleted: bool,
}

/// 列出实例中所有依赖（供 remove 找不到时交互选择）
/// 返回 (mod_id, slug)，slug 从 lockfile 的 [package.modrinth] 读取，
/// 若 lockfile 不存在或无 modrinth 信息则回退到 mod_id。
pub fn list_dependencies(instance_dir: &Path) -> Result<Vec<(String, String)>, OrbitError> {
    let manifest_file = ManifestFile::open(instance_dir)?;
    let lock = Lockfile::open(instance_dir).ok();
    Ok(manifest_file
        .inner
        .dependencies
        .iter()
        .map(|(k, _)| {
            let slug = lock
                .as_ref()
                .and_then(|l| l.find(k))
                .and_then(|e| e.modrinth.as_ref())
                .map(|m| m.slug.clone())
                .unwrap_or_else(|| k.clone());
            (k.clone(), slug)
        })
        .collect())
}

/// `orbit list` 输出结构
#[derive(Debug, Clone)]
pub struct ListOutput {
    pub packages: Vec<ListedPackage>,
}

#[derive(Debug, Clone)]
pub struct ListedPackage {
    pub mod_id: String,
    pub version: String,
    pub slug: Option<String>,
    pub provider: String,
    /// 依赖的 mod_id 列表
    pub dependencies: Vec<String>,
    /// 内嵌子模组 (name, version)
    pub implanted: Vec<(String, String)>,
}

/// 读取 lockfile 中所有已安装模组供 list 命令展示。
pub fn list_installed(instance_dir: &Path) -> Result<ListOutput, OrbitError> {
    let lock = Lockfile::open(instance_dir)?;
    let packages: Vec<ListedPackage> = lock
        .inner
        .packages
        .iter()
        .map(|e| ListedPackage {
            mod_id: e.mod_id.clone(),
            version: e.version.clone(),
            slug: e.modrinth.as_ref().map(|m| m.slug.clone()),
            provider: e.provider.clone(),
            dependencies: e.dependencies.iter().map(|d| d.name.clone()).collect(),
            implanted: e
                .implanted
                .iter()
                .map(|i| (i.name.clone(), i.version.clone()))
                .collect(),
        })
        .collect();
    Ok(ListOutput { packages })
}

// ── 内部实现 ──────────────────────────────────────────────────────────

struct InstallModInput<'a> {
    slug: &'a str,
    constraint: &'a str,
    providers: &'a [Box<dyn ModProvider>],
    manifest: &'a mut OrbitManifest,
    lockfile: &'a mut OrbitLockfile,
    mods_dir: &'a Path,
    options: InstallOptions,
    prompt_fn: Option<InstallPrompt>,
}

async fn install_mod(input: InstallModInput<'_>) -> Result<InstallReport, OrbitError> {
    let InstallModInput {
        slug,
        constraint,
        providers,
        manifest,
        lockfile,
        mods_dir,
        options,
        prompt_fn,
    } = input;

    if !options.existing_ok && crate::resolver::find_entry(slug, &lockfile.packages).is_some() {
        return Err(OrbitError::Conflict(format!(
            "'{slug}' already in lockfile. Use 'orbit upgrade {slug}' to update it."
        )));
    }

    let loader = &manifest.project.modloader;
    let mc_version = &manifest.project.mc_version;

    // 1-2. BFS download all JARs
    let seeds = vec![slug.to_string()];
    let crate::outdated::CandidateDownload {
        mut candidates,
        resolved,
        source_packages,
    } = crate::outdated::download_candidates_with_fallback(
        providers, &seeds, lockfile, mc_version, loader,
    )
    .await?;
    if candidates.is_empty() {
        return Err(OrbitError::ModNotFound(slug.to_string()));
    }
    let requested_package = source_packages
        .get(slug)
        .cloned()
        .or_else(|| {
            crate::resolver::find_entry(slug, &lockfile.packages)
                .map(|entry| entry.mod_id.clone())
                .filter(|package| candidates.contains_key(package))
        })
        .ok_or_else(|| OrbitError::ModNotFound(slug.to_string()))?;

    let mut resolution_manifest = manifest.clone();
    let requested_requirement =
        requested_requirement(constraint, options.optional, options.env.as_deref())?;
    ensure_root_requirement(
        &mut resolution_manifest,
        &requested_package,
        requested_requirement.clone(),
    );

    // 3. Resolve offline
    let resolution = match crate::resolver::resolve_with_candidates_report(
        &resolution_manifest,
        lockfile,
        &mut candidates,
        providers,
    )
    .await
    {
        Ok(resolution) => resolution,
        Err(e) => return Err(OrbitError::Conflict(e)),
    };
    let upgrades = resolution.upgrades;
    let diagnostics = resolution.diagnostics;

    // 4. Download resolved versions and apply
    let mut planned = Vec::new();
    let mut already_satisfied = Vec::new();

    for (mod_id, new_ver) in &upgrades {
        let key = (mod_id.clone(), new_ver.clone());
        let Some(resolved) = resolved.get(&key) else {
            continue;
        };

        if let Some(existing) = crate::resolver::find_entry(mod_id, &lockfile.packages)
            && existing.version == *new_ver
        {
            already_satisfied.push(mod_id.clone());
            continue;
        }
        if options.no_deps && mod_id != &requested_package {
            continue;
        }

        planned.push(plan_from_resolved(mod_id, new_ver, resolved));
    }

    let report = InstallReport {
        installed: planned.clone(),
        already_satisfied: already_satisfied.clone(),
        skipped_optional: vec![],
        diagnostics: diagnostics.clone(),
    };

    if let Some(prompt) = prompt_fn
        && !prompt(&report)
    {
        return Ok(InstallReport {
            installed: vec![],
            already_satisfied,
            skipped_optional: vec![],
            diagnostics,
        }); // aborted
    }

    if options.dry_run {
        return Ok(report);
    }

    for plan in &planned {
        // 升级时删旧 JAR
        if options.existing_ok
            && let Some(old) = lockfile.find(&plan.mod_id)
            && !old.filename.is_empty()
        {
            let _ = std::fs::remove_file(mods_dir.join(&old.filename));
        }
    }
    let installed = materialize_plans(planned, &resolved, mods_dir, loader).await?;

    if installed
        .iter()
        .any(|installed| installed.mod_id == requested_package)
    {
        ensure_root_requirement(manifest, &requested_package, requested_requirement);
    }
    apply_to_lockfile(lockfile, &installed, mods_dir);

    Ok(InstallReport {
        installed,
        already_satisfied,
        skipped_optional: vec![],
        diagnostics,
    })
}

// ── download / jar / manifest helpers ─────────────────────────────────

fn validate_restore_options(options: &RestoreOptions) -> Result<(), OrbitError> {
    if let Some(target) = options.target.as_deref()
        && !matches!(target, "client" | "server" | "both")
    {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "invalid target '{target}'; expected client, server, or both"
        )));
    }
    Ok(())
}

fn reconcile_lock_metadata(
    manifest: &OrbitManifest,
    lockfile: &mut OrbitLockfile,
    locked: bool,
) -> Result<(), OrbitError> {
    let matches = lockfile.meta.mc_version == manifest.project.mc_version
        && lockfile.meta.modloader == manifest.project.modloader
        && lockfile.meta.modloader_version == manifest.project.modloader_version;
    if matches {
        return Ok(());
    }
    if locked {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "--locked: orbit.lock metadata does not match orbit.toml"
        )));
    }
    lockfile.meta = LockMeta {
        mc_version: manifest.project.mc_version.clone(),
        modloader: manifest.project.modloader.clone(),
        modloader_version: manifest.project.modloader_version.clone(),
    };
    lockfile.packages.clear();
    Ok(())
}

async fn resolve_missing_lock_entries(
    manifest: &OrbitManifest,
    lockfile: &mut OrbitLockfile,
    providers: &[Box<dyn ModProvider>],
) -> Result<crate::resolver::types::ResolutionReport, OrbitError> {
    let seeds: Vec<_> = manifest.dependencies.keys().cloned().collect();
    if seeds.is_empty() {
        return Ok(crate::resolver::types::ResolutionReport::default());
    }
    let crate::outdated::CandidateDownload {
        mut candidates,
        resolved,
        ..
    } = crate::outdated::download_candidates_with_fallback(
        providers,
        &seeds,
        lockfile,
        &manifest.project.mc_version,
        &manifest.project.modloader,
    )
    .await?;

    let mut resolution_manifest = manifest.clone();
    for entry in &lockfile.packages {
        resolution_manifest
            .overrides
            .entry(entry.mod_id.clone())
            .or_insert_with(|| DependencySpec::Short(entry.version.clone()));
    }
    let resolution = crate::resolver::resolve_with_candidates_report(
        &resolution_manifest,
        lockfile,
        &mut candidates,
        providers,
    )
    .await
    .map_err(|error| OrbitError::Conflict(error.to_string()))?;
    for (package, version) in &resolution.upgrades {
        let Some(resolved) = resolved.get(&(package.clone(), version.clone())) else {
            continue;
        };
        let candidate = candidates.get(package).and_then(|versions| {
            versions
                .iter()
                .find(|candidate| candidate.jar_version == *version)
        });
        lockfile.packages.retain(|entry| entry.mod_id != *package);
        lockfile.packages.push(lock_entry_from_candidate(
            package, version, resolved, candidate,
        ));
    }
    lockfile
        .packages
        .sort_by(|left, right| left.mod_id.cmp(&right.mod_id));
    Ok(resolution)
}

fn lock_entry_from_candidate(
    package: &str,
    version: &str,
    resolved: &ResolvedMod,
    candidate: Option<&crate::resolver::types::CandidateVersion>,
) -> PackageEntry {
    let dependencies = candidate
        .map(|candidate| {
            candidate
                .deps
                .iter()
                .filter(|(name, _, required)| {
                    *required && !matches!(name.as_str(), "java" | "mixinextras")
                })
                .map(|(name, constraint, _)| LockDependency {
                    name: name.clone(),
                    version: if constraint.is_empty() {
                        "*".to_string()
                    } else {
                        constraint.clone()
                    },
                })
                .collect()
        })
        .unwrap_or_default();
    let implanted = candidate
        .map(|candidate| {
            candidate
                .implanted
                .iter()
                .map(|implanted| crate::lockfile::ImplantedMod {
                    name: implanted.mod_id.clone(),
                    version: implanted.version.clone(),
                    sha256: String::new(),
                    filename: String::new(),
                    dependencies: implanted
                        .deps
                        .iter()
                        .filter(|(name, _, required)| {
                            *required && !matches!(name.as_str(), "java" | "mixinextras")
                        })
                        .map(|(name, constraint, _)| LockDependency {
                            name: name.clone(),
                            version: constraint.clone(),
                        })
                        .collect(),
                })
                .collect()
        })
        .unwrap_or_default();
    PackageEntry {
        mod_id: package.to_string(),
        version: version.to_string(),
        sha1: resolved.sha1.clone(),
        sha256: String::new(),
        sha512: resolved.sha512.clone(),
        filename: resolved.filename.clone(),
        provider: resolved.provider.clone(),
        modrinth: resolved.modrinth.as_ref().map(|modrinth| ModrinthInfo {
            project_id: modrinth.project_id.clone(),
            version_id: modrinth.version_id.clone(),
            version: modrinth.version_number.clone(),
            slug: resolved.slug.clone(),
            download_url: resolved.download_url.clone(),
        }),
        file: None,
        dependencies,
        implanted,
    }
}

fn selected_packages(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    options: &RestoreOptions,
) -> Result<(Vec<String>, Vec<String>), OrbitError> {
    let group = options
        .group
        .as_ref()
        .map(|name| {
            manifest.groups.get(name).ok_or_else(|| {
                OrbitError::Other(anyhow::anyhow!(
                    "group '{name}' is not defined in orbit.toml"
                ))
            })
        })
        .transpose()?;
    let target = options.target.as_deref().unwrap_or("both");
    let mut roots = Vec::new();
    let mut skipped = Vec::new();
    for (package, spec) in &manifest.dependencies {
        let in_group = group.is_none_or(|group| {
            group
                .dependencies
                .iter()
                .any(|dependency| dependency == package)
        });
        let environment = spec.env().unwrap_or("both");
        let environment_matches = match target {
            "client" => matches!(environment, "client" | "both"),
            "server" => matches!(environment, "server" | "both"),
            _ => true,
        };
        if in_group && environment_matches && !(options.no_optional && spec.optional()) {
            roots.push(package.clone());
        } else {
            skipped.push(package.clone());
        }
    }

    let mut selected = std::collections::HashSet::new();
    let mut pending = roots;
    while let Some(package) = pending.pop() {
        if !selected.insert(package.clone()) {
            continue;
        }
        let entry = lockfile.find(&package).ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!(
                "orbit.lock is missing selected package '{package}'"
            ))
        })?;
        pending.extend(
            entry
                .dependencies
                .iter()
                .map(|dependency| dependency.name.clone()),
        );
    }
    let mut selected: Vec<_> = selected.into_iter().collect();
    selected.sort();
    skipped.sort();
    Ok((selected, skipped))
}

fn package_is_present(entry: &PackageEntry, mods_dir: &Path) -> Result<bool, OrbitError> {
    let filename = package_filename(entry);
    if filename.is_empty() {
        return Ok(false);
    }
    let path = mods_dir.join(filename);
    if !path.is_file() {
        return Ok(false);
    }
    if !entry.sha256.is_empty() {
        return Ok(crate::jar::compute_sha256(&path)? == entry.sha256);
    }
    if !entry.sha512.is_empty() {
        return Ok(crate::jar::compute_sha512(&path)? == entry.sha512);
    }
    Ok(std::fs::metadata(path)?.len() > 0)
}

async fn restore_package(
    entry: &mut PackageEntry,
    instance_dir: &Path,
    mods_dir: &Path,
    providers: &[Box<dyn ModProvider>],
    locked: bool,
) -> Result<(), OrbitError> {
    match entry.provider.as_str() {
        "file" => restore_file_package(entry, instance_dir, mods_dir),
        "modrinth" => restore_modrinth_package(entry, mods_dir, providers, locked).await,
        provider => Err(OrbitError::Other(anyhow::anyhow!(
            "unsupported lockfile provider '{provider}' for '{}'",
            entry.mod_id
        ))),
    }
}

fn restore_file_package(
    entry: &mut PackageEntry,
    instance_dir: &Path,
    mods_dir: &Path,
) -> Result<(), OrbitError> {
    let source = entry
        .file
        .as_ref()
        .map(|file| instance_dir.join(&file.path))
        .ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!(
                "file package '{}' has no source path",
                entry.mod_id
            ))
        })?;
    if !source.is_file() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "local source for '{}' is missing: {}",
            entry.mod_id,
            source.display()
        )));
    }
    let filename = source
        .file_name()
        .ok_or_else(|| OrbitError::Other(anyhow::anyhow!("invalid file package path")))?
        .to_string_lossy()
        .into_owned();
    let destination = mods_dir.join(&filename);
    if source != destination {
        std::fs::copy(&source, &destination)?;
    }
    verify_package_hash(entry, &destination)?;
    entry.filename = filename;
    entry.sha1 = crate::jar::compute_sha1(&destination)?;
    entry.sha256 = crate::jar::compute_sha256(&destination)?;
    entry.sha512 = crate::jar::compute_sha512(&destination)?;
    Ok(())
}

async fn restore_modrinth_package(
    entry: &mut PackageEntry,
    mods_dir: &Path,
    providers: &[Box<dyn ModProvider>],
    locked: bool,
) -> Result<(), OrbitError> {
    let metadata = entry.modrinth.clone().ok_or_else(|| {
        OrbitError::Other(anyhow::anyhow!(
            "Modrinth package '{}' has no Modrinth metadata",
            entry.mod_id
        ))
    })?;
    let filename = package_filename(entry);
    if !entry.sha512.is_empty() && !filename.is_empty() {
        let destination = mods_dir.join(&filename);
        if let Ok(cache) = crate::jar_cache::JarCache::load()
            && cache.copy_to(&entry.sha512, &destination)
        {
            verify_package_hash(entry, &destination)?;
            entry.filename = filename;
            return Ok(());
        }
    }

    let resolved = if metadata.download_url.is_empty() {
        if locked {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "--locked: '{}' has no download_url and is not available in cache",
                entry.mod_id
            )));
        }
        let provider = crate::providers::find_provider(providers, "modrinth").ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!(
                "Modrinth provider is required to restore '{}'",
                entry.mod_id
            ))
        })?;
        provider
            .get_versions(&metadata.project_id, None, None)
            .await?
            .into_iter()
            .find(|version| {
                version
                    .modrinth
                    .as_ref()
                    .is_some_and(|modrinth| modrinth.version_id == metadata.version_id)
            })
            .ok_or_else(|| {
                OrbitError::ModNotFound(format!("{} version {}", entry.mod_id, metadata.version_id))
            })?
    } else {
        ResolvedMod {
            mod_id: entry.mod_id.clone(),
            version: entry.version.clone(),
            sha1: entry.sha1.clone(),
            sha512: entry.sha512.clone(),
            slug: metadata.slug.clone(),
            provider: "modrinth".to_string(),
            modrinth: Some(crate::providers::ModrinthResolvedInfo {
                project_id: metadata.project_id.clone(),
                version_id: metadata.version_id.clone(),
                version_number: metadata.version.clone(),
            }),
            date_published: String::new(),
            download_url: metadata.download_url.clone(),
            filename: filename.clone(),
            dependencies: Vec::new(),
            client_side: None,
            server_side: None,
        }
    };
    let destination = download_mod(&resolved, mods_dir).await?;
    verify_package_hash(entry, &destination)?;
    entry.filename = resolved.filename.clone();
    entry.sha1 = crate::jar::compute_sha1(&destination)?;
    entry.sha256 = crate::jar::compute_sha256(&destination)?;
    entry.sha512 = crate::jar::compute_sha512(&destination)?;
    if let Some(modrinth) = &mut entry.modrinth {
        modrinth.download_url = resolved.download_url;
    }
    Ok(())
}

fn verify_package_hash(entry: &PackageEntry, path: &Path) -> Result<(), OrbitError> {
    if entry.sha256.is_empty() {
        return Ok(());
    }
    let actual = crate::jar::compute_sha256(path)?;
    if actual != entry.sha256 {
        return Err(OrbitError::ChecksumMismatch {
            name: entry.mod_id.clone(),
            expected: entry.sha256.clone(),
            actual,
        });
    }
    Ok(())
}

fn package_filename(entry: &PackageEntry) -> String {
    if !entry.filename.is_empty() {
        return entry.filename.clone();
    }
    entry
        .file
        .as_ref()
        .and_then(|file| Path::new(&file.path).file_name())
        .map(|filename| filename.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn plan_from_resolved(mod_id: &str, version: &str, resolved: &ResolvedMod) -> InstalledMod {
    InstalledMod {
        slug: resolved.slug.clone(),
        mod_id: mod_id.to_string(),
        version: version.to_string(),
        filename: resolved.filename.clone(),
        provider: resolved.provider.clone(),
        project_id: resolved
            .modrinth
            .as_ref()
            .map(|modrinth| modrinth.project_id.clone())
            .unwrap_or_default(),
        version_id: resolved
            .modrinth
            .as_ref()
            .map(|modrinth| modrinth.version_id.clone())
            .unwrap_or_default(),
        modrinth_version: resolved
            .modrinth
            .as_ref()
            .map(|modrinth| modrinth.version_number.clone())
            .unwrap_or_default(),
        download_url: resolved.download_url.clone(),
        jar_deps: Vec::new(),
        implanted: Vec::new(),
    }
}

async fn materialize_plans(
    planned: Vec<InstalledMod>,
    resolved_candidates: &crate::outdated::ResolvedCandidates,
    mods_dir: &Path,
    loader: &str,
) -> Result<Vec<InstalledMod>, OrbitError> {
    let mut installed = Vec::new();
    for mut plan in planned {
        let key = (plan.mod_id.clone(), plan.version.clone());
        let Some(resolved) = resolved_candidates.get(&key) else {
            continue;
        };
        let dest_path = download_mod(resolved, mods_dir).await?;
        let metadata = crate::jar::read_mod_metadata(&dest_path, loader)?;
        if !metadata.mod_id.is_empty() {
            plan.mod_id = metadata.mod_id;
        }
        if !metadata.version.is_empty() {
            plan.version = metadata.version;
        }
        plan.jar_deps = metadata.dependencies;
        plan.implanted = metadata
            .implanted_mods
            .into_iter()
            .map(|implanted| crate::lockfile::ImplantedMod {
                name: if implanted.mod_id.is_empty() {
                    implanted.name
                } else {
                    implanted.mod_id
                },
                version: implanted.version,
                sha256: String::new(),
                filename: String::new(),
                dependencies: implanted
                    .dependencies
                    .into_iter()
                    .filter(|(name, _, required)| {
                        *required
                            && !matches!(
                                name.as_str(),
                                "java" | "mixinextras" | "minecraft" | "fabricloader"
                            )
                    })
                    .map(|(name, version, _)| LockDependency { name, version })
                    .collect(),
            })
            .collect();
        installed.push(plan);
    }
    Ok(installed)
}

async fn download_mod(m: &ResolvedMod, mods_dir: &Path) -> Result<PathBuf, OrbitError> {
    let final_path = mods_dir.join(&m.filename);
    if final_path.exists() {
        if !m.sha512.is_empty() {
            let existing_sha = crate::jar::compute_sha512(&final_path).unwrap_or_default();
            if existing_sha == m.sha512 {
                return Ok(final_path);
            }
        } else {
            let meta = std::fs::metadata(&final_path).map_err(OrbitError::Io)?;
            if meta.len() > 0 {
                return Ok(final_path);
            }
        }
    }

    // 查全局缓存
    if let Ok(cache) = crate::jar_cache::JarCache::load()
        && cache.copy_to(&m.sha512, &final_path)
        && final_path.exists()
    {
        return Ok(final_path);
    }

    let client = download_client();
    let response = client
        .get(&m.download_url)
        .send()
        .await
        .map_err(OrbitError::Network)?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(OrbitError::Other(anyhow::anyhow!(
            "download of '{}' failed with HTTP {}: {}",
            m.filename,
            status,
            body
        )));
    }
    let bytes = response.bytes().await.map_err(OrbitError::Network)?;
    if !m.sha512.is_empty() {
        let actual = crate::jar::sha512_digest(&bytes);
        if actual != m.sha512 {
            return Err(OrbitError::ChecksumMismatch {
                name: m.filename.clone(),
                expected: m.sha512.clone(),
                actual,
            });
        }
    }

    // 存入全局缓存
    let _ = crate::jar_cache::JarCache::load().map(|mut c| {
        let _ = c.store_bytes(&m.sha512, &m.filename, &bytes);
    });

    let tmp_path = mods_dir.join(format!(".{}.tmp", m.filename));
    std::fs::write(&tmp_path, &bytes).map_err(OrbitError::Io)?;
    if final_path.exists() {
        std::fs::remove_file(&final_path).map_err(OrbitError::Io)?;
    }
    std::fs::rename(&tmp_path, &final_path).map_err(OrbitError::Io)?;
    Ok(final_path)
}

fn apply_to_lockfile(lockfile: &mut OrbitLockfile, installed: &[InstalledMod], mods_dir: &Path) {
    for inst in installed {
        let key = &inst.mod_id;
        lockfile.packages.retain(|e| e.mod_id != *key);
        let lock_deps: Vec<LockDependency> = inst
            .jar_deps
            .iter()
            .filter(|(_, _, required)| *required)
            .map(|(dep_id, constraint, _)| LockDependency {
                name: dep_id.clone(),
                version: if constraint.is_empty() {
                    "*".into()
                } else {
                    constraint.clone()
                },
            })
            .collect();
        let jar_path = mods_dir.join(&inst.filename);
        let sha1 = crate::jar::compute_sha1(&jar_path).unwrap_or_default();
        let sha256 = crate::jar::compute_sha256(&jar_path).unwrap_or_default();
        let sha512 = crate::jar::compute_sha512(&jar_path).unwrap_or_default();
        lockfile.packages.push(PackageEntry {
            mod_id: key.clone(),
            version: inst.version.clone(),
            sha1,
            sha256,
            sha512,
            filename: inst.filename.clone(),
            provider: inst.provider.clone(),
            modrinth: if inst.provider == "modrinth" {
                Some(ModrinthInfo {
                    project_id: inst.project_id.clone(),
                    version_id: inst.version_id.clone(),
                    version: inst.modrinth_version.clone(),
                    slug: inst.slug.clone(),
                    download_url: inst.download_url.clone(),
                })
            } else {
                None
            },
            file: if inst.provider == "file" {
                Some(FileInfo {
                    path: format!("mods/{}", inst.filename),
                })
            } else {
                None
            },
            dependencies: lock_deps,
            implanted: inst.implanted.clone(),
        });
    }
}

fn requested_requirement(
    constraint: &str,
    optional: bool,
    env: Option<&str>,
) -> Result<DependencySpec, OrbitError> {
    if let Some(env) = env
        && !matches!(env, "client" | "server" | "both")
    {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "invalid dependency environment '{env}'; expected client, server, or both"
        )));
    }
    let version = if constraint.is_empty() {
        "*".to_string()
    } else {
        constraint.to_string()
    };
    if !optional && env.is_none() {
        Ok(DependencySpec::Short(version))
    } else {
        Ok(DependencySpec::Full {
            version: Some(version),
            optional: optional.then_some(true),
            env: env.map(str::to_string),
            exclude: None,
        })
    }
}

fn ensure_root_requirement(
    manifest: &mut OrbitManifest,
    package: &str,
    requirement: DependencySpec,
) {
    manifest
        .dependencies
        .entry(package.to_string())
        .or_insert(requirement);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> OrbitManifest {
        toml::from_str(
            r#"
[project]
name = "test"
mc_version = "1"
modloader = "forge"
modloader_version = "1"
"#,
        )
        .unwrap()
    }

    fn locked_package(mod_id: &str, dependencies: &[&str]) -> PackageEntry {
        PackageEntry {
            mod_id: mod_id.to_string(),
            version: "1".to_string(),
            sha1: String::new(),
            sha256: String::new(),
            sha512: String::new(),
            filename: format!("{mod_id}.jar"),
            provider: "file".to_string(),
            modrinth: None,
            file: Some(FileInfo {
                path: format!("mods/{mod_id}.jar"),
            }),
            dependencies: dependencies
                .iter()
                .map(|dependency| LockDependency {
                    name: (*dependency).to_string(),
                    version: "*".to_string(),
                })
                .collect(),
            implanted: Vec::new(),
        }
    }

    #[test]
    fn requested_constraint_is_bound_to_the_actual_package_id() {
        let mut manifest = manifest();

        ensure_root_requirement(
            &mut manifest,
            "actual-mod-id",
            requested_requirement("^1", false, None).unwrap(),
        );

        assert_eq!(
            manifest.dependencies["actual-mod-id"].version_constraint(),
            Some("^1")
        );
    }

    #[test]
    fn installed_version_does_not_replace_an_existing_requirement() {
        let mut manifest = manifest();
        ensure_root_requirement(
            &mut manifest,
            "example",
            requested_requirement("^1", false, None).unwrap(),
        );

        ensure_root_requirement(
            &mut manifest,
            "example",
            requested_requirement("1.5", false, None).unwrap(),
        );

        assert_eq!(
            manifest.dependencies["example"].version_constraint(),
            Some("^1")
        );
    }

    #[test]
    fn optional_environment_requirement_uses_full_manifest_form() {
        let requirement = requested_requirement("^1", true, Some("client")).unwrap();

        match requirement {
            DependencySpec::Full {
                version,
                optional,
                env,
                exclude,
            } => {
                assert_eq!(version.as_deref(), Some("^1"));
                assert_eq!(optional, Some(true));
                assert_eq!(env.as_deref(), Some("client"));
                assert!(exclude.is_none());
            }
            DependencySpec::Short(_) => panic!("expected full dependency form"),
        }
    }

    #[test]
    fn invalid_dependency_environment_is_rejected() {
        let error = requested_requirement("*", false, Some("desktop")).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("expected client, server, or both")
        );
    }

    #[test]
    fn restore_selection_filters_roots_but_keeps_transitive_dependencies() {
        let manifest: OrbitManifest = toml::from_str(
            r#"
[project]
name = "test"
mc_version = "1"
modloader = "fabric"
modloader_version = "1"

[dependencies]
client-mod = { version = "*", env = "client" }
server-mod = { version = "*", env = "server" }
optional-mod = { version = "*", optional = true }

[groups.small]
dependencies = ["client-mod", "optional-mod"]
"#,
        )
        .unwrap();
        let lockfile = OrbitLockfile {
            meta: LockMeta {
                mc_version: "1".to_string(),
                modloader: "fabric".to_string(),
                modloader_version: "1".to_string(),
            },
            packages: vec![
                locked_package("client-mod", &["library"]),
                locked_package("server-mod", &[]),
                locked_package("optional-mod", &[]),
                locked_package("library", &[]),
            ],
        };
        let options = RestoreOptions {
            target: Some("client".to_string()),
            group: Some("small".to_string()),
            no_optional: true,
            locked: false,
            dry_run: false,
        };

        let (selected, skipped) = selected_packages(&manifest, &lockfile, &options).unwrap();

        assert_eq!(selected, vec!["client-mod", "library"]);
        assert_eq!(skipped, vec!["optional-mod", "server-mod"]);
    }

    #[test]
    fn unlocked_metadata_mismatch_rebuilds_the_lock() {
        let manifest = manifest();
        let mut lockfile = OrbitLockfile {
            meta: LockMeta {
                mc_version: "old".to_string(),
                modloader: "forge".to_string(),
                modloader_version: "1".to_string(),
            },
            packages: vec![locked_package("old", &[])],
        };

        reconcile_lock_metadata(&manifest, &mut lockfile, false).unwrap();

        assert_eq!(lockfile.meta.mc_version, manifest.project.mc_version);
        assert!(lockfile.packages.is_empty());
    }
}
