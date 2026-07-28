//! 模组安装 / 卸载逻辑。
//!
//! 提供顶层 API 供 CLI 调用。CLI 层不直接操作 TOML / 文件。

use std::path::{Path, PathBuf};

use crate::error::OrbitError;
use crate::lockfile::{ArtifactSource, LockMeta, OrbitLockfile, PackageEntry};
use crate::manifest::{DependencySpec, OrbitManifest, PackageRemote};
use crate::progress::{
    ArtifactProgressState, ProgressEvent, ProgressReporter, emit as emit_progress,
};
use crate::providers::ModProvider;
use crate::resolver::types::{CandidateDiagnostic, ResolutionSelector};
use crate::workspace::{Lockfile, ManifestFile};

mod local;

pub use local::install_local_file_to_instance;

pub type InstallPrompt = Box<dyn FnOnce(&InstallReport) -> bool + Send>;
pub type PackageSelector = Box<dyn FnOnce(&[String]) -> Result<usize, String> + Send>;

#[derive(Default)]
pub struct InstallInteraction {
    /// Selects one JAR-declared package when a provider locator contains
    /// artifacts with multiple real `mod_id` values.
    pub select_package: Option<PackageSelector>,
    pub select_resolution: Option<ResolutionSelector>,
    pub confirm_install: Option<InstallPrompt>,
    /// Optional structured progress observer owned by the frontend.
    pub progress: Option<ProgressReporter>,
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
    pub mod_id: String,
    /// JAR loader 元数据声明的版本
    pub version: String,
    pub filename: String,
    pub remotes: Vec<PackageRemote>,
    pub artifact_sources: Vec<ArtifactSource>,
    pub dependencies: Vec<crate::metadata::DependencyExpression>,
    pub environment: crate::metadata::Environment,
    pub provides: Vec<crate::metadata::ProvidedMod>,
    pub language_loader: Option<crate::metadata::LanguageLoaderRequirement>,
    pub embedded_artifacts: Vec<crate::metadata::EmbeddedArtifact>,
    pub bundled: Vec<crate::lockfile::BundledMod>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallTarget {
    Remote(PackageRemote),
    Package(String),
}

/// 顶层 API：在指定实例目录安装模组。
///
/// 接收 `instance_dir`，内部完成 orbit.toml / orbit.lock 的读写和 mods/ 目录管理。
/// `constraint` 会绑定到候选 JAR 自声明的 `mod_id`，并作为 manifest 根约束保留。
/// `dry_run` 为 true 时仅解析不下载不写文件。
pub async fn install_to_instance(
    target: InstallTarget,
    constraint: &str,
    instance_dir: &Path,
    providers: &[Box<dyn ModProvider>],
    jar_cache: &crate::jar_cache::JarCache,
    options: InstallOptions,
    interaction: InstallInteraction,
) -> Result<InstallReport, OrbitError> {
    let dry_run = options.dry_run;
    let mut manifest_file = ManifestFile::open(instance_dir)?;
    let platform = crate::platform::Platform::load(instance_dir, &manifest_file.inner)?;
    let mut lock = Lockfile::open_or_default(
        instance_dir,
        LockMeta {
            mc_version: manifest_file.inner.project.mc_version.clone(),
            modloader: manifest_file.inner.project.modloader.clone(),
            modloader_version: manifest_file.inner.project.modloader_version.clone(),
        },
    )?;
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
        target,
        instance_dir,
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

    if !dry_run {
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
    let manifest = ManifestFile::open(instance_dir)?;
    let platform = crate::platform::Platform::load(instance_dir, &manifest.inner)?;
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
    )?;
    let lock_metadata_changed =
        reconcile_lock_metadata(&manifest.inner, &mut lock.inner, options.locked)?;
    let InstallInteraction {
        select_package: _,
        select_resolution,
        confirm_install,
        progress,
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
        let resolution = resolve_missing_lock_entries(MissingLockResolutionInput {
            instance_dir,
            manifest: &manifest.inner,
            lockfile: &mut lock.inner,
            providers,
            jar_cache,
            loader_package: loader_package.clone(),
            selector: select_resolution,
            progress: progress.clone(),
        })
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

    let total = selected.len();
    emit_progress(progress.as_ref(), ProgressEvent::ApplyStarted { total });
    let mut completed = 0;
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
        let filename = package_filename(&lock.inner.packages[index]);
        emit_progress(
            progress.as_ref(),
            ProgressEvent::ApplyArtifact {
                completed,
                total,
                filename: filename.clone(),
                state: ArtifactProgressState::Started,
            },
        );
        if package_is_present(&lock.inner.packages[index], &mods_dir)? {
            report.already_present.push(package);
            completed += 1;
            emit_progress(
                progress.as_ref(),
                ProgressEvent::ApplyArtifact {
                    completed,
                    total,
                    filename,
                    state: ArtifactProgressState::AlreadyPresent,
                },
            );
            continue;
        }
        if options.dry_run {
            report.restored.push(package);
            completed += 1;
            emit_progress(
                progress.as_ref(),
                ProgressEvent::ApplyArtifact {
                    completed,
                    total,
                    filename,
                    state: ArtifactProgressState::Finished,
                },
            );
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
        completed += 1;
        emit_progress(
            progress.as_ref(),
            ProgressEvent::ApplyArtifact {
                completed,
                total,
                filename,
                state: ArtifactProgressState::Finished,
            },
        );
    }
    emit_progress(progress.as_ref(), ProgressEvent::ApplyFinished { total });
    if !options.dry_run && (lock_changed || lock_metadata_changed) {
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
    jar_cache: &crate::jar_cache::JarCache,
    dry_run: bool,
    interaction: InstallInteraction,
) -> Result<InstallReport, OrbitError> {
    let InstallInteraction {
        select_package: _,
        select_resolution,
        confirm_install,
        progress,
    } = interaction;
    let manifest_file = ManifestFile::open(instance_dir)?;
    let platform = crate::platform::Platform::load(instance_dir, &manifest_file.inner)?;
    let mut lock = Lockfile::open_or_default(
        instance_dir,
        LockMeta {
            mc_version: manifest_file.inner.project.mc_version.clone(),
            modloader: manifest_file.inner.project.modloader.clone(),
            modloader_version: manifest_file.inner.project.modloader_version.clone(),
        },
    )?;
    lock.inner.meta = LockMeta {
        mc_version: manifest_file.inner.project.mc_version.clone(),
        modloader: manifest_file.inner.project.modloader.clone(),
        modloader_version: manifest_file.inner.project.modloader_version.clone(),
    };

    let crate::outdated::OutdatedReport {
        updates: _,
        resolved: resolved_candidates,
        candidate_remotes,
        changes: _,
        resolution,
        diagnostics,
        warnings,
    } = crate::outdated::check_all_outdated_with_progress(
        instance_dir,
        &manifest_file.inner,
        &lock.inner,
        providers,
        select_resolution,
        jar_cache,
        progress.clone(),
    )
    .await?;

    if !resolution.has_upgrade() {
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

    let loader = platform.loader;
    let mut planned = Vec::new();
    for (package, candidate_id) in &resolution.selected_candidates {
        let version = &resolution.selected_versions[package];
        let resolved = resolved_candidate(&resolved_candidates, candidate_id)?;
        let mut remotes = candidate_remotes.get(package).cloned().unwrap_or_default();
        if let Some(requirement) = manifest_file.inner.dependencies.get(package) {
            remotes.extend(requirement.remotes.iter().cloned());
        }
        if let Some(entry) = lock.inner.find(package) {
            remotes.extend(entry.remotes.iter().cloned());
        }
        remotes.sort();
        remotes.dedup();
        planned.push(plan_from_resolved(
            package,
            version,
            candidate_id,
            resolved,
            remotes,
        ));
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

    if let Some(prompt) = confirm_install
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
        progress,
    )
    .await?;
    remove_packages(&mods_dir, &removals, &installed)?;
    retain_selected_lock_entries(&mut lock.inner, &resolution.selected_sources);

    apply_to_lockfile(&mut lock.inner, &installed, &mods_dir);

    if !installed.is_empty() || !removals.is_empty() {
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
/// `package` 只能是 JAR loader 元数据声明的 `mod_id`。远端 locator
/// 不属于包身份，不能用于已安装包操作。
pub fn remove_from_instance(
    package: &str,
    instance_dir: &Path,
    dry_run: bool,
) -> Result<RemoveReport, OrbitError> {
    let mut manifest_file = ManifestFile::open(instance_dir)?;
    let mut lock = Lockfile::open(instance_dir)?;

    let entry = crate::resolver::find_entry(package, &lock.inner.packages)
        .ok_or_else(|| OrbitError::ModNotFound(package.to_string()))?;
    let key = entry.mod_id.clone();

    if !manifest_file.inner.dependencies.contains_key(&key) {
        return Err(OrbitError::ModNotFound(package.to_string()));
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

/// 列出实例中的所有顶层包身份，供 remove 找不到时交互选择。
pub fn list_dependencies(instance_dir: &Path) -> Result<Vec<String>, OrbitError> {
    let manifest_file = ManifestFile::open(instance_dir)?;
    Ok(manifest_file.inner.dependencies.keys().cloned().collect())
}

/// `orbit list` 输出结构
#[derive(Debug, Clone)]
pub struct ListOutput {
    pub packages: Vec<ListedPackage>,
    pub roots: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ListedPackage {
    pub mod_id: String,
    pub version: String,
    pub remotes: Vec<String>,
    /// Explicit root-package override; `None` means follow the selected JAR.
    pub configured_environment: Option<String>,
    /// Effective environment after applying the optional override.
    pub environment: String,
    pub root: bool,
    pub optional: bool,
    /// 依赖的 mod_id 列表
    pub dependencies: Vec<String>,
    /// 顶层包内容中声明的其他模组模块 (mod_id, version)
    pub bundled: Vec<(String, String)>,
    /// Loader-declared icon source retained for machine presentation output.
    pub icon: Option<PackageIcon>,
}

#[derive(Debug, Clone)]
pub struct PackageIcon {
    jar_path: PathBuf,
    archive_entry: String,
    sha256: String,
}

pub fn materialize_listed_package_icon(
    package: &ListedPackage,
    cache_root: &Path,
) -> Result<Option<PathBuf>, OrbitError> {
    let Some(icon) = &package.icon else {
        return Ok(None);
    };
    crate::jar::materialize_mod_icon(
        &icon.jar_path,
        &icon.archive_entry,
        &icon.sha256,
        cache_root,
    )
}

/// 读取 lockfile 中所有已安装模组供 list 命令展示。
pub fn list_installed(instance_dir: &Path) -> Result<ListOutput, OrbitError> {
    let manifest = ManifestFile::open(instance_dir)?;
    let lock = Lockfile::open(instance_dir)?;
    let roots = manifest.inner.dependencies.keys().cloned().collect();
    let loader = manifest.inner.project.loader_kind()?;
    Ok(list_output(
        instance_dir,
        &manifest.inner,
        &lock.inner,
        loader,
        None,
        roots,
    ))
}

/// Read installed packages selected for a client/server target.
///
/// Environment filters apply to manifest roots; their transitive dependencies
/// remain visible so the result describes an installable closure.
pub fn list_installed_for_target(
    instance_dir: &Path,
    target: &str,
) -> Result<ListOutput, OrbitError> {
    let manifest = ManifestFile::open(instance_dir)?;
    let lock = Lockfile::open(instance_dir)?;
    let options = RestoreOptions {
        target: Some(target.to_string()),
        ..RestoreOptions::default()
    };
    validate_restore_options(&options)?;
    let platform = crate::platform::Platform::load(instance_dir, &manifest.inner)?;
    let loader_package = platform.loader_package;
    let (selected, skipped) = selected_packages(
        &manifest.inner,
        &lock.inner,
        &options,
        loader_package.as_ref(),
    )?;
    let selected: std::collections::HashSet<_> = selected.into_iter().collect();
    let roots = manifest
        .inner
        .dependencies
        .keys()
        .filter(|package| !skipped.contains(package))
        .cloned()
        .collect();
    Ok(list_output(
        instance_dir,
        &manifest.inner,
        &lock.inner,
        platform.loader,
        Some(&selected),
        roots,
    ))
}

fn list_output(
    instance_dir: &Path,
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    loader: crate::loader::LoaderKind,
    selected: Option<&std::collections::HashSet<String>>,
    roots: Vec<String>,
) -> ListOutput {
    let packages: Vec<ListedPackage> = lockfile
        .packages
        .iter()
        .filter(|entry| selected.is_none_or(|selected| selected.contains(&entry.mod_id)))
        .map(|entry| {
            let requirement = manifest.dependencies.get(&entry.mod_id);
            let jar_path = instance_dir.join("mods").join(&entry.filename);
            let icon = jar_path
                .is_file()
                .then(|| {
                    crate::jar::read_mod_icon_entry(&jar_path, loader)
                        .ok()
                        .flatten()
                        .map(|archive_entry| PackageIcon {
                            jar_path,
                            archive_entry,
                            sha256: entry.sha256.clone(),
                        })
                })
                .flatten();
            ListedPackage {
                mod_id: entry.mod_id.clone(),
                version: entry.version.clone(),
                remotes: entry
                    .remotes
                    .iter()
                    .map(PackageRemote::display_locator)
                    .collect(),
                configured_environment: requirement
                    .and_then(DependencySpec::env)
                    .map(|environment| environment.as_str().to_string()),
                environment: requirement
                    .map(|requirement| requirement.effective_environment(entry.environment))
                    .unwrap_or(entry.environment)
                    .as_str()
                    .to_string(),
                root: requirement.is_some(),
                optional: requirement.is_some_and(DependencySpec::optional),
                dependencies: declared_dependency_ids(&entry.dependencies)
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                bundled: bundled_pairs(&entry.bundled),
                icon,
            }
        })
        .collect();
    ListOutput { packages, roots }
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
    target: InstallTarget,
    instance_dir: &'a Path,
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
        target,
        instance_dir,
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
    let InstallInteraction {
        select_package,
        select_resolution,
        confirm_install,
        progress,
    } = interaction;

    let requested_package = match &target {
        InstallTarget::Package(package) => Some(package.as_str()),
        InstallTarget::Remote(_) => None,
    };
    if options.intent == InstallIntent::Add && requested_package.is_some() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "add requires a package remote, not an installed package id"
        )));
    }
    if options.intent == InstallIntent::Upgrade && requested_package.is_none() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "upgrade requires an installed package id"
        )));
    }

    let loader = manifest.project.loader_kind()?;
    let mc_version = &manifest.project.mc_version;

    // 1-2. BFS download all JARs
    let requested_remotes: Vec<_> = match &target {
        InstallTarget::Remote(remote) => vec![remote.clone()],
        InstallTarget::Package(_) => Vec::new(),
    };
    let manifest_remotes: Vec<_> = manifest
        .dependencies
        .values()
        .flat_map(|dependency| dependency.remotes.iter().cloned())
        .collect();
    let mut catalog = crate::outdated::download_candidate_catalog(
        crate::outdated::CandidateDiscoveryInput {
            instance_dir,
            providers,
            additional_remotes: &manifest_remotes,
            lockfile,
            mc_version,
            loader,
            jar_cache,
            progress: progress.clone(),
        },
        &requested_remotes,
    )
    .await?;
    catalog.loader_package = loader_package;
    if catalog.candidates.is_empty() {
        return Err(OrbitError::ModNotFound(
            requested_package
                .map(str::to_string)
                .or_else(|| {
                    requested_remotes
                        .first()
                        .map(PackageRemote::display_locator)
                })
                .unwrap_or_default(),
        ));
    }
    let requested_requirement =
        requested_requirement(constraint, options.optional, options.env.as_deref())?;
    emit_progress(
        progress.as_ref(),
        ProgressEvent::ResolutionStarted {
            packages: catalog.candidates.len(),
            candidates: catalog.candidates.values().map(Vec::len).sum(),
        },
    );
    let (requested_package, portfolio) = resolve_requested_package(RequestedPackageInput {
        requested_package,
        intent: options.intent,
        manifest,
        lockfile,
        catalog: &catalog,
        requirement: requested_requirement.clone(),
        selector: select_package,
        progress: progress.clone(),
    })
    .await?;

    // 3. Resolve offline
    let resolution = if options.intent == InstallIntent::Upgrade {
        crate::resolver::select_upgrade_resolution(
            portfolio,
            Some(&requested_package),
            select_resolution,
        )
    } else {
        crate::resolver::select_resolution(portfolio, select_resolution)
    }
    .map_err(OrbitError::Conflict)?;
    if options.intent == InstallIntent::Upgrade && !resolution.has_upgrade() {
        return Ok(InstallReport {
            installed: Vec::new(),
            removed: Vec::new(),
            changes: Vec::new(),
            already_satisfied: Vec::new(),
            skipped_optional: Vec::new(),
            diagnostics: resolution.diagnostics,
            warnings: resolution.warnings,
        });
    }
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

        planned.push(plan_from_resolved(
            mod_id,
            new_ver,
            candidate_id,
            resolved,
            package_remotes(mod_id, manifest, lockfile, &catalog),
        ));
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

    if let Some(prompt) = confirm_install
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
        progress,
    )
    .await?;
    remove_packages(mods_dir, &removals, &installed)?;
    retain_selected_lock_entries(lockfile, &selected_sources);

