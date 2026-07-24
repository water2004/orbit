//! 模组安装 / 卸载逻辑。
//!
//! 提供顶层 API 供 CLI 调用。CLI 层不直接操作 TOML / 文件。

use std::path::{Path, PathBuf};

use crate::error::OrbitError;
use crate::lockfile::{
    CurseForgeInfo, FileInfo, LockMeta, ModrinthInfo, OrbitLockfile, PackageEntry,
};
use crate::manifest::{DependencySpec, OrbitManifest};
use crate::providers::{ModProvider, RemoteArtifact};
use crate::resolver::types::{CandidateDiagnostic, ResolutionSelector};
use crate::workspace::{Lockfile, ManifestFile};

mod local;

pub use local::install_local_file_to_instance;

pub type InstallPrompt = Box<dyn FnOnce(&InstallReport) -> bool + Send>;
pub type PackageSelector = Box<dyn FnOnce(&[String]) -> usize + Send>;

#[derive(Default)]
pub struct InstallInteraction {
    /// Selects one JAR-declared package when a provider locator contains
    /// artifacts with multiple real `mod_id` values.
    pub select_package: Option<PackageSelector>,
    pub select_resolution: Option<ResolutionSelector>,
    pub confirm_install: Option<InstallPrompt>,
}

#[derive(Debug, Clone, Default)]
pub struct InstallOptions {
    pub no_deps: bool,
    pub dry_run: bool,
    pub intent: InstallIntent,
    pub optional: bool,
    pub env: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InstallIntent {
    #[default]
    Add,
    Upgrade,
}

/// 单次 install 报告
#[derive(Debug, Clone)]
pub struct InstallReport {
    pub installed: Vec<InstalledMod>,
    pub removed: Vec<RemovedPackage>,
    pub changes: Vec<crate::resolver::types::PackageChange>,
    pub already_satisfied: Vec<String>,
    pub skipped_optional: Vec<String>,
    pub diagnostics: Vec<CandidateDiagnostic>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovedPackage {
    pub mod_id: String,
    pub version: String,
    pub filename: String,
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
    pub removed: Vec<RemovedPackage>,
    pub already_present: Vec<String>,
    pub skipped: Vec<String>,
    pub diagnostics: Vec<CandidateDiagnostic>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct InstalledMod {
    /// Remote candidate identity used only while materializing this plan.
    pub candidate_id: Option<String>,
    pub slug: String,
    pub mod_id: String,
    /// JAR loader 元数据声明的版本
    pub version: String,
    pub filename: String,
    pub provider: String,
    pub modrinth: Option<ModrinthInfo>,
    pub curseforge: Option<CurseForgeInfo>,
    pub dependencies: Vec<crate::metadata::DependencyExpression>,
    pub environment: crate::metadata::Environment,
    pub provides: Vec<crate::metadata::ProvidedMod>,
    pub language_loader: Option<crate::metadata::LanguageLoaderRequirement>,
    pub embedded_artifacts: Vec<crate::metadata::EmbeddedArtifact>,
    pub bundled: Vec<crate::lockfile::BundledMod>,
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
    jar_cache: &crate::jar_cache::JarCache,
    options: InstallOptions,
    interaction: InstallInteraction,
) -> Result<InstallReport, OrbitError> {
    let dry_run = options.dry_run;
    let mut manifest_file = ManifestFile::open(instance_dir)?;
    let platform = crate::platform::discover_install_platform(
        instance_dir,
        &manifest_file.inner.project.mc_version,
    )?;
    let platform_changed =
        crate::platform::apply_to_manifest(instance_dir, &mut manifest_file.inner, &platform)?;
    let mut lock = Lockfile::open_or_default(
        instance_dir,
        LockMeta {
            mc_version: manifest_file.inner.project.mc_version.clone(),
            modloader: manifest_file.inner.project.modloader.clone(),
            modloader_version: manifest_file.inner.project.modloader_version.clone(),
        },
    );
    lock.inner.meta = LockMeta {
        mc_version: manifest_file.inner.project.mc_version.clone(),
        modloader: manifest_file.inner.project.modloader.clone(),
        modloader_version: manifest_file.inner.project.modloader_version.clone(),
    };

    let mods_dir = instance_dir.join("mods");
    if !mods_dir.exists() && !dry_run {
        std::fs::create_dir_all(&mods_dir).map_err(OrbitError::Io)?;
    }

    let loader_package = platform.loader_package;
    let report = install_mod(InstallModInput {
        slug,
        constraint,
        providers,
        jar_cache,
        manifest: &mut manifest_file.inner,
        lockfile: &mut lock.inner,
        mods_dir: &mods_dir,
        loader_package,
        options,
        interaction,
    })
    .await?;

    if !dry_run && (platform_changed || !report.installed.is_empty() || !report.removed.is_empty())
    {
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
    jar_cache: &crate::jar_cache::JarCache,
    options: RestoreOptions,
    interaction: InstallInteraction,
) -> Result<RestoreReport, OrbitError> {
    validate_restore_options(&options)?;
    let mut manifest = ManifestFile::open(instance_dir)?;
    let platform = crate::platform::discover_install_platform(
        instance_dir,
        &manifest.inner.project.mc_version,
    )?;
    let platform_changed =
        crate::platform::apply_to_manifest(instance_dir, &mut manifest.inner, &platform)?;
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
    let lock_metadata_changed =
        reconcile_lock_metadata(&manifest.inner, &mut lock.inner, options.locked)?;
    let InstallInteraction {
        select_package: _,
        select_resolution,
        confirm_install,
    } = interaction;
    let mods_dir = instance_dir.join("mods");
    let loader_package = platform.loader_package;

    let mut report = RestoreReport::default();
    let missing_roots: Vec<_> = manifest
        .inner
        .dependencies
        .keys()
        .filter(|package| lock.inner.find(package).is_none())
        .cloned()
        .collect();
    let lock_graph_error = crate::resolver::check_lockfile_graph_with_loader(
        &manifest.inner,
        &lock.inner,
        loader_package.as_ref(),
    )
    .err();
    if !missing_roots.is_empty() || lock_graph_error.is_some() {
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
        let resolution = resolve_missing_lock_entries(
            &manifest.inner,
            &mut lock.inner,
            providers,
            jar_cache,
            loader_package.clone(),
            select_resolution,
        )
        .await?;
        let removals = package_removals(&resolution.changes);
        if confirm_install.is_some_and(|confirm| {
            !confirm(&InstallReport {
                installed: Vec::new(),
                removed: removals.clone(),
                changes: resolution.changes.clone(),
                already_satisfied: Vec::new(),
                skipped_optional: Vec::new(),
                diagnostics: resolution.diagnostics.clone(),
                warnings: resolution.warnings.clone(),
            })
        }) {
            return Ok(report);
        }
        if !options.dry_run {
            remove_packages(&mods_dir, &removals, &[])?;
            lock.save()?;
        }
        report.removed = removals;
        report.diagnostics = resolution.diagnostics;
        report.warnings = resolution.warnings;
    }

    let (selected, skipped) = selected_packages(
        &manifest.inner,
        &lock.inner,
        &options,
        loader_package.as_ref(),
    )?;
    report.skipped = skipped;
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
            jar_cache,
            options.locked,
        )
        .await?;
        lock_changed = true;
        report.restored.push(package);
    }
    if !options.dry_run && (lock_changed || lock_metadata_changed) {
        lock.save()?;
    }
    if !options.dry_run && platform_changed {
        manifest.save()?;
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
    jar_cache: &crate::jar_cache::JarCache,
    dry_run: bool,
    interaction: InstallInteraction,
) -> Result<InstallReport, OrbitError> {
    let mut manifest_file = ManifestFile::open(instance_dir)?;
    let platform = crate::platform::discover_install_platform(
        instance_dir,
        &manifest_file.inner.project.mc_version,
    )?;
    let platform_changed =
        crate::platform::apply_to_manifest(instance_dir, &mut manifest_file.inner, &platform)?;
    let mut lock = Lockfile::open_or_default(
        instance_dir,
        LockMeta {
            mc_version: manifest_file.inner.project.mc_version.clone(),
            modloader: manifest_file.inner.project.modloader.clone(),
            modloader_version: manifest_file.inner.project.modloader_version.clone(),
        },
    );
    lock.inner.meta = LockMeta {
        mc_version: manifest_file.inner.project.mc_version.clone(),
        modloader: manifest_file.inner.project.modloader.clone(),
        modloader_version: manifest_file.inner.project.modloader_version.clone(),
    };

    let crate::outdated::OutdatedReport {
        updates: _,
        resolved: resolved_candidates,
        changes: _,
        resolution,
        diagnostics,
        warnings,
    } = crate::outdated::check_all_outdated(
        instance_dir,
        &manifest_file.inner,
        &lock.inner,
        providers,
        interaction.select_resolution,
        jar_cache,
    )
    .await?;

    if !resolution.has_upgrade() {
        if !dry_run && platform_changed {
            manifest_file.save()?;
        }
        return Ok(InstallReport {
            installed: vec![],
            removed: vec![],
            changes: vec![],
            already_satisfied: vec![],
            skipped_optional: vec![],
            diagnostics,
            warnings,
        });
    }

    let mods_dir = instance_dir.join("mods");
    if !mods_dir.exists() && !dry_run {
        std::fs::create_dir_all(&mods_dir).map_err(OrbitError::Io)?;
    }

    let loader = &manifest_file.inner.project.modloader;
    let mut planned = Vec::new();
    for (package, candidate_id) in &resolution.selected_candidates {
        let version = &resolution.selected_versions[package];
        let resolved = resolved_candidate(&resolved_candidates, candidate_id)?;
        planned.push(plan_from_resolved(package, version, candidate_id, resolved));
    }
    let removals = package_removals(&resolution.changes);

    let report = InstallReport {
        installed: planned.clone(),
        removed: removals.clone(),
        changes: resolution.changes.clone(),
        already_satisfied: vec![],
        skipped_optional: vec![],
        diagnostics: diagnostics.clone(),
        warnings: warnings.clone(),
    };

    if let Some(prompt) = interaction.confirm_install
        && !prompt(&report)
    {
        return Ok(InstallReport {
            installed: vec![],
            removed: vec![],
            changes: vec![],
            already_satisfied: vec![],
            skipped_optional: vec![],
            diagnostics,
            warnings,
        }); // aborted
    }

    if dry_run {
        return Ok(report);
    }

    let installed = materialize_plans(
        planned,
        &resolved_candidates,
        &mods_dir,
        loader,
        providers,
        jar_cache,
    )
    .await?;
    remove_packages(&mods_dir, &removals, &installed)?;
    retain_selected_lock_entries(&mut lock.inner, &resolution.selected_sources);

    apply_to_lockfile(&mut lock.inner, &installed, &mods_dir);

    if platform_changed || !installed.is_empty() || !removals.is_empty() {
        manifest_file.save()?;
        lock.save()?;
    }

    Ok(InstallReport {
        installed,
        removed: removals,
        changes: resolution.changes,
        already_satisfied: vec![],
        skipped_optional: vec![],
        diagnostics,
        warnings,
    })
}

/// 顶层 API：从指定实例目录移除模组。
///
/// `input` 可以是 JAR loader 元数据声明的 `mod_id` 或平台 slug。
/// 先从 lockfile 查找（同时匹配 mod_id 和平台 slug），
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
/// 返回 (mod_id, slug)，slug 从 lockfile 的平台专属子表读取，
/// 若 lockfile 不存在或无平台来源信息则回退到 mod_id。
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
                .and_then(PackageEntry::source_slug)
                .map(str::to_string)
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
    pub environment: String,
    pub optional: bool,
    /// 依赖的 mod_id 列表
    pub dependencies: Vec<String>,
    /// 顶层包内容中声明的其他模组模块 (mod_id, version)
    pub bundled: Vec<(String, String)>,
}

/// 读取 lockfile 中所有已安装模组供 list 命令展示。
pub fn list_installed(instance_dir: &Path) -> Result<ListOutput, OrbitError> {
    let manifest = ManifestFile::open(instance_dir)?;
    let lock = Lockfile::open(instance_dir)?;
    Ok(list_output(&manifest.inner, &lock.inner, None))
}

/// Read installed packages selected for a client/server target.
///
/// Environment filters apply to manifest roots; their transitive dependencies
/// remain visible so the result describes an installable closure.
pub fn list_installed_for_target(
    instance_dir: &Path,
    target: &str,
) -> Result<ListOutput, OrbitError> {
    let mut manifest = ManifestFile::open(instance_dir)?;
    let lock = Lockfile::open(instance_dir)?;
    let options = RestoreOptions {
        target: Some(target.to_string()),
        ..RestoreOptions::default()
    };
    validate_restore_options(&options)?;
    let platform = crate::platform::discover_install_platform(
        instance_dir,
        &manifest.inner.project.mc_version,
    )?;
    crate::platform::apply_to_manifest(instance_dir, &mut manifest.inner, &platform)?;
    let loader_package = platform.loader_package;
    let (selected, _) = selected_packages(
        &manifest.inner,
        &lock.inner,
        &options,
        loader_package.as_ref(),
    )?;
    let selected: std::collections::HashSet<_> = selected.into_iter().collect();
    Ok(list_output(&manifest.inner, &lock.inner, Some(&selected)))
}

fn list_output(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    selected: Option<&std::collections::HashSet<String>>,
) -> ListOutput {
    let packages: Vec<ListedPackage> = lockfile
        .packages
        .iter()
        .filter(|entry| selected.is_none_or(|selected| selected.contains(&entry.mod_id)))
        .map(|entry| {
            let requirement = manifest.dependencies.get(&entry.mod_id);
            ListedPackage {
                mod_id: entry.mod_id.clone(),
                version: entry.version.clone(),
                slug: entry.source_slug().map(str::to_string),
                provider: entry.provider.clone(),
                environment: requirement
                    .and_then(DependencySpec::env)
                    .unwrap_or("both")
                    .to_string(),
                optional: requirement.is_some_and(DependencySpec::optional),
                dependencies: declared_dependency_ids(&entry.dependencies)
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                bundled: bundled_pairs(&entry.bundled),
            }
        })
        .collect();
    ListOutput { packages }
}

fn declared_dependency_ids(dependencies: &[crate::metadata::DependencyExpression]) -> Vec<&str> {
    let mut ids = Vec::new();
    for dependency in dependencies {
        for relation in dependency.relations() {
            if !ids.contains(&relation.id.as_str()) {
                ids.push(relation.id.as_str());
            }
        }
    }
    ids
}

fn bundled_pairs(bundled: &[crate::lockfile::BundledMod]) -> Vec<(String, String)> {
    fn collect(bundled: &[crate::lockfile::BundledMod], output: &mut Vec<(String, String)>) {
        for metadata in bundled {
            output.push((metadata.mod_id.clone(), metadata.version.clone()));
            collect(&metadata.bundled, output);
        }
    }

    let mut output = Vec::new();
    collect(bundled, &mut output);
    output
}

// ── 内部实现 ──────────────────────────────────────────────────────────

struct InstallModInput<'a> {
    slug: &'a str,
    constraint: &'a str,
    providers: &'a [Box<dyn ModProvider>],
    jar_cache: &'a crate::jar_cache::JarCache,
    manifest: &'a mut OrbitManifest,
    lockfile: &'a mut OrbitLockfile,
    mods_dir: &'a Path,
    loader_package: Option<crate::resolver::types::PlatformCandidate>,
    options: InstallOptions,
    interaction: InstallInteraction,
}

async fn install_mod(input: InstallModInput<'_>) -> Result<InstallReport, OrbitError> {
    let InstallModInput {
        slug,
        constraint,
        providers,
        jar_cache,
        manifest,
        lockfile,
        mods_dir,
        loader_package,
        options,
        interaction,
    } = input;

    if options.intent == InstallIntent::Add
        && crate::resolver::find_entry(slug, &lockfile.packages).is_some()
    {
        return Err(OrbitError::Conflict(format!(
            "'{slug}' already in lockfile. Use 'orbit upgrade {slug}' to update it."
        )));
    }

    let loader = &manifest.project.modloader;
    let mc_version = &manifest.project.mc_version;

    // 1-2. BFS download all JARs
    let seeds = vec![slug.to_string()];
    let mut catalog = crate::outdated::download_candidates_with_fallback(
        providers, &seeds, mc_version, loader, jar_cache,
    )
    .await?;
    catalog.loader_package = loader_package;
    if catalog.candidates.is_empty() {
        return Err(OrbitError::ModNotFound(slug.to_string()));
    }
    let requested_requirement =
        requested_requirement(constraint, options.optional, options.env.as_deref())?;
    let (requested_package, mut portfolio) = resolve_requested_package(
        slug,
        options.intent,
        manifest,
        lockfile,
        &catalog,
        requested_requirement.clone(),
        interaction.select_package,
    )
    .await?;

    // 3. Resolve offline
    if options.intent == InstallIntent::Upgrade {
        portfolio
            .alternatives
            .retain(crate::resolver::types::ResolutionReport::has_upgrade);
        if portfolio.alternatives.is_empty() {
            return Ok(InstallReport {
                installed: Vec::new(),
                removed: Vec::new(),
                changes: Vec::new(),
                already_satisfied: Vec::new(),
                skipped_optional: Vec::new(),
                diagnostics: Vec::new(),
                warnings: Vec::new(),
            });
        }
    }
    let resolution = crate::resolver::select_resolution(portfolio, interaction.select_resolution)
        .map_err(OrbitError::Conflict)?;
    let selected_versions = resolution.selected_versions.clone();
    let selected_sources = resolution.selected_sources.clone();
    let selected_candidates = resolution.selected_candidates.clone();
    let removals = package_removals(&resolution.changes);
    let changes = resolution.changes;
    let diagnostics = resolution.diagnostics;
    let warnings = resolution.warnings;

    // 4. Download resolved versions and apply
    let mut planned = Vec::new();
    let already_satisfied = Vec::new();

    for (mod_id, candidate_id) in &selected_candidates {
        let new_ver = &selected_versions[mod_id];
        let resolved = resolved_candidate(&catalog.resolved, candidate_id)?;
        if options.no_deps && mod_id != &requested_package {
            continue;
        }

        planned.push(plan_from_resolved(mod_id, new_ver, candidate_id, resolved));
    }

    let report = InstallReport {
        installed: planned.clone(),
        removed: removals.clone(),
        changes: changes.clone(),
        already_satisfied: already_satisfied.clone(),
        skipped_optional: vec![],
        diagnostics: diagnostics.clone(),
        warnings: warnings.clone(),
    };

    if let Some(prompt) = interaction.confirm_install
        && !prompt(&report)
    {
        return Ok(InstallReport {
            installed: vec![],
            removed: vec![],
            changes: vec![],
            already_satisfied,
            skipped_optional: vec![],
            diagnostics,
            warnings,
        }); // aborted
    }

    if options.dry_run {
        return Ok(report);
    }

    let installed = materialize_plans(
        planned,
        &catalog.resolved,
        mods_dir,
        loader,
        providers,
        jar_cache,
    )
    .await?;
    remove_packages(mods_dir, &removals, &installed)?;
    retain_selected_lock_entries(lockfile, &selected_sources);

    if installed
        .iter()
        .any(|installed| installed.mod_id == requested_package)
    {
        ensure_root_requirement(manifest, &requested_package, requested_requirement);
    }
    apply_to_lockfile(lockfile, &installed, mods_dir);

    Ok(InstallReport {
        installed,
        removed: removals,
        changes,
        already_satisfied,
        skipped_optional: vec![],
        diagnostics,
        warnings,
    })
}

async fn resolve_requested_package(
    locator: &str,
    intent: InstallIntent,
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    catalog: &crate::resolver::types::CandidateCatalog,
    requirement: DependencySpec,
    selector: Option<PackageSelector>,
) -> Result<(String, crate::resolver::types::ResolutionPortfolio), OrbitError> {
    if intent == InstallIntent::Upgrade {
        let package = crate::resolver::find_entry(locator, &lockfile.packages)
            .map(|entry| entry.mod_id.clone())
            .ok_or_else(|| OrbitError::ModNotFound(locator.to_string()))?;
        if !catalog.candidates.contains_key(&package) {
            return Err(OrbitError::Conflict(format!(
                "provider locator '{locator}' no longer returns a JAR declaring the installed \
                 package '{package}'; a mod_id change is a package replacement, not an upgrade"
            )));
        }
        let mut resolution_manifest = manifest.clone();
        ensure_root_requirement(&mut resolution_manifest, &package, requirement);
        let portfolio =
            crate::resolver::resolve_candidate_portfolio(&resolution_manifest, lockfile, catalog)
                .await
                .map_err(OrbitError::Conflict)?;
        return Ok((package, portfolio));
    }

    let mut packages = catalog.packages_for_locator(locator);
    if packages.is_empty() && catalog.candidates.contains_key(locator) {
        packages.push(locator.to_string());
    }
    if packages.is_empty() {
        return Err(OrbitError::ModNotFound(locator.to_string()));
    }

    let mut feasible = Vec::new();
    let mut failures = Vec::new();
    for package in packages {
        if lockfile
            .packages
            .iter()
            .any(|entry| entry.mod_id == package)
        {
            failures.push((
                package.clone(),
                format!("package '{package}' is already installed; use 'orbit upgrade {package}'"),
            ));
            continue;
        }
        let mut resolution_manifest = manifest.clone();
        ensure_root_requirement(&mut resolution_manifest, &package, requirement.clone());
        match crate::resolver::resolve_candidate_portfolio(&resolution_manifest, lockfile, catalog)
            .await
        {
            Ok(portfolio) => feasible.push((package, portfolio)),
            Err(error) => failures.push((package, error)),
        }
    }

    if feasible.is_empty() {
        let details = failures
            .into_iter()
            .map(|(package, error)| format!("  - {package}: {error}"))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(OrbitError::Conflict(format!(
            "provider locator '{locator}' contains no feasible JAR-declared package:\n{details}"
        )));
    }

    let index = if feasible.len() == 1 {
        0
    } else {
        let package_names: Vec<_> = feasible
            .iter()
            .map(|(package, _)| package.clone())
            .collect();
        selector
            .map(|select| select(&package_names))
            .unwrap_or_default()
    };
    if index >= feasible.len() {
        return Err(OrbitError::Conflict(format!(
            "package selector returned invalid choice {} for {} JAR-declared packages",
            index + 1,
            feasible.len()
        )));
    }
    Ok(feasible.swap_remove(index))
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
) -> Result<bool, OrbitError> {
    if lockfile.meta.mc_version == manifest.project.mc_version
        && lockfile.meta.modloader == manifest.project.modloader
        && lockfile.meta.modloader_version == manifest.project.modloader_version
    {
        return Ok(false);
    }
    if lockfile.meta.mc_version != manifest.project.mc_version
        || lockfile.meta.modloader != manifest.project.modloader
    {
        if locked {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "--locked: orbit.lock Minecraft/modloader metadata does not match the actual instance"
            )));
        }
        lockfile.packages.clear();
    }
    lockfile.meta = LockMeta {
        mc_version: manifest.project.mc_version.clone(),
        modloader: manifest.project.modloader.clone(),
        modloader_version: manifest.project.modloader_version.clone(),
    };
    Ok(true)
}

