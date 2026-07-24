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
    pub dependency_error: Option<String>,
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
    pub curseforge_fingerprint: u32,
    pub dependencies: Vec<crate::metadata::DependencyExpression>,
    pub environment: crate::metadata::Environment,
    pub provides: Vec<crate::metadata::ProvidedMod>,
    pub language_loader: Option<crate::metadata::LanguageLoaderRequirement>,
    pub embedded_artifacts: Vec<crate::metadata::EmbeddedArtifact>,
    pub bundled: Vec<crate::lockfile::BundledMod>,
    pub embedded_jars: Vec<String>,
}

/// 扫描 mods/ 目录并提取元数据。
///
/// 遍历 `{instance_dir}/mods/` 下所有 .jar 文件，
/// 读取 fabric.mod.json 并计算 SHA-256。
pub(crate) fn scan_mods_dir(
    instance_dir: &Path,
    loader: &str,
) -> Result<Vec<ScannedMod>, OrbitError> {
    let mods_dir = instance_dir.join("mods");
    if !mods_dir.is_dir() {
        return Ok(vec![]);
    }

    let mut results = vec![];

    for entry in std::fs::read_dir(&mods_dir)
        .map_err(|e| OrbitError::Other(anyhow::anyhow!("cannot read mods/ directory: {e}")))?
    {
        let entry = entry
            .map_err(|e| OrbitError::Other(anyhow::anyhow!("cannot read directory entry: {e}")))?;
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

        let sha256 = crate::jar::compute_sha256(&path).map_err(|e| {
            OrbitError::Other(anyhow::anyhow!("cannot hash {}: {e}", path.display()))
        })?;
        let sha1 = crate::jar::compute_sha1(&path).map_err(|e| {
            OrbitError::Other(anyhow::anyhow!("cannot hash {}: {e}", path.display()))
        })?;
        let sha512 = crate::jar::compute_sha512(&path).map_err(|e| {
            OrbitError::Other(anyhow::anyhow!("cannot hash {}: {e}", path.display()))
        })?;
        let curseforge_fingerprint =
            crate::jar::compute_curseforge_fingerprint(&path).map_err(|e| {
                OrbitError::Other(anyhow::anyhow!(
                    "cannot fingerprint {} for CurseForge: {e}",
                    path.display()
                ))
            })?;
        let metadata = crate::jar::read_mod_metadata(&path, loader).ok();

        results.push(ScannedMod {
            filename,
            mod_id: metadata
                .as_ref()
                .map(|metadata| metadata.mod_id.clone())
                .filter(|id| !id.is_empty()),
            mod_name: metadata
                .as_ref()
                .map(|metadata| metadata.name.clone())
                .filter(|name| !name.is_empty()),
            version: metadata
                .as_ref()
                .map(|metadata| metadata.version.clone())
                .filter(|version| !version.is_empty()),
            sha1,
            sha256,
            sha512,
            curseforge_fingerprint,
            dependencies: metadata
                .as_ref()
                .map(|metadata| metadata.dependencies.clone())
                .unwrap_or_default(),
            environment: metadata
                .as_ref()
                .map(|metadata| metadata.environment)
                .unwrap_or_default(),
            provides: metadata
                .as_ref()
                .map(|metadata| metadata.provides.clone())
                .unwrap_or_default(),
            language_loader: metadata
                .as_ref()
                .and_then(|metadata| metadata.language_loader.clone()),
            embedded_artifacts: metadata
                .as_ref()
                .map(|metadata| metadata.embedded_artifacts.clone())
                .unwrap_or_default(),
            bundled: metadata
                .as_ref()
                .map(|metadata| {
                    metadata
                        .bundled_mods
                        .iter()
                        .map(crate::lockfile::BundledMod::from_jar_metadata)
                        .collect()
                })
                .unwrap_or_default(),
            embedded_jars: metadata
                .map(|metadata| metadata.embedded_jars)
                .unwrap_or_default(),
        });
    }

    Ok(results)
}