    if installed
        .iter()
        .any(|installed| installed.mod_id == requested_package)
    {
        let mut requested_requirement = requested_requirement;
        requested_requirement.remotes =
            package_remotes(&requested_package, manifest, lockfile, &catalog);
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

struct RequestedPackageInput<'a> {
    requested_package: Option<&'a str>,
    intent: InstallIntent,
    manifest: &'a OrbitManifest,
    lockfile: &'a OrbitLockfile,
    catalog: &'a crate::resolver::types::CandidateCatalog,
    requirement: DependencySpec,
    selector: Option<PackageSelector>,
    progress: Option<ProgressReporter>,
}

async fn resolve_requested_package(
    input: RequestedPackageInput<'_>,
) -> Result<(String, crate::resolver::types::ResolutionPortfolio), OrbitError> {
    let RequestedPackageInput {
        requested_package,
        intent,
        manifest,
        lockfile,
        catalog,
        requirement,
        selector,
        progress,
    } = input;
    if intent == InstallIntent::Upgrade {
        let locator = requested_package.unwrap_or_default();
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
        let portfolio = crate::resolver::resolve_candidate_portfolio_with_progress(
            &resolution_manifest,
            lockfile,
            catalog,
            progress.clone(),
        )
        .await
        .map_err(OrbitError::Conflict)?;
        emit_progress(
            progress.as_ref(),
            ProgressEvent::ResolutionFinished {
                solutions: portfolio.alternatives.len(),
            },
        );
        return Ok((package, portfolio));
    }

    let packages: Vec<_> = catalog.requested_packages.iter().cloned().collect();
    if packages.is_empty() {
        return Err(OrbitError::ModNotFound(
            "requested remotes contain no JAR-declared package".to_string(),
        ));
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
        let mut package_requirement = requirement.clone();
        package_requirement.remotes = catalog.remotes_for_package(&package);
        ensure_root_requirement(&mut resolution_manifest, &package, package_requirement);
        match crate::resolver::resolve_candidate_portfolio_with_progress(
            &resolution_manifest,
            lockfile,
            catalog,
            progress.clone(),
        )
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
            "requested remotes contain no feasible JAR-declared package:\n{details}"
        )));
    }

    emit_progress(
        progress.as_ref(),
        ProgressEvent::ResolutionFinished {
            solutions: feasible
                .iter()
                .map(|(_, portfolio)| portfolio.alternatives.len())
                .sum(),
        },
    );
    let index = if feasible.len() == 1 {
        0
    } else {
        let package_names: Vec<_> = feasible
            .iter()
            .map(|(package, _)| package.clone())
            .collect();
        match selector {
            Some(select) => select(&package_names).map_err(OrbitError::Conflict)?,
            None => 0,
        }
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

struct MissingLockResolutionInput<'a> {
    instance_dir: &'a Path,
    manifest: &'a OrbitManifest,
    lockfile: &'a mut OrbitLockfile,
    providers: &'a [Box<dyn ModProvider>],
    jar_cache: &'a crate::jar_cache::JarCache,
    loader_package: Option<crate::resolver::types::PlatformCandidate>,
    selector: Option<ResolutionSelector>,
    progress: Option<ProgressReporter>,
}