async fn resolve_missing_lock_entries(
    manifest: &OrbitManifest,
    lockfile: &mut OrbitLockfile,
    providers: &[Box<dyn ModProvider>],
    jar_cache: &crate::jar_cache::JarCache,
    loader_package: Option<crate::resolver::types::PlatformCandidate>,
    selector: Option<ResolutionSelector>,
) -> Result<crate::resolver::types::ResolutionReport, OrbitError> {
    let mut catalog = crate::outdated::download_lockfile_candidate_catalog(
        providers,
        lockfile,
        &manifest.project.mc_version,
        &manifest.project.modloader,
        jar_cache,
    )
    .await?;
    catalog.loader_package = loader_package;

    let portfolio = crate::resolver::resolve_candidate_portfolio(manifest, lockfile, &catalog)
        .await
        .map_err(|error| OrbitError::Conflict(error.to_string()))?;
    let resolution =
        crate::resolver::select_resolution(portfolio, selector).map_err(OrbitError::Conflict)?;
    retain_selected_lock_entries(lockfile, &resolution.selected_sources);
    for (package, candidate_id) in &resolution.selected_candidates {
        let version = &resolution.selected_versions[package];
        let resolved = resolved_candidate(&catalog.resolved, candidate_id)?;
        let candidate = catalog.candidates.get(package).and_then(|versions| {
            versions
                .iter()
                .find(|candidate| candidate.id == *candidate_id)
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
    artifact: &RemoteArtifact,
    candidate: Option<&crate::resolver::types::CandidateVersion>,
) -> PackageEntry {
    let dependencies = candidate
        .map(|candidate| candidate.dependencies.clone())
        .unwrap_or_default();
    let bundled = candidate
        .map(|candidate| {
            candidate
                .bundled
                .iter()
                .map(crate::lockfile::BundledMod::from_candidate)
                .collect()
        })
        .unwrap_or_default();
    PackageEntry {
        mod_id: package.to_string(),
        version: version.to_string(),
        sha1: artifact.sha1.clone(),
        sha256: String::new(),
        sha512: artifact.sha512.clone(),
        filename: artifact.filename.clone(),
        provider: artifact.provider.clone(),
        modrinth: artifact.modrinth.as_ref().map(|modrinth| ModrinthInfo {
            project_id: modrinth.project_id.clone(),
            version_id: modrinth.version_id.clone(),
            slug: artifact.slug.clone(),
            download_url: artifact.download_url.clone(),
        }),
        curseforge: artifact
            .curseforge
            .as_ref()
            .map(|curseforge| CurseForgeInfo {
                project_id: curseforge.project_id,
                file_id: curseforge.file_id,
                slug: artifact.slug.clone(),
                download_url: artifact.download_url.clone(),
            }),
        file: None,
        dependencies,
        environment: candidate
            .map(|candidate| candidate.environment)
            .unwrap_or_default(),
        provides: candidate
            .map(|candidate| candidate.provides.clone())
            .unwrap_or_default(),
        language_loader: candidate.and_then(|candidate| candidate.language_loader.clone()),
        embedded_artifacts: candidate
            .map(|candidate| candidate.embedded_artifacts.clone())
            .unwrap_or_default(),
        bundled,
    }
}

pub(crate) fn selected_packages(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    options: &RestoreOptions,
    loader_package: Option<&crate::resolver::types::PlatformCandidate>,
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

    let target = match target {
        "client" => crate::metadata::Environment::Client,
        "server" => crate::metadata::Environment::Server,
        _ => crate::metadata::Environment::Both,
    };
    let roots: std::collections::HashSet<_> = roots.into_iter().collect();
    let mut selected_manifest = manifest.clone();
    selected_manifest
        .dependencies
        .retain(|package, _| roots.contains(package));
    let solution = crate::resolver::resolve_lockfile_for_target(
        &selected_manifest,
        lockfile,
        target,
        loader_package,
    )
    .map_err(|error| OrbitError::Conflict(error.to_string()))?;
    let mut selected: Vec<_> = solution
        .iter()
        .filter_map(|(package, _)| package.top_level_mod_id())
        .filter(|package| lockfile.find(package).is_some())
        .map(str::to_string)
        .collect();
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
    if !entry.sha1.is_empty() {
        return Ok(crate::jar::compute_sha1(&path)?.eq_ignore_ascii_case(&entry.sha1));
    }
    Ok(std::fs::metadata(path)?.len() > 0)
}

async fn restore_package(
    entry: &mut PackageEntry,
    instance_dir: &Path,
    mods_dir: &Path,
    providers: &[Box<dyn ModProvider>],
    jar_cache: &crate::jar_cache::JarCache,
    locked: bool,
) -> Result<(), OrbitError> {
    match entry.provider.as_str() {
        "file" => restore_file_package(entry, instance_dir, mods_dir),
        "modrinth" | "curseforge" => {
            restore_platform_package(entry, mods_dir, providers, jar_cache, locked).await
        }
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

async fn restore_platform_package(
    entry: &mut PackageEntry,
    mods_dir: &Path,
    providers: &[Box<dyn ModProvider>],
    jar_cache: &crate::jar_cache::JarCache,
    locked: bool,
) -> Result<(), OrbitError> {
    let provider =
        crate::providers::find_provider(providers, &entry.provider).ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!(
                "{} provider is required to restore '{}'",
                entry.provider,
                entry.mod_id,
            ))
        })?;
    let project_id = entry.source_project_id().ok_or_else(|| {
        OrbitError::Other(anyhow::anyhow!(
            "{} package '{}' has no provider metadata",
            entry.provider,
            entry.mod_id
        ))
    })?;
    let version_id = entry.source_version_id().ok_or_else(|| {
        OrbitError::Other(anyhow::anyhow!(
            "{} package '{}' has no file/version id",
            entry.provider,
            entry.mod_id
        ))
    })?;
    let download_url = entry.source_download_url().unwrap_or_default().to_string();
    let filename = package_filename(entry);
    if (!entry.sha512.is_empty() || !entry.sha1.is_empty()) && !filename.is_empty() {
        let destination = mods_dir.join(&filename);
        if jar_cache.copy_to(&entry.sha512, &entry.sha1, &destination) {
            verify_package_hash(entry, &destination)?;
            entry.filename = filename;
            return Ok(());
        }
    }

    let resolved = if download_url.is_empty() {
        if locked {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "--locked: '{}' has no download_url and is not available in cache",
                entry.mod_id
            )));
        }
        provider
            .get_versions(&project_id, None, None)
            .await?
            .into_iter()
            .find(|version| version.version_id().as_deref() == Some(version_id.as_str()))
            .ok_or_else(|| {
                OrbitError::ModNotFound(format!("{} version {}", entry.mod_id, version_id))
            })?
    } else {
        resolved_from_lock_entry(entry, download_url, filename.clone())?
    };
    let destination = download_mod(&resolved, provider, mods_dir, jar_cache).await?;
    verify_package_hash(entry, &destination)?;
    entry.filename = resolved.filename.clone();
    entry.sha1 = crate::jar::compute_sha1(&destination)?;
    entry.sha256 = crate::jar::compute_sha256(&destination)?;
    entry.sha512 = crate::jar::compute_sha512(&destination)?;
    if let Some(modrinth) = &mut entry.modrinth {
        modrinth.download_url = resolved.download_url.clone();
    }
    if let Some(curseforge) = &mut entry.curseforge {
        curseforge.download_url = resolved.download_url;
    }
    Ok(())
}

