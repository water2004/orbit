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
use crate::workspace::{Lockfile, ManifestFile};

pub type InstallPrompt = Box<dyn FnOnce(&InstallReport) -> bool + Send>;

#[derive(Debug, Clone, Copy, Default)]
pub struct InstallOptions {
    pub no_deps: bool,
    pub dry_run: bool,
    pub existing_ok: bool,
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
    if !mods_dir.exists() && !options.dry_run {
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

    if !options.dry_run && !report.installed.is_empty() {
        manifest_file.save()?;
        lock.save()?;
    }

    Ok(report)
}

/// 升级实例中所有过期模组
pub async fn upgrade_all_in_instance(
    instance_dir: &Path,
    providers: &[Box<dyn ModProvider>],
    dry_run: bool,
    prompt_fn: Option<InstallPrompt>,
) -> Result<InstallReport, OrbitError> {
    let mut manifest_file = ManifestFile::open(instance_dir)?;
    let mut lock = Lockfile::open_or_default(
        instance_dir,
        LockMeta {
            mc_version: manifest_file.inner.project.mc_version.clone(),
            modloader: manifest_file.inner.project.modloader.clone(),
            modloader_version: manifest_file.inner.project.modloader_version.clone(),
        },
    );

    let (outdated, resolved_candidates) =
        crate::outdated::check_all_outdated(&manifest_file.inner, &lock.inner, providers).await?;

    if outdated.is_empty() {
        return Ok(InstallReport {
            installed: vec![],
            already_satisfied: vec![],
            skipped_optional: vec![],
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
        planned.push(InstalledMod {
            slug: resolved.slug.clone(),
            mod_id: o.mod_id.clone(),
            version: o.new_version.clone(),
            filename: resolved.filename.clone(),
            provider: resolved.provider.clone(),
            project_id: resolved
                .modrinth
                .as_ref()
                .map(|mr| mr.project_id.clone())
                .unwrap_or_default(),
            version_id: resolved
                .modrinth
                .as_ref()
                .map(|mr| mr.version_id.clone())
                .unwrap_or_default(),
            modrinth_version: resolved
                .modrinth
                .as_ref()
                .map(|mr| mr.version_number.clone())
                .unwrap_or_default(),
            jar_deps: vec![],
            implanted: vec![],
        });
    }

    let report = InstallReport {
        installed: planned.clone(),
        already_satisfied: vec![],
        skipped_optional: vec![],
    };

    if let Some(prompt) = prompt_fn
        && !prompt(&report)
    {
        return Ok(InstallReport {
            installed: vec![],
            already_satisfied: vec![],
            skipped_optional: vec![],
        }); // aborted
    }

    if dry_run {
        return Ok(report);
    }

    let mut installed = Vec::new();
    for mut plan in planned {
        let key = (plan.mod_id.clone(), plan.version.clone());
        let Some(resolved) = resolved_candidates.get(&key).cloned() else {
            continue;
        };
        let dest_path = download_mod(&resolved, &mods_dir).await?;
        let meta = crate::jar::read_mod_metadata(&dest_path, loader)?;

        plan.mod_id = if meta.mod_id.is_empty() {
            plan.mod_id
        } else {
            meta.mod_id
        };
        plan.version = if meta.version.is_empty() {
            plan.version
        } else {
            meta.version
        };
        plan.jar_deps = meta.dependencies;
        plan.implanted = meta
            .implanted_mods
            .into_iter()
            .map(|im| crate::lockfile::ImplantedMod {
                name: if !im.mod_id.is_empty() {
                    im.mod_id
                } else {
                    im.name
                },
                version: im.version,
                sha256: String::new(),
                filename: String::new(),
                dependencies: im
                    .dependencies
                    .into_iter()
                    .filter(|(n, _, req)| {
                        *req && n != "java"
                            && n != "mixinextras"
                            && n != "minecraft"
                            && n != "fabricloader"
                    })
                    .map(|(name, version, _)| crate::lockfile::LockDependency { name, version })
                    .collect(),
            })
            .collect();
        installed.push(plan);
    }

    apply_to_manifest_and_lock(
        &mut manifest_file.inner,
        &mut lock.inner,
        &installed,
        &mods_dir,
    );

    if !installed.is_empty() {
        manifest_file.save()?;
        lock.save()?;
    }

    Ok(InstallReport {
        installed,
        already_satisfied: vec![],
        skipped_optional: vec![],
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
    } = crate::outdated::download_candidates_bfs(
        providers[0].as_ref(),
        &seeds,
        lockfile,
        mc_version,
        loader,
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
    ensure_root_requirement(&mut resolution_manifest, &requested_package, constraint);

    // 3. Resolve offline
    eprintln!(
        "  resolving with {} mod(s) in candidates...",
        candidates.len()
    );
    let upgrades = match crate::resolver::resolve_with_candidates(
        &resolution_manifest,
        lockfile,
        &mut candidates,
        providers,
    )
    .await
    {
        Ok(u) => {
            eprintln!("  resolved: {:?}", u);
            u
        }
        Err(e) => return Err(OrbitError::Conflict(e)),
    };

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

        planned.push(InstalledMod {
            slug: resolved.slug.clone(),
            mod_id: mod_id.clone(),
            version: new_ver.clone(),
            filename: resolved.filename.clone(),
            provider: resolved.provider.clone(),
            project_id: resolved
                .modrinth
                .as_ref()
                .map(|mr| mr.project_id.clone())
                .unwrap_or_default(),
            version_id: resolved
                .modrinth
                .as_ref()
                .map(|mr| mr.version_id.clone())
                .unwrap_or_default(),
            modrinth_version: resolved
                .modrinth
                .as_ref()
                .map(|mr| mr.version_number.clone())
                .unwrap_or_default(),
            jar_deps: vec![],
            implanted: vec![],
        });
    }

    let report = InstallReport {
        installed: planned.clone(),
        already_satisfied: already_satisfied.clone(),
        skipped_optional: vec![],
    };

    if let Some(prompt) = prompt_fn
        && !prompt(&report)
    {
        return Ok(InstallReport {
            installed: vec![],
            already_satisfied,
            skipped_optional: vec![],
        }); // aborted
    }

    if options.dry_run {
        return Ok(report);
    }

    let mut installed = Vec::new();
    for mut plan in planned {
        // 升级时删旧 JAR
        if options.existing_ok
            && let Some(old) = lockfile.find(&plan.mod_id)
            && !old.filename.is_empty()
        {
            let _ = std::fs::remove_file(mods_dir.join(&old.filename));
        }
        let key = (plan.mod_id.clone(), plan.version.clone());
        let Some(resolved) = resolved.get(&key) else {
            continue;
        };
        let dest_path = download_mod(resolved, mods_dir).await?;
        let meta = crate::jar::read_mod_metadata(&dest_path, loader)?;

        plan.mod_id = if meta.mod_id.is_empty() {
            plan.mod_id
        } else {
            meta.mod_id
        };
        plan.version = if meta.version.is_empty() {
            plan.version
        } else {
            meta.version
        };
        plan.jar_deps = meta.dependencies;
        plan.implanted = meta
            .implanted_mods
            .into_iter()
            .map(|im| crate::lockfile::ImplantedMod {
                name: if !im.mod_id.is_empty() {
                    im.mod_id
                } else {
                    im.name
                },
                version: im.version,
                sha256: String::new(),
                filename: String::new(),
                dependencies: im
                    .dependencies
                    .into_iter()
                    .filter(|(n, _, req)| {
                        *req && n != "java"
                            && n != "mixinextras"
                            && n != "minecraft"
                            && n != "fabricloader"
                    })
                    .map(|(name, version, _)| crate::lockfile::LockDependency { name, version })
                    .collect(),
            })
            .collect();
        installed.push(plan);
    }

    if installed
        .iter()
        .any(|installed| installed.mod_id == requested_package)
    {
        ensure_root_requirement(manifest, &requested_package, constraint);
    }
    apply_to_manifest_and_lock(manifest, lockfile, &installed, mods_dir);

    Ok(InstallReport {
        installed,
        already_satisfied,
        skipped_optional: vec![],
    })
}

// ── download / jar / manifest helpers ─────────────────────────────────

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
    let bytes = client
        .get(&m.download_url)
        .send()
        .await
        .map_err(OrbitError::Network)?
        .bytes()
        .await
        .map_err(OrbitError::Network)?;
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
    std::fs::rename(&tmp_path, &final_path).map_err(OrbitError::Io)?;
    Ok(final_path)
}

fn apply_to_manifest_and_lock(
    manifest: &mut OrbitManifest,
    lockfile: &mut OrbitLockfile,
    installed: &[InstalledMod],
    mods_dir: &Path,
) {
    for inst in installed {
        let key = &inst.mod_id;
        ensure_root_requirement(manifest, key, &inst.version);
        lockfile.packages.retain(|e| e.mod_id != *key);
        let lock_deps: Vec<LockDependency> = inst
            .jar_deps
            .iter()
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
        let sha256 = crate::jar::compute_sha256(&jar_path).unwrap_or_default();
        let sha512 = crate::jar::compute_sha512(&jar_path).unwrap_or_default();
        lockfile.packages.push(PackageEntry {
            mod_id: key.clone(),
            version: inst.version.clone(),
            sha1: String::new(),
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

fn ensure_root_requirement(manifest: &mut OrbitManifest, package: &str, constraint: &str) {
    manifest
        .dependencies
        .entry(package.to_string())
        .or_insert_with(|| {
            DependencySpec::Short(if constraint.is_empty() {
                "*".to_string()
            } else {
                constraint.to_string()
            })
        });
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

    #[test]
    fn requested_constraint_is_bound_to_the_actual_package_id() {
        let mut manifest = manifest();

        ensure_root_requirement(&mut manifest, "actual-mod-id", "^1");

        assert_eq!(
            manifest.dependencies["actual-mod-id"].version_constraint(),
            Some("^1")
        );
    }

    #[test]
    fn installed_version_does_not_replace_an_existing_requirement() {
        let mut manifest = manifest();
        ensure_root_requirement(&mut manifest, "example", "^1");

        ensure_root_requirement(&mut manifest, "example", "1.5");

        assert_eq!(
            manifest.dependencies["example"].version_constraint(),
            Some("^1")
        );
    }
}
