//! 模组安装 / 卸载逻辑。
//!
//! 提供顶层 API 供 CLI 调用。CLI 层不直接操作 TOML / 文件。

use std::path::{Path, PathBuf};

use crate::error::OrbitError;
use crate::lockfile::{LockDependency, LockEntry, OrbitLockfile};
use crate::manifest::{DependencySpec, OrbitManifest};
use crate::providers::{ModProvider, ResolvedMod};

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
    pub key: String,
    pub version: String,
    pub filename: String,
    pub provider: String,
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
    let toml_path = instance_dir.join("orbit.toml");
    if !toml_path.exists() {
        return Err(OrbitError::ManifestNotFound);
    }
    let mut manifest = OrbitManifest::from_path(&toml_path)?;

    let lock_path = instance_dir.join("orbit.lock");
    let mut lockfile = if lock_path.exists() {
        OrbitLockfile::from_path(&lock_path)?
    } else {
        OrbitLockfile {
            meta: crate::lockfile::LockMeta {
                mc_version: manifest.project.mc_version.clone(),
                modloader: manifest.project.modloader.clone(),
                modloader_version: manifest.project.modloader_version.clone(),
            },
            entries: vec![],
        }
    };

    let mods_dir = instance_dir.join("mods");
    if !mods_dir.exists() && !dry_run {
        std::fs::create_dir_all(&mods_dir).map_err(OrbitError::Io)?;
    }

    // 按 provider 顺序尝试解析
    let report = install_mod(slug, constraint, &providers[..], &mut manifest, &mut lockfile, &mods_dir, no_deps, false, dry_run).await?;

    if !dry_run {
        std::fs::write(&toml_path, manifest.to_toml_string()?).map_err(OrbitError::Io)?;
        std::fs::write(&lock_path, lockfile.to_toml_string()?).map_err(OrbitError::Io)?;
    }

    Ok(report)
}

/// 顶层 API：从指定实例目录移除模组。
pub fn remove_from_instance(
    input: &str,
    instance_dir: &Path,
    dry_run: bool,
) -> Result<RemoveReport, OrbitError> {
    let toml_path = instance_dir.join("orbit.toml");
    if !toml_path.exists() {
        return Err(OrbitError::ManifestNotFound);
    }
    let mut manifest = OrbitManifest::from_path(&toml_path)?;

    // 查找依赖
    let key = find_by_slug(input, &manifest)
        .ok_or_else(|| OrbitError::ModNotFound(input.to_string()))?;

    let spec = manifest.dependencies.swap_remove(&key)
        .expect("dependency entry should exist");

    let lock_path = instance_dir.join("orbit.lock");
    let mut lockfile = if lock_path.exists() {
        OrbitLockfile::from_path(&lock_path)?
    } else {
        return Err(OrbitError::LockfileNotFound);
    };

    // 反查依赖图
    let slug = spec.slug().unwrap_or(&key);
    let dependents = crate::resolver::dependents(slug, &lockfile.entries);
    if !dependents.is_empty() {
        return Err(OrbitError::Conflict(format!(
            "'{key}' is required by: {}\nRemove those mods first.",
            dependents.join(", ")
        )));
    }

    // 找到 lock 条目
    let filename = lockfile.entries.iter()
        .find(|e| e.name == key || e.mod_id.as_deref() == Some(slug))
        .map(|e| e.filename.clone());

    // 删除 JAR
    if let Some(ref fname) = filename {
        let jar_path = instance_dir.join("mods").join(fname);
        if jar_path.exists() && !dry_run {
            std::fs::remove_file(&jar_path).map_err(OrbitError::Io)?;
        }
    }

    lockfile.entries.retain(|e| e.name != key);

    if !dry_run {
        std::fs::write(&toml_path, manifest.to_toml_string()?).map_err(OrbitError::Io)?;
        std::fs::write(&lock_path, lockfile.to_toml_string()?).map_err(OrbitError::Io)?;
    }

    Ok(RemoveReport {
        key,
        jar_deleted: filename.is_some(),
    })
}

