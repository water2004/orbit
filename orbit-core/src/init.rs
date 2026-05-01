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
    pub sha512: String,
    /// 从 fabric.mod.json 提取的依赖: mod_id → version_constraint
    pub jar_deps: Vec<(String, String)>,
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

        eprintln!("  → {filename}:");

        // SHA-256 + SHA-512（平台哈希反查用 SHA-512）
        let sha256 = crate::jar::compute_sha256(&path).map_err(|e| {
            OrbitError::Other(anyhow::anyhow!("cannot hash {}: {e}", path.display()))
        })?;
        let sha512 = crate::jar::compute_sha512(&path).map_err(|e| {
            OrbitError::Other(anyhow::anyhow!("cannot hash {}: {e}", path.display()))
        })?;
        eprintln!("    SHA-256: {}", &sha256[..16]);

        // 尝试从 JAR 中提取 fabric.mod.json
        let (mod_id, mod_name, version, jar_deps) = match read_jar_metadata(file) {
            Ok((id, name, ver, deps)) => {
                eprintln!("    fabric.mod.json: id={:?} name={:?} version={ver} deps={}", id, name, deps.len());
                (id, name, Some(ver), deps)
            }
            Err(e) => {
                eprintln!("    ⚠ cannot read fabric.mod.json: {e}");
                (None, None, None, vec![])
            }
        };

        results.push(ScannedMod {
            filename,
            mod_id: mod_id.clone().or_else(|| mod_name.clone()),
            mod_name,
            version,
            sha256,
            sha512,
            jar_deps,
        });
    }

    Ok(results)
}

/// 从 JAR 中读取 fabric.mod.json 并返回 (id, name, version, dependencies)
fn read_jar_metadata(
    file: std::fs::File,
) -> Result<(Option<String>, Option<String>, String, Vec<(String, String)>), OrbitError> {
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
    let deps: Vec<(String, String)> = meta.dependencies.into_iter().collect();
    Ok((id, name, meta.version, deps))
}

/// 从实例目录的 JAR 中自动检测 MC 版本。
///
/// 先查当前目录，找不到再查 versions/ 下一级子目录。
/// 提取 JAR 内的 `version.json`，若完全找不到则报错。
pub fn detect_mc_version(instance_dir: &std::path::Path) -> Result<crate::metadata::mojang::McVersion, OrbitError> {
    let mut search_dirs = vec![instance_dir.to_path_buf()];

    let versions_dir = instance_dir.join("versions");
    if versions_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&versions_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                if entry.path().is_dir() {
                    search_dirs.push(entry.path());
                }
            }
        }
    }

    for dir in &search_dirs {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.extension().map(|e| e != "jar").unwrap_or(true) {
                    continue;
                }
                // 尝试从 JAR 中提取 version.json
                if let Ok(version) = read_version_json_from_jar(&path) {
                    return Ok(version);
                }
            }
        }
    }

    Err(OrbitError::Other(anyhow::anyhow!(
        "no Minecraft version.json found in any JAR under {} or its versions/ subdirectories.\n\
         Specify --mc-version manually.",
        instance_dir.display()
    )))
}

/// 从游戏 JAR 中提取 version.json
fn read_version_json_from_jar(
    jar_path: &std::path::Path,
) -> Result<crate::metadata::mojang::McVersion, OrbitError> {
    let file = std::fs::File::open(jar_path).map_err(|e| {
        OrbitError::Other(anyhow::anyhow!("cannot open {}: {e}", jar_path.display()))
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| {
        OrbitError::Other(anyhow::anyhow!("cannot open {} as ZIP: {e}", jar_path.display()))
    })?;
    let mut entry = archive.by_name("version.json").map_err(|_| {
        OrbitError::Other(anyhow::anyhow!("no version.json in {}", jar_path.display()))
    })?;
    let mut content = String::new();
    std::io::Read::read_to_string(&mut entry, &mut content).map_err(|e| {
        OrbitError::Other(anyhow::anyhow!("cannot read version.json from {}: {e}", jar_path.display()))
    })?;
    crate::metadata::mojang::McVersion::from_json(&content)
}

/// 执行 init 流程。
///
/// 扫描 mods/ → 识别来源 → 构建 OrbitManifest → 写入文件。
pub async fn run_init(
    input: InitInput,
    providers: &[Box<dyn crate::providers::ModProvider>],
) -> Result<InitOutput, OrbitError> {
    // 1. 扫描 mods/
    eprintln!("Scanning mods/ ...");
    let scanned = scan_mods_dir(&input.instance_dir, &input.modloader)?;
    eprintln!("  found {} jar(s)\n", scanned.len());

    // 2. 识别来源
    eprintln!("Identifying mods via Modrinth ...");
    let ctx = crate::identification::IdentificationContext {
        mc_version: input.mc_version.clone(),
        loader: input.modloader.clone(),
    };
    let identified = crate::identification::identify_mods(&scanned, providers, &ctx).await?;

    // 3. 构建依赖声明 + lock 条目
    let (lock_entries, _warnings) = crate::resolver::build_lock_entries(&identified, &scanned);
    let mc_ver = input.mc_version.clone();
    let loader_name = input.modloader.clone();
    let loader_ver = input.modloader_version.clone();
    let mut dependencies = indexmap::IndexMap::new();
    for m in identified {
        let key = if m.mod_name.is_empty() { m.filename.clone() } else { m.mod_name.clone() };
        let spec = match &m.source {
            crate::identification::IdentifiedSource::Platform { platform, slug } => {
                DependencySpec::Full {
                    platform: Some(platform.clone()),
                    slug: Some(slug.clone()),
                    version: if m.version.is_empty() { None } else { Some(m.version.clone()) },
                    optional: None,
                    env: None,
                    exclude: None,
                    source_type: None,
                    path: None,
                    url: None,
                    sha256: None,
                }
            }
            crate::identification::IdentifiedSource::File { path } => {
                DependencySpec::Full {
                    platform: None,
                    slug: None,
                    version: if m.version.is_empty() { None } else { Some(m.version.clone()) },
                    optional: None,
                    env: None,
                    exclude: None,
                    source_type: Some("file".into()),
                    path: Some(path.clone()),
                    url: None,
                    sha256: Some(m.sha256.clone()),
                }
            }
        };
        dependencies.insert(key, spec);
    }

    // 3. 构建 manifest
    let manifest = OrbitManifest {
        project: ProjectMeta {
            name: input.name,
            mc_version: mc_ver.clone(),
            modloader: loader_name.clone(),
            modloader_version: loader_ver.clone(),
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
    std::fs::write(&toml_path, &content).map_err(|e| {
        OrbitError::Other(anyhow::anyhow!("cannot write {}: {e}", toml_path.display()))
    })?;

    // 5. 写入 orbit.lock
    let lockfile = crate::lockfile::OrbitLockfile {
        meta: crate::lockfile::LockMeta {
            mc_version: mc_ver,
            modloader: loader_name,
            modloader_version: loader_ver,
        },
        entries: lock_entries,
    };
    let lock_path = input.instance_dir.join("orbit.lock");
    let lock_content = lockfile.to_toml_string()?;
    std::fs::write(&lock_path, &lock_content).map_err(|e| {
        OrbitError::Other(anyhow::anyhow!("cannot write {}: {e}", lock_path.display()))
    })?;

    Ok(InitOutput {
        manifest,
        scanned_mods: scanned,
    })
}