async fn resolve_missing_lock_entries(
    input: MissingLockResolutionInput<'_>,
) -> Result<crate::resolver::types::ResolutionReport, OrbitError> {
    let MissingLockResolutionInput {
        instance_dir,
        manifest,
        lockfile,
        providers,
        jar_cache,
        loader_package,
        selector,
        progress,
    } = input;
    let manifest_remotes: Vec<_> = manifest
        .dependencies
        .values()
        .flat_map(|dependency| dependency.remotes.iter().cloned())
        .collect();
    let mut catalog = crate::outdated::download_candidate_catalog(
        crate::outdated::CandidateDiscoveryInput {
            instance_dir,
            providers,
            additional_remotes: &manifest_remotes,
            lockfile,
            mc_version: &manifest.project.mc_version,
            loader: manifest.project.loader_kind()?,
            jar_cache,
            progress: progress.clone(),
        },
        &[],
    )
    .await?;
    catalog.loader_package = loader_package;

    emit_progress(
        progress.as_ref(),
        ProgressEvent::ResolutionStarted {
            packages: catalog.candidates.len(),
            candidates: catalog.candidates.values().map(Vec::len).sum(),
        },
    );
    let portfolio = crate::resolver::resolve_candidate_portfolio_with_progress(
        manifest,
        lockfile,
        &catalog,
        progress.clone(),
    )
    .await
    .map_err(|error| OrbitError::Conflict(error.to_string()))?;
    emit_progress(
        progress.as_ref(),
        ProgressEvent::ResolutionFinished {
            solutions: portfolio.alternatives.len(),
        },
    );
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
            package,
            version,
            resolved,
            package_remotes(package, manifest, lockfile, &catalog),
            candidate,
        ));
    }
    lockfile
        .packages
        .sort_by(|left, right| left.mod_id.cmp(&right.mod_id));
    Ok(resolution)
}

