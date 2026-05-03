//! 单模组安装逻辑。
//!
//! `orbit install <slug>` 的完整流程：
//! 1. provider 解析 slug → 版本 + 依赖（含正确的 slug）
//! 2. 检查本地状态（toml + lock），区分可选/必选依赖
//! 3. 缺失的必选依赖 → 尝试在线解析
//! 4. 版本冲突 → 报错
//! 5. 下载所有文件 → 写入 mods/
//! 6. 解析 JAR 内的 fabric.mod.json → 更新 toml + lock

use std::path::{Path, PathBuf};

use crate::error::OrbitError;
use crate::lockfile::{LockDependency, LockEntry, OrbitLockfile};
use crate::manifest::{DependencySpec, OrbitManifest};
use crate::providers::{ModProvider, ResolvedMod};

/// 单次 install 报告
#[derive(Debug, Clone)]
pub struct InstallReport {
    /// 已成功安装的模组（含主模组和依赖）
    pub installed: Vec<InstalledMod>,
    /// 已存在、无需安装的依赖名
    pub already_satisfied: Vec<String>,
    /// 跳过的可选依赖
    pub skipped_optional: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct InstalledMod {
    pub slug: String,
    /// orbit.toml 中的键名（即用户看到的依赖名）
    pub key: String,
    pub version: String,
    pub filename: String,
    /// 从 JAR 提取的真实依赖: (mod_id, version_constraint, required)
    pub jar_deps: Vec<(String, String, bool)>,
}

/// 安装单个模组（含依赖解析、下载、JAR 校验）。
///
/// - `no_deps`: 不安装传递依赖
/// - `existing_ok`: 主模组已存在时是否视为成功（否则报错）
pub async fn install_mod(
    slug: &str,
    constraint: &str,
    provider: &dyn ModProvider,
    manifest: &mut OrbitManifest,
    lockfile: &mut OrbitLockfile,
    mods_dir: &Path,
    no_deps: bool,
    existing_ok: bool,
) -> Result<InstallReport, OrbitError> {
    let mc_version = manifest.project.mc_version.clone();
    let loader = manifest.project.modloader.clone();

    // ── 1. 解析主模组 ──────────────────────────────────────────────
    let main_mod = provider
        .resolve(slug, constraint, &mc_version, &loader)
        .await?;

    // 检查是否已存在
    let main_key = slug.to_string();
    if !existing_ok && crate::resolver::find_entry(slug, &lockfile.entries).is_some() {
        return Err(OrbitError::Conflict(format!(
            "'{main_key}' already in lockfile. Use 'orbit upgrade {main_key}' to update it."
        )));
    }

    // ── 2. 检查依赖 ────────────────────────────────────────────────
    let mut to_install: Vec<ResolvedMod> = vec![main_mod.clone()];
    let mut already_satisfied: Vec<String> = Vec::new();
    let mut skipped_optional: Vec<String> = Vec::new();

    if !no_deps {
        for dep in &main_mod.dependencies {
            let Some(dep_slug) = dep.slug.as_deref() else { continue; };

            if !dep.required {
                skipped_optional.push(dep_slug.to_string());
                continue;
            }

            if crate::resolver::find_entry(dep_slug, &lockfile.entries).is_some() {
                already_satisfied.push(dep_slug.to_string());
                continue;
            }

            // 尝试在线解析
            match provider.resolve(dep_slug, "*", &mc_version, &loader).await {
                Ok(resolved_dep) => {
                    to_install.push(resolved_dep);
                }
                Err(_) => {
                    return Err(OrbitError::Conflict(format!(
                        "required dependency '{dep_slug}' could not be resolved on {}",
                        provider.name()
                    )));
                }
            }
        }
    }

    // ── 3. 版本冲突检查 ────────────────────────────────────────────
    for m in &to_install {
        if let Err(msg) = crate::resolver::check_version_conflict(&m.name, &m.version, &lockfile.entries) {
            return Err(OrbitError::Conflict(msg));
        }
    }

    // ── 4. 下载 ────────────────────────────────────────────────────
    let mut installed = Vec::new();
    for m in &to_install {
        let dest_path = download_mod(m, mods_dir).await?;
        let jar_deps = parse_jar_deps(&dest_path)?;

        installed.push(InstalledMod {
            slug: m.name.clone(),
            key: pick_key(&m.name, &m.name, manifest),
            version: m.version.clone(),
            filename: m.filename.clone(),
            jar_deps,
        });
    }

    // ── 5. 写入 manifest + lockfile ────────────────────────────────
    apply_to_manifest_and_lock(manifest, lockfile, &installed, &to_install);

    Ok(InstallReport {
        installed,
        already_satisfied,
        skipped_optional,
    })
}

// ── helpers ──────────────────────────────────────────────────────────

/// 下载模组 JAR 到 mods/ 目录，返回最终文件路径
async fn download_mod(
    m: &ResolvedMod,
    mods_dir: &Path,
) -> Result<PathBuf, OrbitError> {
    let final_path = mods_dir.join(&m.filename);

    // 已存在 → 跳过（后续写入 lock 时会重算 SHA-256）
    if final_path.exists() {
        let meta = std::fs::metadata(&final_path).map_err(OrbitError::Io)?;
        if meta.len() > 0 {
            return Ok(final_path);
        }
    }

    // 下载
    let bytes = reqwest::get(&m.download_url)
        .await
        .map_err(OrbitError::Network)?
        .bytes()
        .await
        .map_err(OrbitError::Network)?;

    // 先写入 .tmp → 原子 rename
    let tmp_path = mods_dir.join(format!(".{}.tmp", m.filename));
    std::fs::write(&tmp_path, &bytes).map_err(OrbitError::Io)?;
    std::fs::rename(&tmp_path, &final_path).map_err(OrbitError::Io)?;

    Ok(final_path)
}

/// 解析 JAR 内的 fabric.mod.json，提取依赖列表
fn parse_jar_deps(jar_path: &Path) -> Result<Vec<(String, String, bool)>, OrbitError> {
    let file = std::fs::File::open(jar_path).map_err(OrbitError::Io)?;
    let mut archive = zip::ZipArchive::new(file).map_err(OrbitError::Zip)?;

    match archive.by_name("fabric.mod.json") {
        Ok(mut entry) => {
            let mut content = String::new();
            std::io::Read::read_to_string(&mut entry, &mut content)
                .map_err(|e| OrbitError::Io(e.into()))?;
            let parser = crate::metadata::fabric::FabricParser;
            let meta = crate::metadata::MetadataParser::parse(&parser, &content)?;
            Ok(meta
                .dependencies
                .into_iter()
                .map(|(k, v)| (k, v, true))
                .collect())
        }
        Err(_) => Ok(vec![]),
    }
}

/// 根据安装结果更新 orbit.toml 和 orbit.lock
fn apply_to_manifest_and_lock(
    manifest: &mut OrbitManifest,
    lockfile: &mut OrbitLockfile,
    installed: &[InstalledMod],
    resolved: &[ResolvedMod],
) {
    // 收集所有已安装的模组信息
    for (inst, resolved_mod) in installed.iter().zip(resolved.iter()) {
        let key = &inst.key;

        // ── manifest ──
        manifest.dependencies.entry(key.clone()).or_insert_with(|| {
            if inst.version.is_empty() {
                DependencySpec::Short("*".into())
            } else {
                DependencySpec::Short(inst.version.clone())
            }
        });

        // ── lockfile ──
        // 删除旧条目（如果存在）
        lockfile.entries.retain(|e| e.name != *key);

        // 从 JAR 解析的真实依赖 → lock dependencies
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

        let sha256 = if inst.filename.is_empty() {
            String::new()
        } else {
            crate::jar::compute_sha256(&std::path::Path::new("mods").join(&inst.filename))
                .unwrap_or_default()
        };

        lockfile.entries.push(LockEntry {
            name: key.clone(),
            platform: Some("modrinth".into()),
            mod_id: Some(resolved_mod.mod_id.clone()),
            version: inst.version.clone(),
            filename: inst.filename.clone(),
            url: Some(resolved_mod.download_url.clone()),
            sha256,
            dependencies: lock_deps,
            implanted: vec![],
            source_type: None,
            path: None,
        });
    }
}

/// 选择一个不在 manifest 中已存在的键名
fn pick_key(slug: &str, name: &str, manifest: &OrbitManifest) -> String {
    if manifest.dependencies.contains_key(slug) {
        slug.to_string()
    } else if manifest.dependencies.contains_key(name) {
        name.to_string()
    } else {
        // 优先用 slug（更短、更规范）
        slug.to_string()
    }
}
