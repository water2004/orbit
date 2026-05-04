//! 模组安装 / 卸载逻辑。
//!
//! 提供顶层 API 供 CLI 调用。CLI 层不直接操作 TOML / 文件。

use std::path::{Path, PathBuf};

use crate::error::OrbitError;
use crate::lockfile::{LockDependency, LockMeta, OrbitLockfile, PackageEntry, ModrinthInfo, FileInfo};
use crate::manifest::{DependencySpec, OrbitManifest};
use crate::providers::{ModProvider, ResolvedMod};
use crate::workspace::{ManifestFile, Lockfile};

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
}

/// 顶层 API：在指定实例目录安装模组。
///
/// 接收 `instance_dir`，内部完成 orbit.toml / orbit.lock 的读写和 mods/ 目录管理。
/// `dry_run` 为 true 时仅解析不下载不写文件。
pub async fn install_to_instance(
    slug: &str,
    constraint: &str,
    instance_dir: &Path,
    providers: &[Box<dyn ModProvider>],
    no_deps: bool,
    dry_run: bool,
) -> Result<InstallReport, OrbitError> {
    let mut manifest_file = ManifestFile::open(instance_dir)?;
    let mut lock = Lockfile::open_or_default(instance_dir, LockMeta {
        mc_version: manifest_file.inner.project.mc_version.clone(),
        modloader: manifest_file.inner.project.modloader.clone(),
        modloader_version: manifest_file.inner.project.modloader_version.clone(),
    });

    let mods_dir = instance_dir.join("mods");
    if !mods_dir.exists() && !dry_run {
        std::fs::create_dir_all(&mods_dir).map_err(OrbitError::Io)?;
    }

    let report = install_mod(slug, constraint, &providers[..], &mut manifest_file.inner, &mut lock.inner, &mods_dir, no_deps, false, dry_run).await?;

    if !dry_run {
        manifest_file.save()?;
        lock.save()?;
    }

    Ok(report)
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
    manifest_file.inner.dependencies.swap_remove(&key)
        .expect("dependency entry should exist");

    let dependents = crate::resolver::dependents(&key, &lock.inner.packages);
    if !dependents.is_empty() {
        return Err(OrbitError::Conflict(format!(
            "'{key}' is required by: {}\nRemove those mods first.",
            dependents.join(", ")
        )));
    }

    let mods_dir = instance_dir.join("mods");
    let filename: Option<String> = lock.inner.packages.iter()
        .find(|e| e.mod_id == key)
        .and_then(|e| {
            e.file.as_ref().map(|f| {
                std::path::Path::new(&f.path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()
            })
        })
        .or_else(|| {
            if let Ok(dir_entries) = std::fs::read_dir(&mods_dir) {
                dir_entries
                    .filter_map(|e| e.ok())
                    .find(|e| {
                        let name = e.file_name().to_string_lossy().to_string();
                        e.path().extension().map(|ext| ext == "jar").unwrap_or(false)
                            && name.contains(key.as_str())
                    })
                    .map(|e| e.file_name().to_string_lossy().to_string())
            } else {
                None
            }
        });

    if let Some(ref fname) = filename {
        let jar_path = mods_dir.join(fname);
        if jar_path.exists() && !dry_run {
            std::fs::remove_file(&jar_path).map_err(OrbitError::Io)?;
        }
    }

    lock.inner.packages.retain(|e| e.mod_id != key);

    if !dry_run {
        manifest_file.save()?;
        lock.save()?;
    }

    Ok(RemoveReport {
        mod_id: key,
        jar_deleted: filename.is_some(),
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
    Ok(manifest_file.inner.dependencies.iter().map(|(k, _)| {
        let slug = lock.as_ref()
            .and_then(|l| l.find(k))
            .and_then(|e| e.modrinth.as_ref())
            .map(|m| m.slug.clone())
            .unwrap_or_else(|| k.clone());
        (k.clone(), slug)
    }).collect())
}

// ── 内部实现 ──────────────────────────────────────────────────────────

async fn install_mod(
    slug: &str,
    constraint: &str,
    providers: &[Box<dyn ModProvider>],
    manifest: &mut OrbitManifest,
    lockfile: &mut OrbitLockfile,
    mods_dir: &Path,
    no_deps: bool,
    existing_ok: bool,
    dry_run: bool,
) -> Result<InstallReport, OrbitError> {
    if !existing_ok && crate::resolver::find_entry(slug, &lockfile.packages).is_some() {
        return Err(OrbitError::Conflict(format!(
            "'{slug}' already in lockfile. Use 'orbit upgrade {slug}' to update it."
        )));
    }

    // 1. 快速检查 slug 是否存在（任一 provider 能找到即可）
    let mc_version = manifest.project.mc_version.clone();
    let loader = manifest.project.modloader.clone();
    let mut found = false;
    for p in providers {
        if let Ok(versions) = p.get_versions(slug, Some(&mc_version), Some(&loader)).await {
            if !versions.is_empty() { found = true; break; }
        }
    }
    if !found {
        return Err(OrbitError::ModNotFound(slug.to_string()));
    }

    // 2. Update manifest temporarily
    let old_dep = manifest.dependencies.insert(
        slug.to_string(),
        DependencySpec::Short(constraint.to_string())
    );

    // 3. Resolve manifest using PubGrub
    let solution = match crate::resolver::resolve_manifest(manifest, lockfile, providers).await {
        Ok(s) => s,
        Err(e) => {
            if let Some(old) = old_dep {
                manifest.dependencies.insert(slug.to_string(), old);
            } else {
                manifest.dependencies.swap_remove(slug);
            }
            return Err(OrbitError::Conflict(e));
        }
    };

    let mut to_install = Vec::new();
    let mut already_satisfied = Vec::new();

    for (pkg, resolved_mod) in solution {
        if let Some(existing) = crate::resolver::find_entry(&pkg, &lockfile.packages) {
            let needs_update = existing.modrinth.as_ref().map(|m| m.version_id.is_empty()).unwrap_or(false);
            if !needs_update && existing.version == resolved_mod.version {
                already_satisfied.push(pkg.clone());
                continue;
            }
        }
        if no_deps && pkg != slug {
            continue;
        }
        to_install.push(resolved_mod);
    }

    let mut installed = Vec::new();
    if !dry_run {
        for m in &to_install {
            let dest_path = download_mod(m, mods_dir).await?;
            let meta = crate::jar::read_mod_metadata(&dest_path, &manifest.project.modloader)?;
            let jar_mod_id = meta.mod_id;
            let jar_version = meta.version;
            let jar_deps = meta.dependencies;
            installed.push(InstalledMod {
                slug: m.slug.clone(),
                mod_id: if jar_mod_id.is_empty() { m.mod_id.clone() } else { jar_mod_id },
                version: if jar_version.is_empty() { m.version.clone() } else { jar_version },
                filename: m.filename.clone(), provider: m.provider.clone(),
                project_id: m.modrinth.as_ref().map(|mr| mr.project_id.clone()).unwrap_or_default(),
                version_id: m.modrinth.as_ref().map(|mr| mr.version_id.clone()).unwrap_or_default(),
                modrinth_version: m.modrinth.as_ref().map(|mr| mr.version_number.clone()).unwrap_or_default(),
                jar_deps,
            });
        }
        apply_to_manifest_and_lock(manifest, lockfile, &installed, mods_dir);
    } else {
        for m in &to_install {
            installed.push(InstalledMod {
                slug: m.slug.clone(), mod_id: m.mod_id.clone(), version: m.version.clone(),
                filename: m.filename.clone(), provider: m.provider.clone(),
                project_id: m.modrinth.as_ref().map(|mr| mr.project_id.clone()).unwrap_or_default(),
                version_id: m.modrinth.as_ref().map(|mr| mr.version_id.clone()).unwrap_or_default(),
                modrinth_version: m.modrinth.as_ref().map(|mr| mr.version_number.clone()).unwrap_or_default(),
                jar_deps: vec![],
            });
        }
    }

    Ok(InstallReport { installed, already_satisfied, skipped_optional: vec![] })
}

// ── download / jar / manifest helpers ─────────────────────────────────

async fn download_mod(m: &ResolvedMod, mods_dir: &Path) -> Result<PathBuf, OrbitError> {
    let final_path = mods_dir.join(&m.filename);
    if final_path.exists() {
        if !m.sha512.is_empty() {
            let existing_sha = crate::jar::compute_sha512(&final_path).unwrap_or_default();
            if existing_sha == m.sha512 { return Ok(final_path); }
        } else {
            let meta = std::fs::metadata(&final_path).map_err(OrbitError::Io)?;
            if meta.len() > 0 { return Ok(final_path); }
        }
    }
    let client = download_client();
    let bytes = client.get(&m.download_url).send().await.map_err(OrbitError::Network)?.bytes().await.map_err(OrbitError::Network)?;
    if !m.sha512.is_empty() {
        let actual = crate::jar::sha512_digest(&bytes);
        if actual != m.sha512 {
            return Err(OrbitError::ChecksumMismatch { name: m.filename.clone(), expected: m.sha512.clone(), actual });
        }
    }
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
        manifest.dependencies.insert(key.clone(),
            DependencySpec::Short(if inst.version.is_empty() { "*".into() } else { inst.version.clone() })
        );
        lockfile.packages.retain(|e| e.mod_id != *key);
        let lock_deps: Vec<LockDependency> = inst.jar_deps.iter().map(|(dep_id, constraint, _)| LockDependency {
            name: dep_id.clone(),
            version: if constraint.is_empty() { "*".into() } else { constraint.clone() },
        }).collect();
        let jar_path = mods_dir.join(&inst.filename);
        let sha256 = crate::jar::compute_sha256(&jar_path).unwrap_or_default();
        let sha512 = crate::jar::compute_sha512(&jar_path).unwrap_or_default();
        lockfile.packages.push(PackageEntry {
            mod_id: key.clone(),
            version: inst.version.clone(),
            sha1: String::new(),
            sha256,
            sha512,
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
                Some(FileInfo { path: format!("mods/{}", inst.filename) })
            } else {
                None
            },
            dependencies: lock_deps,
            implanted: vec![],
        });
    }
}
