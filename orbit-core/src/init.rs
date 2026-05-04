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
    pub sha1: String,
    pub sha256: String,
    pub sha512: String,
    /// 从 fabric.mod.json 提取的依赖: (mod_id, version_constraint, required)
    pub jar_deps: Vec<(String, String, bool)>,
    /// META-INF/jars/ 下的内嵌 JAR 路径（只有父模组才有值）
    pub embedded_jars: Vec<String>,
    /// 如果此模组是从某个父 JAR 解出的内嵌模组，记录父 JAR 的文件名
    pub embedded_parent: Option<String>,
}

/// 扫描 mods/ 目录并提取元数据。
///
/// 遍历 `{instance_dir}/mods/` 下所有 .jar 文件，
/// 读取 fabric.mod.json 并计算 SHA-256。
fn scan_mods_dir(
    instance_dir: &Path,
    loader: &str,
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

        eprintln!("  → {filename}:");

        let sha256 = crate::jar::compute_sha256(&path).map_err(|e| {
            OrbitError::Other(anyhow::anyhow!("cannot hash {}: {e}", path.display()))
        })?;
        let sha512 = crate::jar::compute_sha512(&path).map_err(|e| {
            OrbitError::Other(anyhow::anyhow!("cannot hash {}: {e}", path.display()))
        })?;
        eprintln!("    SHA-256: {}", &sha256[..16]);

        let (mod_id, mod_name, version, jar_deps, embedded) = match crate::jar::read_mod_metadata(&path, loader) {
            Ok(meta) => {
                eprintln!("    id={} name={} version={} deps={}", meta.mod_id, meta.name, meta.version, meta.dependencies.len());
                (if meta.mod_id.is_empty() { None } else { Some(meta.mod_id) },
                 if meta.name.is_empty() { None } else { Some(meta.name) },
                 if meta.version.is_empty() { None } else { Some(meta.version) },
                 meta.dependencies,
                 meta.embedded_jars)
            }
            Err(e) => {
                eprintln!("    ⚠ cannot read mod metadata: {e}");
                (None, None, None, vec![], vec![])
            }
        };

        results.push(ScannedMod {
            filename,
            mod_id: mod_id.clone().or_else(|| mod_name.clone()),
            mod_name,
            version,
            sha1: String::new(),
            sha256,
            sha512,
            jar_deps,
            embedded_jars: embedded,
            embedded_parent: None,
        });
    }

    // 扫描内嵌 JAR（META-INF/jars/ 下的子模组）
    scan_embedded_jars(instance_dir, &mut results, loader)?;

    Ok(results)
}

/// 扫描所有已发现模组的内嵌 JAR（按父模组分组，每个父 JAR 只打开一次）
fn scan_embedded_jars(
    instance_dir: &Path,
    results: &mut Vec<ScannedMod>,
    loader: &str,
) -> Result<(), OrbitError> {
    let mods_dir = instance_dir.join("mods");
    let mut new_mods = vec![];

    for parent in results.iter().filter(|p| !p.embedded_jars.is_empty()) {
        let parent_jar = mods_dir.join(&parent.filename);
        let file = std::fs::File::open(&parent_jar).map_err(|e| {
            OrbitError::Other(anyhow::anyhow!("cannot open {}: {e}", parent_jar.display()))
        })?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| {
            OrbitError::Other(anyhow::anyhow!("cannot open {} as ZIP: {e}", parent_jar.display()))
        })?;

        for emb_path in &parent.embedded_jars {
            eprintln!("    ↳ [{}] embedded: {emb_path}", parent.filename);
            let mut entry = match archive.by_name(emb_path) {
                Ok(e) => e,
                Err(_) => {
                    eprintln!("      ⚠ not found in JAR");
                    continue;
                }
            };
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut bytes).map_err(|e| {
                OrbitError::Other(anyhow::anyhow!("cannot read {emb_path}: {e}"))
            })?;
            let sha256 = crate::jar::sha256_digest(&bytes);
            let sha512 = crate::jar::sha512_digest(&bytes);
            let filename = std::path::Path::new(emb_path)
                .file_name().unwrap_or_default().to_string_lossy().to_string();
            eprintln!("      SHA-256: {}", &sha256[..16]);
            let (mod_id, mod_name, version, jar_deps) = match crate::jar::read_mod_metadata_from_bytes(&bytes, loader) {
                Ok(meta) => {
                    (if meta.mod_id.is_empty() { None } else { Some(meta.mod_id) },
                     if meta.name.is_empty() { None } else { Some(meta.name) },
                     if meta.version.is_empty() { None } else { Some(meta.version) },
                     meta.dependencies)
                }
                Err(e) => {
                    eprintln!("      ⚠ cannot read mod metadata from embedded: {e}");
                    (None, None, None, vec![])
                }
            };
            new_mods.push(ScannedMod {
                filename, mod_id: mod_id.clone().or_else(|| mod_name.clone()), mod_name,
                version,
                sha1: String::new(), sha256, sha512, jar_deps, embedded_jars: vec![],
                embedded_parent: Some(parent.filename.clone()),
            });
        }
    }
    results.append(&mut new_mods);
    Ok(())
}