fn resolved_from_lock_entry(
    entry: &PackageEntry,
    download_url: String,
    filename: String,
) -> Result<RemoteArtifact, OrbitError> {
    let modrinth = entry
        .modrinth
        .as_ref()
        .map(|metadata| crate::providers::ModrinthResolvedInfo {
            project_id: metadata.project_id.clone(),
            version_id: metadata.version_id.clone(),
        });
    let curseforge =
        entry
            .curseforge
            .as_ref()
            .map(|metadata| crate::providers::CurseForgeResolvedInfo {
                project_id: metadata.project_id,
                file_id: metadata.file_id,
                fingerprint: 0,
            });
    if modrinth.is_none() && curseforge.is_none() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "{} package '{}' has no provider metadata",
            entry.provider,
            entry.mod_id
        )));
    }
    Ok(RemoteArtifact {
        sha1: entry.sha1.clone(),
        sha512: entry.sha512.clone(),
        slug: entry.source_slug().unwrap_or(&entry.mod_id).to_string(),
        provider: entry.provider.clone(),
        modrinth,
        curseforge,
        download_url,
        filename,
        related_projects: Vec::new(),
    })
}

fn verify_package_hash(entry: &PackageEntry, path: &Path) -> Result<(), OrbitError> {
    let (expected, actual) = if !entry.sha256.is_empty() {
        (&entry.sha256, crate::jar::compute_sha256(path)?)
    } else if !entry.sha512.is_empty() {
        (&entry.sha512, crate::jar::compute_sha512(path)?)
    } else if !entry.sha1.is_empty() {
        (&entry.sha1, crate::jar::compute_sha1(path)?)
    } else {
        return Ok(());
    };
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(OrbitError::ChecksumMismatch {
            name: entry.mod_id.clone(),
            expected: expected.clone(),
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

fn plan_from_resolved(
    mod_id: &str,
    version: &str,
    candidate_id: &str,
    artifact: &RemoteArtifact,
) -> InstalledMod {
    InstalledMod {
        candidate_id: Some(candidate_id.to_string()),
        slug: artifact.slug.clone(),
        mod_id: mod_id.to_string(),
        version: version.to_string(),
        filename: artifact.filename.clone(),
        provider: artifact.provider.clone(),
        modrinth: artifact.modrinth.as_ref().map(|metadata| ModrinthInfo {
            project_id: metadata.project_id.clone(),
            version_id: metadata.version_id.clone(),
            slug: artifact.slug.clone(),
            download_url: artifact.download_url.clone(),
        }),
        curseforge: artifact.curseforge.as_ref().map(|metadata| CurseForgeInfo {
            project_id: metadata.project_id,
            file_id: metadata.file_id,
            slug: artifact.slug.clone(),
            download_url: artifact.download_url.clone(),
        }),
        dependencies: Vec::new(),
        environment: crate::metadata::Environment::Both,
        provides: Vec::new(),
        language_loader: None,
        embedded_artifacts: Vec::new(),
        bundled: Vec::new(),
    }
}

async fn materialize_plans(
    planned: Vec<InstalledMod>,
    resolved_candidates: &crate::resolver::types::ResolvedCandidates,
    mods_dir: &Path,
    loader: &str,
    providers: &[Box<dyn ModProvider>],
    jar_cache: &crate::jar_cache::JarCache,
) -> Result<Vec<InstalledMod>, OrbitError> {
    let mut installed = Vec::new();
    for mut plan in planned {
        let candidate_id = plan.candidate_id.as_deref().ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!(
                "remote install plan for '{}' has no candidate identity",
                plan.mod_id
            ))
        })?;
        let resolved = resolved_candidate(resolved_candidates, candidate_id)?;
        let provider =
            crate::providers::find_provider(providers, &resolved.provider).ok_or_else(|| {
                OrbitError::Other(anyhow::anyhow!(
                    "{} provider is required to download '{}'",
                    resolved.provider,
                    resolved.filename
                ))
            })?;
        let dest_path = download_mod(resolved, provider, mods_dir, jar_cache).await?;
        let metadata = crate::jar::read_mod_metadata(&dest_path, loader)?;
        if metadata.mod_id != plan.mod_id || metadata.version != plan.version {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "downloaded JAR '{}' changed identity after resolution: expected {} {}, found {} {}",
                resolved.filename,
                plan.mod_id,
                plan.version,
                metadata.mod_id,
                metadata.version
            )));
        }
        plan.dependencies = metadata.dependencies;
        plan.environment = metadata.environment;
        plan.provides = metadata.provides;
        plan.language_loader = metadata.language_loader;
        plan.embedded_artifacts = metadata.embedded_artifacts;
        plan.bundled = metadata
            .bundled_mods
            .iter()
            .map(crate::lockfile::BundledMod::from_jar_metadata)
            .collect();
        installed.push(plan);
    }
    Ok(installed)
}

