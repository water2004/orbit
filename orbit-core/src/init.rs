//! `orbit init` 命令编排。
//!
//! 检测加载器、扫描 mods/、生成 orbit.toml。

use std::path::Path;

use crate::error::OrbitError;
use crate::manifest::{OrbitManifest, PackageSpec, ProjectMeta, ResolverConfig};

pub use crate::platform_detection::{
    InitLoaderCandidate, detect_loader_candidates, detect_mc_version, detect_mc_versions,
    known_loader_choices,
};

/// 一次 init 的输入
pub struct InitInput {
    pub name: String,
    pub mc_version: String,
    pub modloader: String,
    pub modloader_version: String,
    /// 实例目录（即当前目录）
    pub instance_dir: std::path::PathBuf,
    pub dry_run: bool,
}

/// init 输出
pub struct InitOutput {
    pub manifest: OrbitManifest,
    pub scanned_mods: Vec<ScannedMod>,
    pub lock_created: bool,
    pub locked_packages: usize,
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
/// 按实例 loader 读取元数据并计算内容哈希。
pub(crate) fn scan_mods_dir(
    instance_dir: &Path,
    loader: crate::loader::LoaderKind,
) -> Result<Vec<ScannedMod>, OrbitError> {
    let Some(mods_dir) = existing_mods_dir(instance_dir)? else {
        return Ok(vec![]);
    };

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
        let metadata = crate::jar::read_mod_metadata(&path, loader).map_err(|error| {
            OrbitError::Other(anyhow::anyhow!(
                "cannot treat top-level package '{}' as a {loader} mod: {error}",
                path.display()
            ))
        })?;

        results.push(ScannedMod {
            filename,
            mod_id: Some(metadata.mod_id.clone()),
            mod_name: Some(metadata.name.clone()).filter(|name| !name.is_empty()),
            version: Some(metadata.version.clone()),
            sha1,
            sha256,
            sha512,
            curseforge_fingerprint,
            dependencies: metadata.dependencies.clone(),
            environment: metadata.environment,
            provides: metadata.provides.clone(),
            language_loader: metadata.language_loader.clone(),
            embedded_artifacts: metadata.embedded_artifacts.clone(),
            bundled: metadata
                .bundled_mods
                .iter()
                .map(crate::lockfile::BundledMod::from_jar_metadata)
                .collect(),
            embedded_jars: metadata.embedded_jars,
        });
    }

    Ok(results)
}