/// 从字节数组读取 JAR 元数据（用于内嵌 JAR）

/// 从实例目录的 JAR 中自动检测 MC 版本。
///
/// 先查 versions/ 子目录（标准 MC 启动器布局），再回退到当前目录。
/// 避免 mod JAR 中的 version.json 干扰检测。
pub fn detect_mc_version(instance_dir: &std::path::Path) -> Result<crate::metadata::mojang::McVersion, OrbitError> {
    let mut search_dirs = Vec::new();

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
    search_dirs.push(instance_dir.to_path_buf());

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

    // 2. 分离内嵌模组：只对顶层模组调 API
    let (top_level, embedded): (Vec<_>, Vec<_>) = scanned.iter().partition(|s| s.embedded_parent.is_none());

    // 2a. 识别顶层模组
    eprintln!("Identifying top-level mods via Modrinth ...");
    let ctx = crate::identification::IdentificationContext {
        mc_version: input.mc_version.clone(),
        loader: input.modloader.clone(),
    };
    let top_slice: Vec<crate::init::ScannedMod> = top_level.into_iter().cloned().collect();
    let identified = crate::identification::identify_mods(&top_slice, providers, &ctx).await?;

    // 2b. 内嵌模组不调 API，直接用 JAR metadata（不加入顶层，仅用于 lock 的 implanted）
    let embedded_identified: Vec<_> = embedded.iter().map(|s| {
        crate::identification::IdentifiedMod {
            filename: s.filename.clone(),
            mod_id: s.mod_id.clone().unwrap_or_default(),
            mod_name: s.mod_name.clone().unwrap_or_default(),
            version: s.version.clone().unwrap_or_default(),
            modrinth_version: String::new(),
            sha1: s.sha1.clone(),
            sha512: s.sha512.clone(),
            sha256: s.sha256.clone(),
            source: crate::identification::IdentifiedSource::File { path: format!("mods/{}", s.filename) },
            deps: s.jar_deps.clone(),
        }
    }).collect();

    // 3. 构建依赖声明 + lock 条目（仅顶层模组）
    let mut lock_entries: Vec<crate::lockfile::PackageEntry> = identified
        .iter()
        .map(|m| {
            let key = if !m.mod_id.is_empty() { m.mod_id.clone() } else if !m.mod_name.is_empty() { m.mod_name.clone() } else { m.filename.clone() };
            let mut entry = crate::lockfile::PackageEntry {
                mod_id: key,
                version: m.version.clone(),
                sha1: m.sha1.clone(),
                sha256: m.sha256.clone(),
                sha512: m.sha512.clone(),
                filename: m.filename.clone(),
                provider: String::new(),
                modrinth: None,
                file: None,
                dependencies: vec![],
                implanted: vec![],
            };

            match &m.source {
                crate::identification::IdentifiedSource::Platform { platform, project_id, version_id, slug } => {
                    entry.provider = platform.clone();
                    entry.modrinth = Some(crate::lockfile::ModrinthInfo {
                        project_id: project_id.clone(),
                        version_id: version_id.clone(),
                        version: m.modrinth_version.clone(),
                        slug: slug.clone(),
                    });
                }
                crate::identification::IdentifiedSource::File { path } => {
                    entry.provider = "file".to_string();
                    entry.file = Some(crate::lockfile::FileInfo {
                        path: path.clone(),
                    });
                }
            }

            for (dep_id, constraint, req) in &m.deps {
                if *req && dep_id != "java" && dep_id != "mixinextras" && dep_id != "minecraft" && dep_id != "fabricloader" {
                    entry.dependencies.push(crate::lockfile::LockDependency {
                        name: dep_id.clone(),
                        version: if constraint.is_empty() { "*".to_string() } else { constraint.to_string() },
                    });
                }
            }
            entry
        })
        .collect();

    for m in &embedded_identified {
        // 找到父 JAR 文件名
        let parent_name = scanned.iter()
            .find(|s| s.filename == m.filename && s.embedded_parent.is_some())
            .and_then(|s| s.embedded_parent.as_deref());

        let Some(parent_name) = parent_name else { continue; };

        // 在顶层 identified 中找到父模组，推导 key 后按 mod_id 匹配 lock_entry
        let parent_key = identified.iter()
            .find(|im| im.filename == parent_name)
            .map(|im| {
                if !im.mod_id.is_empty() { im.mod_id.clone() }
                else if !im.mod_name.is_empty() { im.mod_name.clone() }
                else { im.filename.clone() }
            });

        let Some(parent_key) = parent_key else { continue; };

        if let Some(parent_entry) = lock_entries.iter_mut().find(|e| e.mod_id == parent_key) {
            if parent_entry.implanted.iter().any(|imp| imp.filename == m.filename) {
                continue;
            }
            let imp_deps: Vec<crate::lockfile::LockDependency> = m.deps.iter()
                .filter(|(id, _, req)| *req && id != "java" && id != "mixinextras" && id != "minecraft" && id != "fabricloader")
                .map(|(dep_id, constraint, _)| crate::lockfile::LockDependency {
                    name: dep_id.clone(),
                    version: if constraint.is_empty() { "*".to_string() } else { constraint.clone() },
                })
                .collect();
            parent_entry.implanted.push(crate::lockfile::ImplantedMod {
                name: if !m.mod_id.is_empty() { m.mod_id.clone() } else if !m.mod_name.is_empty() { m.mod_name.clone() } else { m.filename.clone() },
                version: m.version.clone(),
                sha256: m.sha256.clone(),
                filename: m.filename.clone(),
                dependencies: imp_deps,
            });
        }
    }

    let mc_ver = input.mc_version.clone();
    let loader_name = input.modloader.clone();
    let loader_ver = input.modloader_version.clone();
    let mut dependencies = indexmap::IndexMap::new();
    for m in &identified {
        let key = if !m.mod_id.is_empty() { m.mod_id.clone() } else if !m.mod_name.is_empty() { m.mod_name.clone() } else { m.filename.clone() };
        let spec = DependencySpec::Full {
            version: if m.version.is_empty() { None } else { Some(m.version.clone()) },
            optional: None,
            env: None,
            exclude: None,
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

    // 4. 使用 PubGrub 解析器检查依赖图完整性
    eprintln!("Verifying dependency graph using PubGrub resolver...");
    let mut all_local_mods = identified.clone();
    all_local_mods.extend(embedded_identified.clone());
    
    if let Err(err_msg) = crate::resolver::check_local_graph(&manifest, &all_local_mods) {
        eprintln!("\n⚠️  WARNING: Dependency graph verification failed!\n{}\n", err_msg);
        eprintln!("Please use 'orbit install' or 'orbit sync' to fix missing dependencies.");
    } else {
        eprintln!("Dependency graph verified successfully.");
    }

    // 4. 写入 orbit.toml + orbit.lock
    let lockfile = crate::lockfile::OrbitLockfile {
        meta: crate::lockfile::LockMeta {
            mc_version: mc_ver,
            modloader: loader_name,
            modloader_version: loader_ver,
        },
        packages: lock_entries,
    };

    let manifest_file = crate::workspace::ManifestFile::new(&input.instance_dir, manifest.clone());
    let lock = crate::workspace::Lockfile::new(&input.instance_dir, lockfile);
    manifest_file.save()?;
    lock.save()?;

    Ok(InitOutput {
        manifest,
        scanned_mods: scanned,
    })
}