fn resolved_candidate<'a>(
    candidates: &'a crate::resolver::types::ResolvedCandidates,
    candidate_id: &str,
) -> Result<&'a RemoteArtifact, OrbitError> {
    candidates.get(candidate_id).ok_or_else(|| {
        OrbitError::Other(anyhow::anyhow!(
            "solver selected package candidate '{candidate_id}' without a download artifact"
        ))
    })
}

fn package_removals(changes: &[crate::resolver::types::PackageChange]) -> Vec<RemovedPackage> {
    let mut removals: Vec<_> = changes
        .iter()
        .filter_map(|change| {
            Some(RemovedPackage {
                mod_id: change.package.clone(),
                version: change.current_version.clone()?,
                filename: change.filename.clone()?,
            })
        })
        .collect();
    removals.sort_by(|left, right| {
        left.mod_id
            .cmp(&right.mod_id)
            .then_with(|| left.version.cmp(&right.version))
            .then_with(|| left.filename.cmp(&right.filename))
    });
    removals.dedup_by(|left, right| left.filename == right.filename);
    removals
}

fn remove_packages(
    mods_dir: &Path,
    removals: &[RemovedPackage],
    installed: &[InstalledMod],
) -> Result<(), OrbitError> {
    let installed_filenames: std::collections::HashSet<_> = installed
        .iter()
        .map(|package| package.filename.as_str())
        .collect();
    for removal in removals {
        if installed_filenames.contains(removal.filename.as_str()) {
            continue;
        }
        let filename = safe_artifact_filename(&removal.filename)?;
        let path = mods_dir.join(filename);
        if path.exists() {
            std::fs::remove_file(path).map_err(OrbitError::Io)?;
        }
    }
    Ok(())
}