/// Return the factual package directory without manufacturing it.
///
/// A missing `mods/` is the canonical empty package set. An existing non-directory
/// path is corrupt instance state and must not be silently treated as empty.
pub(crate) fn existing_mods_dir(
    instance_dir: &Path,
) -> Result<Option<std::path::PathBuf>, OrbitError> {
    let mods_dir = instance_dir.join("mods");
    match std::fs::metadata(&mods_dir) {
        Ok(metadata) if metadata.is_dir() => Ok(Some(mods_dir)),
        Ok(_) => Err(OrbitError::Other(anyhow::anyhow!(
            "mods path is not a directory: {}",
            mods_dir.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(OrbitError::Io(error)),
    }
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

    // Platform discovery is the validity gate for an instance. The caller's
    // values select a candidate; the paths and metadata always come from disk.
    let platform = crate::platform_detection::discover_platform_for_init(
        &input.instance_dir,
        &input.mc_version,
        &input.modloader,
        &input.modloader_version,
    )?;
    let platform_snapshot = platform.snapshot(&input.instance_dir)?;

    // 1. 扫描 mods/
    let scanned = scan_mods_dir(&input.instance_dir, platform.loader)?;

    // 2. Identify top-level package JARs. Modules contained in one package are
    // already represented by the JAR layer as bundled metadata.
    let mut identified = crate::identification::identify_mods(&scanned, providers).await?;
    if !input.dry_run {
        crate::identification::preserve_local_sources(&input.instance_dir, &mut identified)?;
    }

    // 3. Build complete managed-package declarations and concrete top-level candidates.
    let mut package_remotes =
        std::collections::HashMap::<String, Vec<crate::manifest::PackageRemote>>::new();
    for package in &identified {
        package_remotes
            .entry(package.package_id())
            .or_default()
            .extend(package.remotes.iter().cloned());
    }
    for remotes in package_remotes.values_mut() {
        remotes.sort();
        remotes.dedup();
    }
    let mut lock_entries: Vec<crate::lockfile::PackageEntry> = identified
        .iter()
        .map(crate::identification::IdentifiedMod::to_package_entry)
        .collect();
    for entry in &mut lock_entries {
        entry.remotes = package_remotes[&entry.mod_id].clone();
    }

    let mc_ver = platform.minecraft_version.id.clone();
    let loader_name = platform.loader;
    let loader_ver = platform.loader_version.clone();
    let mut packages = indexmap::IndexMap::new();
    for m in &identified {
        let key = m.package_id();
        let spec = PackageSpec {
            version: "*".to_string(),
            string: "all".to_string(),
            optional: false,
            env: None,
            exclude: Vec::new(),
            remotes: m.remotes.clone(),
        };
        packages
            .entry(key)
            .and_modify(|existing: &mut PackageSpec| {
                existing.remotes.extend(spec.remotes.iter().cloned());
                existing.remotes.sort();
                existing.remotes.dedup();
            })
            .or_insert(spec);
    }

    // 3. 构建 manifest
    let manifest = OrbitManifest {
        project: ProjectMeta {
            name: input.name,
            mc_version: mc_ver.clone(),
            modloader: loader_name.to_string(),
            modloader_version: loader_ver.clone(),
            description: None,
            authors: None,
            version: None,
        },
        platform: platform_snapshot,
        resolver: ResolverConfig::default(),
        packages,
        groups: Default::default(),
    };

    // 4. Initialization records factual local state. It never chooses among
    // duplicate realizations or changes the package set; those actions belong
    // exclusively to `fix`.
    let duplicates = duplicate_packages(&lock_entries);
    let lock_created = duplicates.is_empty();
    let lockfile = crate::lockfile::OrbitLockfile {
        meta: crate::lockfile::LockMeta {
            mc_version: mc_ver,
            modloader: loader_name.to_string(),
            modloader_version: loader_ver,
        },
        packages: if lock_created {
            lock_entries
        } else {
            Vec::new()
        },
    };
    let locked_packages = lockfile.packages.len();
    let dependency_error = if lock_created {
        crate::resolver::check_lockfile_graph_with_loader(
            &manifest,
            &lockfile,
            platform.loader_package.as_ref(),
        )
        .err()
        .map(|error| error.to_string())
    } else {
        Some(format!(
            "multiple local realizations exist for: {}; run 'orbit fix' to select a feasible package solution",
            duplicates.join(", ")
        ))
    };

    let manifest_file = crate::workspace::ManifestFile::new(&input.instance_dir, manifest.clone());
    let lock = crate::workspace::Lockfile::new(&input.instance_dir, lockfile);
    if !input.dry_run {
        manifest_file.save()?;
        if lock_created {
            lock.save()?;
        }
    }

    Ok(InitOutput {
        manifest,
        scanned_mods: scanned,
        lock_created,
        locked_packages,
        dependency_error,
    })
}

fn duplicate_packages(entries: &[crate::lockfile::PackageEntry]) -> Vec<String> {
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for entry in entries {
        *counts.entry(entry.mod_id.clone()).or_default() += 1;
    }
    counts
        .into_iter()
        .filter_map(|(package, count)| (count > 1).then_some(package))
        .collect()
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

    fn write_fabric_package(path: &Path, mod_id: &str, version: &str) {
        let metadata = format!(
            r#"{{"schemaVersion":1,"id":"{mod_id}","version":"{version}","name":"Example"}}"#
        );
        write_jar(
            path,
            &jar_bytes(&[("fabric.mod.json", metadata.as_bytes())]),
        );
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

        let scanned = scan_mods_dir(&instance, crate::loader::LoaderKind::Fabric).unwrap();

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

        let scanned = scan_mods_dir(&instance, crate::loader::LoaderKind::Fabric).unwrap();

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

    #[test]
    fn scan_mods_dir_rejects_a_top_level_jar_without_loader_metadata() {
        let instance = temp_instance_dir("invalid-top-level-package");
        let mods_dir = instance.join("mods");
        std::fs::create_dir_all(&mods_dir).unwrap();
        write_jar(
            &mods_dir.join("library-only.jar"),
            &jar_bytes(&[("example.txt", b"not a mod")]),
        );

        let error = scan_mods_dir(&instance, crate::loader::LoaderKind::Fabric).unwrap_err();

        assert!(error.to_string().contains("top-level package"));
        assert!(error.to_string().contains("fabric"));
        std::fs::remove_dir_all(instance).ok();
    }

    #[test]
    fn scan_mods_dir_rejects_an_existing_non_directory_mods_path() {
        let instance = temp_instance_dir("mods-is-file");
        std::fs::create_dir_all(&instance).unwrap();
        std::fs::write(instance.join("mods"), b"not a directory").unwrap();

        let error = scan_mods_dir(&instance, crate::loader::LoaderKind::Fabric).unwrap_err();

        assert!(error.to_string().contains("mods path is not a directory"));
        std::fs::remove_dir_all(instance).unwrap();
    }

    #[tokio::test]
    async fn init_and_sync_keep_a_missing_mods_directory_as_an_empty_package_set() {
        let directory = temp_instance_dir("missing-mods");
        crate::platform_detection::test_support::write_platform(
            &directory, "1.20.1", "fabric", "0.16.10",
        );
        assert!(!directory.join("mods").exists());

        let output = run_init(
            InitInput {
                name: "empty-instance".to_string(),
                mc_version: "1.20.1".to_string(),
                modloader: "fabric".to_string(),
                modloader_version: "0.16.10".to_string(),
                instance_dir: directory.clone(),
                dry_run: false,
            },
            &[],
        )
        .await
        .unwrap();

        assert!(output.scanned_mods.is_empty());
        assert!(output.lock_created);
        assert_eq!(output.locked_packages, 0);
        assert!(!directory.join("mods").exists());

        let report = crate::sync::sync_instance(&directory, &[], false)
            .await
            .unwrap();

        assert!(report.added.is_empty());
        assert!(report.changed.is_empty());
        assert!(report.missing.is_empty());
        assert!(report.removed.is_empty());
        assert!(!directory.join("mods").exists());
        std::fs::remove_dir_all(directory).unwrap();
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
            dry_run: false,
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

    #[tokio::test]
    async fn init_records_duplicate_sources_without_selecting_or_deleting_them() {
        let directory = temp_instance_dir("duplicate-packages");
        crate::platform_detection::test_support::write_platform(
            &directory, "1.20.1", "fabric", "0.16.10",
        );
        let mods = directory.join("mods");
        std::fs::create_dir_all(&mods).unwrap();
        write_fabric_package(&mods.join("alpha-1.jar"), "alpha", "1");
        write_fabric_package(&mods.join("alpha-2.jar"), "alpha", "2");
        let input = |dry_run| InitInput {
            name: "test".to_string(),
            mc_version: "1.20.1".to_string(),
            modloader: "fabric".to_string(),
            modloader_version: "0.16.10".to_string(),
            instance_dir: directory.clone(),
            dry_run,
        };

        let preview = run_init(input(true), &[]).await.unwrap();
        assert!(!preview.lock_created);
        assert_eq!(preview.locked_packages, 0);
        assert!(!directory.join("orbit.toml").exists());
        assert!(mods.join("alpha-1.jar").exists());

        let output = run_init(input(false), &[]).await.unwrap();

        assert!(!output.lock_created);
        assert_eq!(output.locked_packages, 0);
        assert!(output.dependency_error.as_deref().is_some_and(|error| {
            error.contains("multiple local realizations") && error.contains("orbit fix")
        }));
        assert!(mods.join("alpha-1.jar").exists());
        assert!(mods.join("alpha-2.jar").exists());
        assert!(matches!(
            crate::workspace::Lockfile::open(&directory),
            Err(OrbitError::LockfileNotFound)
        ));
        assert_eq!(output.manifest.packages["alpha"].version_constraint(), "*");
        assert_eq!(output.manifest.packages["alpha"].env(), None);
        assert_eq!(output.manifest.packages["alpha"].remotes.len(), 2);
        assert!(
            output.manifest.packages["alpha"]
                .remotes
                .iter()
                .all(|remote| remote.display_locator() == "file:managed local source")
        );
        assert_eq!(
            std::fs::read_dir(directory.join(".orbit/sources"))
                .unwrap()
                .count(),
            2
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