/// 从实例目录的 JAR 中自动检测 MC 版本。
///
/// 先查 versions/ 子目录（标准 MC 启动器布局），再回退到当前目录。
/// 避免 mod JAR 中的 version.json 干扰检测。
pub fn detect_mc_version(
    instance_dir: &std::path::Path,
) -> Result<crate::metadata::mojang::McVersion, OrbitError> {
    let mut search_dirs = Vec::new();

    let versions_dir = instance_dir.join("versions");
    if versions_dir.is_dir()
        && let Ok(entries) = std::fs::read_dir(&versions_dir)
    {
        for entry in entries.filter_map(|e| e.ok()) {
            if entry.path().is_dir() {
                search_dirs.push(entry.path());
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
        OrbitError::Other(anyhow::anyhow!(
            "cannot open {} as ZIP: {e}",
            jar_path.display()
        ))
    })?;
    let mut entry = archive.by_name("version.json").map_err(|_| {
        OrbitError::Other(anyhow::anyhow!("no version.json in {}", jar_path.display()))
    })?;
    let mut content = String::new();
    std::io::Read::read_to_string(&mut entry, &mut content).map_err(|e| {
        OrbitError::Other(anyhow::anyhow!(
            "cannot read version.json from {}: {e}",
            jar_path.display()
        ))
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
    if input.instance_dir.join("orbit.toml").exists() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "orbit.toml already exists in this directory; use 'orbit sync' to reconcile it"
        )));
    }

    // 1. 扫描 mods/
    let scanned = scan_mods_dir(&input.instance_dir, &input.modloader)?;

    // 2. 识别物理 JAR；同文件提供的模组已经由 JAR 层归入 bundled。
    let identified = crate::identification::identify_mods(&scanned, providers).await?;

    // 3. 构建依赖声明 + lock 条目（仅顶层模组）
    let lock_entries: Vec<crate::lockfile::PackageEntry> = identified
        .iter()
        .map(|m| {
            let key = if !m.mod_id.is_empty() {
                m.mod_id.clone()
            } else if !m.mod_name.is_empty() {
                m.mod_name.clone()
            } else {
                m.filename.clone()
            };
            let mut entry = crate::lockfile::PackageEntry {
                mod_id: key,
                version: m.version.clone(),
                sha1: m.sha1.clone(),
                sha256: m.sha256.clone(),
                sha512: m.sha512.clone(),
                filename: m.filename.clone(),
                provider: String::new(),
                modrinth: None,
                curseforge: None,
                file: None,
                dependencies: m.dependencies.clone(),
                environment: m.environment,
                provides: m.provides.clone(),
                language_loader: m.language_loader.clone(),
                embedded_artifacts: m.embedded_artifacts.clone(),
                bundled: m.bundled.clone(),
            };

            match &m.source {
                crate::identification::IdentifiedSource::Platform(platform) => {
                    entry.provider = platform.name().to_string();
                    match platform {
                        crate::identification::IdentifiedPlatform::Modrinth(metadata) => {
                            entry.modrinth = Some(metadata.clone());
                        }
                        crate::identification::IdentifiedPlatform::CurseForge(metadata) => {
                            entry.curseforge = Some(metadata.clone());
                        }
                    }
                }
                crate::identification::IdentifiedSource::File { path } => {
                    entry.provider = "file".to_string();
                    entry.file = Some(crate::lockfile::FileInfo { path: path.clone() });
                }
            }

            entry
        })
        .collect();

    let mc_ver = input.mc_version.clone();
    let loader_name = input.modloader.clone();
    let loader_ver = input.modloader_version.clone();
    let mut dependencies = indexmap::IndexMap::new();
    for m in &identified {
        let key = if !m.mod_id.is_empty() {
            m.mod_id.clone()
        } else if !m.mod_name.is_empty() {
            m.mod_name.clone()
        } else {
            m.filename.clone()
        };
        let spec = DependencySpec::Full {
            version: if m.version.is_empty() {
                None
            } else {
                Some(m.version.clone())
            },
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
    let dependency_error = crate::resolver::check_local_graph(&manifest, &identified).err();

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
        dependency_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use std::path::{Path, PathBuf};
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    const SODIUM_LIKE: &str = include_str!("../tests/fixtures/sodium_like.fabric.mod.json");
    const PRINTER_PARENT: &str = include_str!("../tests/fixtures/printer_parent.fabric.mod.json");
    const PRINTER_EMBEDDED: &str =
        include_str!("../tests/fixtures/printer_embedded.fabric.mod.json");

    fn jar_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut zip = ZipWriter::new(cursor);
        let options = SimpleFileOptions::default();

        for (name, contents) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(contents).unwrap();
        }

        zip.finish().unwrap().into_inner()
    }

    fn temp_instance_dir(test_name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../target/test-tmp")
            .join(format!("orbit-{test_name}-{}-{nanos}", std::process::id()))
    }

    fn write_jar(path: &Path, bytes: &[u8]) {
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn scan_mods_dir_ignores_old_and_disabled_jars() {
        let instance = temp_instance_dir("scan-ignores-disabled");
        let mods_dir = instance.join("mods");
        std::fs::create_dir_all(&mods_dir).unwrap();

        let active = jar_bytes(&[("fabric.mod.json", SODIUM_LIKE.as_bytes())]);
        write_jar(
            &mods_dir.join("sodium-fabric-0.8.11+mc1.21.11.jar"),
            &active,
        );
        write_jar(
            &mods_dir.join("sodium-fabric-0.8.10+mc1.21.11.jar.old"),
            &active,
        );
        write_jar(
            &mods_dir.join("sodium-fabric-0.8.9+mc1.21.11.jar.disabled"),
            &active,
        );

        let scanned = scan_mods_dir(&instance, "fabric").unwrap();

        assert_eq!(scanned.len(), 1);
        assert_eq!(scanned[0].filename, "sodium-fabric-0.8.11+mc1.21.11.jar");
        assert_eq!(scanned[0].mod_id.as_deref(), Some("sodium"));
        assert_eq!(scanned[0].embedded_jars.len(), 9);

        std::fs::remove_dir_all(instance).ok();
    }

    #[test]
    fn scan_mods_dir_records_embedded_fabric_jars() {
        let instance = temp_instance_dir("scan-embedded");
        let mods_dir = instance.join("mods");
        std::fs::create_dir_all(&mods_dir).unwrap();

        let embedded_path = "META-INF/jars/litematica-printer-1.21.11-2.4+20260330.10.jar";
        let embedded = jar_bytes(&[("fabric.mod.json", PRINTER_EMBEDDED.as_bytes())]);
        let parent_entries: [(&str, &[u8]); 2] = [
            ("fabric.mod.json", PRINTER_PARENT.as_bytes()),
            (embedded_path, embedded.as_slice()),
        ];
        let parent = jar_bytes(&parent_entries);
        write_jar(
            &mods_dir.join("litematica-printer-all-2.4+20260330.10.jar"),
            &parent,
        );

        let scanned = scan_mods_dir(&instance, "fabric").unwrap();

        assert_eq!(scanned.len(), 1);
        let parent_mod = &scanned[0];
        assert_eq!(parent_mod.mod_id.as_deref(), Some("litematica-printer-all"));
        assert_eq!(parent_mod.embedded_jars.len(), 13);
        assert!(
            parent_mod
                .embedded_jars
                .iter()
                .any(|path| path == embedded_path)
        );

        let embedded_mod = &parent_mod.bundled[0];
        assert_eq!(embedded_mod.mod_id, "litematica-printer");
        assert!(
            embedded_mod
                .dependencies
                .iter()
                .flat_map(|dependency| dependency.relations())
                .any(|dependency| dependency.id == "minecraft"
                    && dependency.requirement == "1.21.11"
                    && dependency.kind.installs_target())
        );

        std::fs::remove_dir_all(instance).ok();
    }

    #[tokio::test]
    async fn init_refuses_to_overwrite_an_existing_manifest() {
        let directory = temp_instance_dir("existing-manifest");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("orbit.toml"),
            "[project]\nname = \"keep-me\"\n",
        )
        .unwrap();
        let input = InitInput {
            name: "replacement".to_string(),
            mc_version: "1.21.1".to_string(),
            modloader: "fabric".to_string(),
            modloader_version: "0.16.0".to_string(),
            instance_dir: directory.clone(),
        };

        let error = match run_init(input, &[]).await {
            Ok(_) => panic!("init unexpectedly overwrote an existing manifest"),
            Err(error) => error.to_string(),
        };

        assert!(error.contains("already exists"));
        assert!(
            std::fs::read_to_string(directory.join("orbit.toml"))
                .unwrap()
                .contains("keep-me")
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