fn package_remotes(
    package: &str,
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    catalog: &crate::resolver::types::CandidateCatalog,
) -> Vec<PackageRemote> {
    let mut remotes = catalog.remotes_for_package(package);
    if let Some(requirement) = manifest.dependencies.get(package) {
        remotes.extend(requirement.remotes.iter().cloned());
    }
    if let Some(entry) = lockfile.find(package) {
        remotes.extend(entry.remotes.iter().cloned());
    }
    remotes.sort();
    remotes.dedup();
    remotes
}

fn lock_entry_from_candidate(
    package: &str,
    version: &str,
    artifact: &crate::resolver::types::ResolvedArtifact,
    remotes: Vec<PackageRemote>,
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
        sha256: artifact.sha256.clone(),
        sha512: artifact.sha512.clone(),
        filename: artifact.filename.clone(),
        remotes,
        artifact_sources: artifact.sources.clone(),
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
        let declared_environment = lockfile
            .find(package)
            .map(|entry| entry.environment)
            .unwrap_or(crate::metadata::Environment::Both);
        let environment = spec.effective_environment(declared_environment);
        let environment_matches = match target {
            "client" => environment.applies_to(crate::metadata::Environment::Client),
            "server" => environment.applies_to(crate::metadata::Environment::Server),
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
    let resolved = crate::resolver::types::ResolvedArtifact {
        filename: package_filename(entry),
        sha1: entry.sha1.clone(),
        sha256: entry.sha256.clone(),
        sha512: entry.sha512.clone(),
        sources: entry.artifact_sources.clone(),
    };
    if locked && resolved.sources.is_empty() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "--locked: '{}' has no source for the selected artifact",
            entry.mod_id
        )));
    }
    let destination =
        materialize_resolved(&resolved, instance_dir, mods_dir, providers, jar_cache).await?;
    verify_package_hash(entry, &destination)?;
    entry.filename = resolved.filename;
    entry.sha1 = crate::jar::compute_sha1(&destination)?;
    entry.sha256 = crate::jar::compute_sha256(&destination)?;
    entry.sha512 = crate::jar::compute_sha512(&destination)?;
    Ok(())
}