#[derive(Debug, Clone)]
pub struct RemoveReport {
    pub key: String,
    pub jar_deleted: bool,
}

/// 列出实例中所有依赖（供 remove 找不到时交互选择）
pub fn list_dependencies(instance_dir: &Path) -> Result<Vec<(String, String)>, OrbitError> {
    let manifest = OrbitManifest::from_path(&instance_dir.join("orbit.toml"))?;
    Ok(manifest.dependencies.iter().map(|(k, spec)| {
        (k.clone(), spec.slug().unwrap_or(k).to_string())
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
    if !existing_ok && crate::resolver::find_entry(slug, &lockfile.entries).is_some() {
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
        if let Some(entry) = crate::resolver::find_entry(&pkg, &lockfile.entries) {
            if entry.version == resolved_mod.version {
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
            let jar_deps = parse_jar_deps(&dest_path)?;
            installed.push(InstalledMod {
                slug: m.name.clone(), key: m.name.clone(), version: m.version.clone(),
                filename: m.filename.clone(), provider: "modrinth".to_string(), jar_deps,
            });
        }
        apply_to_manifest_and_lock(manifest, lockfile, &installed, mods_dir);
    } else {
        for m in &to_install {
            installed.push(InstalledMod {
                slug: m.name.clone(), key: m.name.clone(), version: m.version.clone(),
                filename: m.filename.clone(), provider: "modrinth".to_string(), jar_deps: vec![],
            });
        }
    }

    Ok(InstallReport { installed, already_satisfied, skipped_optional: vec![] })
}

fn find_by_slug(name: &str, manifest: &OrbitManifest) -> Option<String> {
    manifest.dependencies.iter().find_map(|(key, spec)| {
        if key == name || spec.slug() == Some(name) { Some(key.clone()) } else { None }
    })
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

fn parse_jar_deps(jar_path: &Path) -> Result<Vec<(String, String, bool)>, OrbitError> {
    let file = std::fs::File::open(jar_path).map_err(OrbitError::Io)?;
    let mut archive = zip::ZipArchive::new(file).map_err(OrbitError::Zip)?;
    match archive.by_name("fabric.mod.json") {
        Ok(mut entry) => {
            let mut content = String::new();
            std::io::Read::read_to_string(&mut entry, &mut content).map_err(|e| OrbitError::Io(e.into()))?;
            let parser = crate::metadata::fabric::FabricParser;
            let meta = crate::metadata::MetadataParser::parse(&parser, &content)?;
            Ok(meta.dependencies.into_iter().map(|(k, v)| (k, v, true)).collect())
        }
        Err(_) => Ok(vec![]),
    }
}

fn apply_to_manifest_and_lock(
    manifest: &mut OrbitManifest,
    lockfile: &mut OrbitLockfile,
    installed: &[InstalledMod],
    mods_dir: &Path,
) {
    for inst in installed {
        let key = &inst.key;
        manifest.dependencies.entry(key.clone()).or_insert_with(|| {
            DependencySpec::Short(if inst.version.is_empty() { "*".into() } else { inst.version.clone() })
        });
        lockfile.entries.retain(|e| e.name != *key);
        let lock_deps: Vec<LockDependency> = inst.jar_deps.iter().map(|(dep_id, constraint, _)| LockDependency {
            name: dep_id.clone(),
            version: if constraint.is_empty() { "*".into() } else { constraint.clone() },
        }).collect();
        let sha256 = if inst.filename.is_empty() { String::new() }
            else { crate::jar::compute_sha256(&mods_dir.join(&inst.filename)).unwrap_or_default() };
        lockfile.entries.push(LockEntry {
            name: key.clone(), platform: Some(inst.provider.clone()),
            version: inst.version.clone(), filename: inst.filename.clone(),
            sha256, sha512: String::new(), dependencies: lock_deps, implanted: vec![],
            source_type: None, path: None, mod_id: None, slug: Some(key.clone()), url: None,
        });
    }
}