fn retain_selected_lock_entries(
    lockfile: &mut OrbitLockfile,
    selected_sources: &std::collections::BTreeMap<String, String>,
) {
    lockfile.packages.retain(|entry| {
        selected_sources
            .get(&entry.mod_id)
            .is_some_and(|source| crate::resolver::locked_source(entry) == *source)
    });
}

async fn download_mod(
    m: &RemoteArtifact,
    provider: &dyn ModProvider,
    mods_dir: &Path,
    jar_cache: &crate::jar_cache::JarCache,
) -> Result<PathBuf, OrbitError> {
    if provider.name() != m.provider {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "cannot download {} artifact with {} provider",
            m.provider,
            provider.name()
        )));
    }
    let filename = safe_artifact_filename(&m.filename)?;
    let final_path = mods_dir.join(filename);
    if final_path.exists() {
        if !m.sha512.is_empty() {
            let existing_sha = crate::jar::compute_sha512(&final_path).unwrap_or_default();
            if existing_sha == m.sha512 {
                return Ok(final_path);
            }
        } else if !m.sha1.is_empty() {
            let existing_sha = crate::jar::compute_sha1(&final_path).unwrap_or_default();
            if existing_sha.eq_ignore_ascii_case(&m.sha1) {
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
    if jar_cache.copy_to(&m.sha512, &m.sha1, &final_path) && final_path.exists() {
        let cached = std::fs::read(&final_path)?;
        if crate::jar::verify_source_hash(&cached, &m.sha1, &m.sha512, &m.filename).is_ok() {
            return Ok(final_path);
        }
    }

    let bytes = provider
        .artifact_downloader()
        .download(&m.download_url, &m.filename)
        .await?;
    crate::jar::verify_source_hash(&bytes, &m.sha1, &m.sha512, &m.filename)?;

    // 存入全局缓存
    jar_cache.store_bytes(&bytes)?;

    let tmp_path = mods_dir.join(format!(".{}.tmp", filename.to_string_lossy()));
    std::fs::write(&tmp_path, &bytes).map_err(OrbitError::Io)?;
    if final_path.exists() {
        std::fs::remove_file(&final_path).map_err(OrbitError::Io)?;
    }
    std::fs::rename(&tmp_path, &final_path).map_err(OrbitError::Io)?;
    Ok(final_path)
}

fn safe_artifact_filename(filename: &str) -> Result<&std::ffi::OsStr, OrbitError> {
    let path = Path::new(filename);
    let basename = path
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!(
                "provider returned an invalid artifact filename '{filename}'"
            ))
        })?;
    if path.components().count() != 1 {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "provider returned a non-local artifact filename '{filename}'"
        )));
    }
    Ok(basename)
}