async fn materialize_resolved(
    resolved: &crate::resolver::types::ResolvedArtifact,
    instance_dir: &Path,
    mods_dir: &Path,
    providers: &[Box<dyn ModProvider>],
    jar_cache: &crate::jar_cache::JarCache,
) -> Result<PathBuf, OrbitError> {
    let filename = safe_artifact_filename(&resolved.filename)?;
    let destination = mods_dir.join(filename);
    if destination.is_file()
        && crate::jar::compute_sha512(&destination)?.eq_ignore_ascii_case(&resolved.sha512)
    {
        return Ok(destination);
    }
    if jar_cache.copy_to(&resolved.sha512, &resolved.sha1, &destination)
        && crate::jar::compute_sha512(&destination)?.eq_ignore_ascii_case(&resolved.sha512)
    {
        return Ok(destination);
    }

    let mut errors = Vec::new();
    for source in &resolved.sources {
        let result = match source {
            ArtifactSource::File { path } => {
                let source_path = {
                    let path = Path::new(path);
                    if path.is_absolute() {
                        path.to_path_buf()
                    } else {
                        instance_dir.join(path)
                    }
                };
                std::fs::read(&source_path)
                    .map_err(OrbitError::Io)
                    .and_then(|bytes| {
                        verify_resolved_bytes(resolved, &bytes)?;
                        Ok(bytes)
                    })
            }
            ArtifactSource::Modrinth { download_url, .. } => {
                download_resolved_source("modrinth", download_url, resolved, providers).await
            }
            ArtifactSource::Curseforge { download_url, .. } => {
                download_resolved_source("curseforge", download_url, resolved, providers).await
            }
        };
        match result {
            Ok(bytes) => {
                jar_cache.store_bytes(&bytes)?;
                let temporary = destination.with_extension("jar.orbit-tmp");
                std::fs::write(&temporary, bytes)?;
                if destination.exists() {
                    std::fs::remove_file(&destination)?;
                }
                std::fs::rename(temporary, &destination)?;
                return Ok(destination);
            }
            Err(error) => errors.push(format!("{}: {error}", source.provider())),
        }
    }

    Err(OrbitError::Other(anyhow::anyhow!(
        "no source could restore '{}':\n  - {}",
        resolved.filename,
        errors.join("\n  - ")
    )))
}

