//! `orbit init` 命令编排。
//!
//! 检测加载器、扫描 mods/、生成 orbit.toml。

use std::path::Path;

use crate::error::OrbitError;
use crate::manifest::{DependencySpec, OrbitManifest, ProjectMeta, ResolverConfig};

/// 一次 init 的输入
pub struct InitInput {
    pub name: String,
    pub mc_version: String,
    pub modloader: String,
    pub modloader_version: String,
    /// 实例目录（即当前目录）
    pub instance_dir: std::path::PathBuf,
}

/// init 输出
pub struct InitOutput {
    pub manifest: OrbitManifest,
    pub scanned_mods: Vec<ScannedMod>,
}

/// 一个扫描到的模组
#[derive(Debug, Clone)]
pub struct ScannedMod {
    pub filename: String,
    pub mod_id: Option<String>,
    pub mod_name: Option<String>,
    pub version: Option<String>,
    pub sha256: String,
}

/// 扫描 mods/ 目录并提取元数据。
///
/// 遍历 `{instance_dir}/mods/` 下所有 .jar 文件，
/// 读取 fabric.mod.json 并计算 SHA-256。
fn scan_mods_dir(
    instance_dir: &Path,
    _loader: &str,
) -> Result<Vec<ScannedMod>, OrbitError> {
    let mods_dir = instance_dir.join("mods");
    if !mods_dir.is_dir() {
        return Ok(vec![]);
    }

    let mut results = vec![];

    for entry in std::fs::read_dir(&mods_dir).map_err(|e| {
        OrbitError::Other(anyhow::anyhow!("cannot read mods/ directory: {e}"))
    })? {
        let entry = entry.map_err(|e| {
            OrbitError::Other(anyhow::anyhow!("cannot read directory entry: {e}"))
        })?;
        let path = entry.path();

        // 只处理 .jar 文件
        if path.extension().map(|e| e != "jar").unwrap_or(true) {
            continue;
        }

        let filename = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let file = std::fs::File::open(&path).map_err(|e| {
            OrbitError::Other(anyhow::anyhow!("cannot open {}: {e}", path.display()))
        })?;

        // SHA-256 + 元数据提取
        let sha256 = crate::jar::compute_sha256(&path).map_err(|e| {
            OrbitError::Other(anyhow::anyhow!("cannot hash {}: {e}", path.display()))
        })?;

        // 尝试从 JAR 中提取 fabric.mod.json
        let (mod_id, mod_name, version) = match read_jar_metadata(file) {
            Ok((id, name, ver)) => (id, name, Some(ver)),
            Err(e) => {
                eprintln!("  ⚠ cannot read metadata from {}: {e}", filename);
                (None, None, None)
            }
        };

        results.push(ScannedMod {
            filename,
            mod_id: mod_id.clone().or_else(|| mod_name.clone()),
            mod_name,
            version,
            sha256,
        });
    }

    Ok(results)
}

/// 从 JAR 中读取 fabric.mod.json 并返回 (id, name, version)
///
/// 有些 JAR 打包时外层会套一层目录（如 `voxy-0.2.14-alpha/fabric.mod.json`），
/// 因此先尝试根路径，找不到则遍历所有条目按文件名匹配。
fn read_jar_metadata(
    file: std::fs::File,
) -> Result<(Option<String>, Option<String>, String), OrbitError> {
    let mut archive = zip::ZipArchive::new(file).map_err(|e| {
        OrbitError::Other(anyhow::anyhow!("cannot open JAR as ZIP: {e}"))
    })?;

    let target = "fabric.mod.json";

    // 先尝试根路径（绝大多数 JAR 的情况）
    let content = if let Ok(mut entry) = archive.by_name(target) {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut entry, &mut s).map_err(|e| {
            OrbitError::Other(anyhow::anyhow!("cannot read {target}: {e}"))
        })?;
        Some(s)
    } else {
        // 遍历查找：匹配 */fabric.mod.json（只取一层目录深度）
        let idx = (0..archive.len()).find(|&i| {
            archive.by_index(i)
                .map(|e| {
                    let name = e.name();
                    name.ends_with(target)
                        && (name == target
                            || name.matches('/').count() == 1)
                })
                .unwrap_or(false)
        });

        match idx {
            Some(i) => {
                let mut entry = archive.by_index(i).map_err(|e| {
                    OrbitError::Other(anyhow::anyhow!("cannot read ZIP entry: {e}"))
                })?;
                let mut s = String::new();
                std::io::Read::read_to_string(&mut entry, &mut s).map_err(|e| {
                    OrbitError::Other(anyhow::anyhow!("cannot read {target}: {e}"))
                })?;
                Some(s)
            }
            None => None,
        }
    };

    let Some(content) = content else {
        return Err(OrbitError::Other(anyhow::anyhow!("no {target} found in JAR")));
    };

    let parser = crate::metadata::fabric::FabricParser;
    let meta = crate::metadata::MetadataParser::parse(&parser, &content)?;

    let id = if meta.id.is_empty() { None } else { Some(meta.id) };
    let name = if meta.name.is_empty() { None } else { Some(meta.name) };
    Ok((id, name, meta.version))
}

/// 执行 init 流程。
///
/// 扫描 mods/ → 构建 OrbitManifest → 写入文件 → 注册实例。
pub fn run_init(input: InitInput) -> Result<InitOutput, OrbitError> {
    // 1. 扫描 mods/
    let scanned = scan_mods_dir(&input.instance_dir, &input.modloader)?;

    // 2. 构建依赖声明（每个扫描到的模组作为依赖条目）
    let mut dependencies = indexmap::IndexMap::new();
    for m in &scanned {
        // 有 mod_id 的模组记录为平台依赖（后续可通过 Modrinth API 查询版本）
        // 无法识别的模组记录为 file 类型
        let key = m.mod_name.clone().unwrap_or_else(|| m.filename.clone());
        let spec = if m.mod_id.is_some() {
            // 平台模组：version 留空，后续 orbit install 时解析
            DependencySpec::Full {
                platform: Some("modrinth".into()),
                slug: m.mod_id.clone(),
                version: m.version.clone(),
                optional: None,
                env: None,
                exclude: None,
                source_type: None,
                path: None,
                url: None,
                sha256: None,
            }
        } else {
            // 未知模组：记录为本地文件
            let relative_path = format!("mods/{}", m.filename);
            DependencySpec::Full {
                platform: None,
                slug: None,
                version: m.version.clone(),
                optional: None,
                env: None,
                exclude: None,
                source_type: Some("file".into()),
                path: Some(relative_path),
                url: None,
                sha256: Some(m.sha256.clone()),
            }
        };
        dependencies.insert(key, spec);
    }

    // 3. 构建 manifest
    let manifest = OrbitManifest {
        project: ProjectMeta {
            name: input.name,
            mc_version: input.mc_version,
            modloader: input.modloader,
            modloader_version: input.modloader_version,
            description: None,
            authors: None,
            version: None,
        },
        resolver: ResolverConfig::default(),
        dependencies,
        groups: Default::default(),
        overrides: Default::default(),
    };

    // 4. 写入 orbit.toml
    let toml_path = input.instance_dir.join("orbit.toml");
    let content = manifest.to_toml_string()?;
    std::fs::write(&toml_path, content).map_err(|e| {
        OrbitError::Other(anyhow::anyhow!("cannot write {}: {e}", toml_path.display()))
    })?;

    Ok(InitOutput {
        manifest,
        scanned_mods: scanned,
    })
}