fn apply_to_lockfile(lockfile: &mut OrbitLockfile, installed: &[InstalledMod], mods_dir: &Path) {
    for inst in installed {
        let key = &inst.mod_id;
        lockfile.packages.retain(|e| e.mod_id != *key);
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
            modrinth: inst.modrinth.clone(),
            curseforge: inst.curseforge.clone(),
            file: if inst.provider == "file" {
                Some(FileInfo {
                    path: format!("mods/{}", inst.filename),
                })
            } else {
                None
            },
            dependencies: inst.dependencies.clone(),
            environment: inst.environment,
            provides: inst.provides.clone(),
            language_loader: inst.language_loader.clone(),
            embedded_artifacts: inst.embedded_artifacts.clone(),
            bundled: inst.bundled.clone(),
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
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    fn manifest() -> OrbitManifest {
        toml::from_str(
            r#"
[project]
name = "test"
mc_version = "1"
modloader = "forge"
modloader_version = "1"
[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }
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
            curseforge: None,
            file: Some(FileInfo {
                path: format!("mods/{mod_id}.jar"),
            }),
            dependencies: dependencies
                .iter()
                .map(|dependency| crate::metadata::ModDependency::required(*dependency, "*").into())
                .collect(),
            environment: crate::metadata::Environment::Both,
            provides: Vec::new(),
            language_loader: None,
            embedded_artifacts: Vec::new(),
            bundled: Vec::new(),
        }
    }

    fn remote_artifact(version_id: &str) -> RemoteArtifact {
        RemoteArtifact {
            sha1: String::new(),
            sha512: String::new(),
            slug: "gca".to_string(),
            provider: "modrinth".to_string(),
            modrinth: Some(crate::providers::ModrinthResolvedInfo {
                project_id: "UHjbX5mk".to_string(),
                version_id: version_id.to_string(),
            }),
            curseforge: None,
            download_url: format!("https://example.invalid/{version_id}.jar"),
            filename: format!("{version_id}.jar"),
            related_projects: Vec::new(),
        }
    }

    fn jar_metadata(
        mod_id: &str,
        version: &str,
        dependencies: &[&str],
    ) -> crate::jar::JarModMetadata {
        crate::jar::JarModMetadata {
            mod_id: mod_id.to_string(),
            name: mod_id.to_string(),
            version: version.to_string(),
            environment: crate::metadata::Environment::Both,
            dependencies: dependencies
                .iter()
                .map(|dependency| crate::metadata::ModDependency::required(*dependency, "*").into())
                .collect(),
            provides: Vec::new(),
            language_loader: None,
            load_condition: crate::metadata::ModLoadCondition::Always,
            origin: crate::jar::JarModOrigin::Root,
            embedded_jars: Vec::new(),
            embedded_artifacts: Vec::new(),
            bundled_mods: Vec::new(),
        }
    }

    fn empty_lockfile() -> OrbitLockfile {
        OrbitLockfile {
            meta: LockMeta {
                mc_version: "1".to_string(),
                modloader: "forge".to_string(),
                modloader_version: "1".to_string(),
            },
            packages: Vec::new(),
        }
    }

    #[tokio::test]
    async fn add_selects_between_feasible_real_packages_from_one_locator() {
        let mut catalog = crate::resolver::types::CandidateCatalog::default();
        catalog
            .record(
                jar_metadata("gca-wrapper", "1.0.1", &[]),
                remote_artifact("old"),
            )
            .unwrap();
        catalog
            .record(
                jar_metadata("gca_wrapper", "1.0.6", &[]),
                remote_artifact("new"),
            )
            .unwrap();

        let (package, portfolio) = resolve_requested_package(
            "gca",
            InstallIntent::Add,
            &manifest(),
            &empty_lockfile(),
            &catalog,
            DependencySpec::Short("*".to_string()),
            Some(Box::new(|packages| {
                packages
                    .iter()
                    .position(|package| package == "gca_wrapper")
                    .unwrap()
            })),
        )
        .await
        .unwrap();

        assert_eq!(package, "gca_wrapper");
        assert!(!portfolio.alternatives.is_empty());
        assert!(
            portfolio
                .alternatives
                .iter()
                .all(|solution| solution.selected_versions.contains_key("gca_wrapper"))
        );
    }

    #[tokio::test]
    async fn add_skips_infeasible_package_id_without_prompting() {
        let mut catalog = crate::resolver::types::CandidateCatalog::default();
        catalog
            .record(
                jar_metadata("gca-wrapper", "1.0.1", &["missing"]),
                remote_artifact("old"),
            )
            .unwrap();
        catalog
            .record(
                jar_metadata("gca_wrapper", "1.0.6", &[]),
                remote_artifact("new"),
            )
            .unwrap();
        let prompted = Arc::new(AtomicBool::new(false));
        let captured = Arc::clone(&prompted);

        let (package, _) = resolve_requested_package(
            "UHjbX5mk",
            InstallIntent::Add,
            &manifest(),
            &empty_lockfile(),
            &catalog,
            DependencySpec::Short("*".to_string()),
            Some(Box::new(move |_| {
                captured.store(true, Ordering::Relaxed);
                0
            })),
        )
        .await
        .unwrap();

        assert_eq!(package, "gca_wrapper");
        assert!(!prompted.load(Ordering::Relaxed));
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

[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }

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

        let (selected, skipped) = selected_packages(&manifest, &lockfile, &options, None).unwrap();

        assert_eq!(selected, vec!["client-mod", "library"]);
        assert_eq!(skipped, vec!["optional-mod", "server-mod"]);
    }

    #[test]
    fn restore_selection_does_not_treat_loader_packages_as_jars() {
        let manifest: OrbitManifest = toml::from_str(
            r#"
[project]
name = "test"
mc_version = "1.21.1"
modloader = "neoforge"
modloader_version = "21.1"
[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }
[dependencies]
example = "*"
"#,
        )
        .unwrap();
        let lockfile = OrbitLockfile {
            meta: LockMeta {
                mc_version: "1.21.1".to_string(),
                modloader: "neoforge".to_string(),
                modloader_version: "21.1".to_string(),
            },
            packages: vec![locked_package("example", &["minecraft", "neoforge"])],
        };

        let (selected, _) =
            selected_packages(&manifest, &lockfile, &RestoreOptions::default(), None).unwrap();

        assert_eq!(selected, ["example"]);
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