async fn download_resolved_source(
    provider_name: &str,
    download_url: &str,
    resolved: &crate::resolver::types::ResolvedArtifact,
    providers: &[Box<dyn ModProvider>],
) -> Result<Vec<u8>, OrbitError> {
    if download_url.is_empty() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "selected source has no download URL"
        )));
    }
    let provider = crate::providers::find_provider(providers, provider_name).ok_or_else(|| {
        OrbitError::Other(anyhow::anyhow!(
            "{provider_name} provider is required to restore '{}'",
            resolved.filename
        ))
    })?;
    let bytes = provider
        .artifact_downloader()
        .download(download_url, &resolved.filename)
        .await?;
    verify_resolved_bytes(resolved, &bytes)?;
    Ok(bytes)
}

fn verify_resolved_bytes(
    resolved: &crate::resolver::types::ResolvedArtifact,
    bytes: &[u8],
) -> Result<(), OrbitError> {
    let actual = crate::jar::sha512_digest(bytes);
    if actual.eq_ignore_ascii_case(&resolved.sha512) {
        Ok(())
    } else {
        Err(OrbitError::ChecksumMismatch {
            name: resolved.filename.clone(),
            expected: resolved.sha512.clone(),
            actual,
        })
    }
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
        .artifact_sources
        .iter()
        .find_map(|source| match source {
            ArtifactSource::File { path } => Path::new(path).file_name(),
            _ => None,
        })
        .map(|filename| filename.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn plan_from_resolved(
    mod_id: &str,
    version: &str,
    candidate_id: &str,
    artifact: &crate::resolver::types::ResolvedArtifact,
    remotes: Vec<PackageRemote>,
) -> InstalledMod {
    InstalledMod {
        candidate_id: Some(candidate_id.to_string()),
        mod_id: mod_id.to_string(),
        version: version.to_string(),
        filename: artifact.filename.clone(),
        remotes,
        artifact_sources: artifact.sources.clone(),
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
    loader: crate::loader::LoaderKind,
    providers: &[Box<dyn ModProvider>],
    jar_cache: &crate::jar_cache::JarCache,
    progress: Option<ProgressReporter>,
) -> Result<Vec<InstalledMod>, OrbitError> {
    let total = planned.len();
    let instance_dir = mods_dir.parent().unwrap_or_else(|| Path::new("."));
    emit_progress(progress.as_ref(), ProgressEvent::ApplyStarted { total });
    let mut installed = Vec::new();
    for mut plan in planned {
        emit_progress(
            progress.as_ref(),
            ProgressEvent::ApplyArtifact {
                completed: installed.len(),
                total,
                filename: plan.filename.clone(),
                state: ArtifactProgressState::Started,
            },
        );
        let candidate_id = plan.candidate_id.as_deref().ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!(
                "remote install plan for '{}' has no candidate identity",
                plan.mod_id
            ))
        })?;
        let resolved = resolved_candidate(resolved_candidates, candidate_id)?;
        let dest_path =
            materialize_resolved(resolved, instance_dir, mods_dir, providers, jar_cache).await?;
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
        emit_progress(
            progress.as_ref(),
            ProgressEvent::ApplyArtifact {
                completed: installed.len(),
                total,
                filename: installed
                    .last()
                    .map(|package| package.filename.clone())
                    .unwrap_or_default(),
                state: ArtifactProgressState::Finished,
            },
        );
    }
    emit_progress(progress.as_ref(), ProgressEvent::ApplyFinished { total });
    Ok(installed)
}

fn resolved_candidate<'a>(
    candidates: &'a crate::resolver::types::ResolvedCandidates,
    candidate_id: &str,
) -> Result<&'a crate::resolver::types::ResolvedArtifact, OrbitError> {
    candidates.get(candidate_id).ok_or_else(|| {
        OrbitError::Other(anyhow::anyhow!(
            "solver selected a package candidate without a materializable artifact"
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
            remotes: inst.remotes.clone(),
            artifact_sources: inst.artifact_sources.clone(),
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
    let env = env
        .map(str::parse)
        .transpose()
        .map_err(|error: String| OrbitError::Other(anyhow::anyhow!(error)))?;
    let version = if constraint.is_empty() {
        "*".to_string()
    } else {
        constraint.to_string()
    };
    Ok(DependencySpec {
        version,
        optional,
        env,
        exclude: Vec::new(),
        remotes: Vec::new(),
    })
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
    use crate::providers::RemoteArtifact;
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
runtime_jars = []
physical_environment = "client"
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
            remotes: vec![PackageRemote::File {
                path: format!("mods/{mod_id}.jar"),
            }],
            artifact_sources: vec![ArtifactSource::File {
                path: format!("mods/{mod_id}.jar"),
            }],
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
            .record_test(
                jar_metadata("gca-wrapper", "1.0.1", &[]),
                remote_artifact("old"),
            )
            .unwrap();
        catalog
            .record_test(
                jar_metadata("gca_wrapper", "1.0.6", &[]),
                remote_artifact("new"),
            )
            .unwrap();

        let (package, portfolio) = resolve_requested_package(RequestedPackageInput {
            requested_package: None,
            intent: InstallIntent::Add,
            manifest: &manifest(),
            lockfile: &empty_lockfile(),
            catalog: &catalog,
            requirement: DependencySpec::new("*", Vec::new()),
            selector: Some(Box::new(|packages| {
                Ok(packages
                    .iter()
                    .position(|package| package == "gca_wrapper")
                    .unwrap())
            })),
            progress: None,
        })
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
            .record_test(
                jar_metadata("gca-wrapper", "1.0.1", &["missing"]),
                remote_artifact("old"),
            )
            .unwrap();
        catalog
            .record_test(
                jar_metadata("gca_wrapper", "1.0.6", &[]),
                remote_artifact("new"),
            )
            .unwrap();
        let prompted = Arc::new(AtomicBool::new(false));
        let captured = Arc::clone(&prompted);

        let (package, _) = resolve_requested_package(RequestedPackageInput {
            requested_package: None,
            intent: InstallIntent::Add,
            manifest: &manifest(),
            lockfile: &empty_lockfile(),
            catalog: &catalog,
            requirement: DependencySpec::new("*", Vec::new()),
            selector: Some(Box::new(move |_| {
                captured.store(true, Ordering::Relaxed);
                Ok(0)
            })),
            progress: None,
        })
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

        assert_eq!(requirement.version, "^1");
        assert!(requirement.optional);
        assert_eq!(requirement.env, Some(crate::metadata::Environment::Client));
        assert!(requirement.exclude.is_empty());
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
runtime_jars = []
physical_environment = "client"

[dependencies]
client-mod = { version = "*", env = "client", remotes = [{ type = "file", path = "client.jar" }] }
server-mod = { version = "*", env = "server", remotes = [{ type = "file", path = "server.jar" }] }
optional-mod = { version = "*", optional = true, remotes = [{ type = "file", path = "optional.jar" }] }

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
    fn missing_manifest_environment_follows_the_locked_jar_declaration() {
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
runtime_jars = []
physical_environment = "client"
[dependencies]
example = { version = "*", remotes = [{ type = "file", path = "example.jar" }] }
"#,
        )
        .unwrap();
        let mut package = locked_package("example", &[]);
        package.environment = crate::metadata::Environment::Client;
        let lockfile = OrbitLockfile {
            meta: LockMeta {
                mc_version: "1".to_string(),
                modloader: "fabric".to_string(),
                modloader_version: "1".to_string(),
            },
            packages: vec![package],
        };

        let client = RestoreOptions {
            target: Some("client".to_string()),
            ..RestoreOptions::default()
        };
        let server = RestoreOptions {
            target: Some("server".to_string()),
            ..RestoreOptions::default()
        };

        assert_eq!(
            selected_packages(&manifest, &lockfile, &client, None).unwrap(),
            (vec!["example".to_string()], Vec::new())
        );
        assert_eq!(
            selected_packages(&manifest, &lockfile, &server, None).unwrap(),
            (Vec::new(), vec!["example".to_string()])
        );
    }

    #[test]
    fn explicit_manifest_environment_overrides_the_locked_jar_filter() {
        let mut manifest: OrbitManifest = toml::from_str(
            r#"
[project]
name = "test"
mc_version = "1"
modloader = "fabric"
modloader_version = "1"
[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }
runtime_jars = []
physical_environment = "client"
[dependencies]
example = { version = "*", remotes = [{ type = "file", path = "example.jar" }] }
"#,
        )
        .unwrap();
        manifest.dependencies["example"].env = Some(crate::metadata::Environment::Both);
        let mut package = locked_package("example", &[]);
        package.environment = crate::metadata::Environment::Client;
        let lockfile = OrbitLockfile {
            meta: LockMeta {
                mc_version: "1".to_string(),
                modloader: "fabric".to_string(),
                modloader_version: "1".to_string(),
            },
            packages: vec![package],
        };
        let server = RestoreOptions {
            target: Some("server".to_string()),
            ..RestoreOptions::default()
        };

        assert!(selected_packages(&manifest, &lockfile, &server, None).is_err());
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
runtime_jars = []
physical_environment = "client"
[dependencies]
example = { version = "*", remotes = [{ type = "file", path = "example.jar" }] }
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
