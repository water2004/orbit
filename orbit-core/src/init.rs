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
    pub dry_run: bool,
}

/// init 输出
pub struct InitOutput {
    pub manifest: OrbitManifest,
    pub scanned_mods: Vec<ScannedMod>,
    pub removed: Vec<crate::installer::RemovedPackage>,
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

/// Detects the single unambiguous Minecraft version for an instance.
pub fn detect_mc_version(
    instance_dir: &std::path::Path,
) -> Result<crate::metadata::mojang::McVersion, OrbitError> {
    let versions = detect_mc_versions(instance_dir)?;
    match versions.as_slice() {
        [version] => Ok(version.clone()),
        [] => Err(OrbitError::Other(anyhow::anyhow!(
            "no Minecraft client JAR with version.json was found for '{}'",
            instance_dir.display()
        ))),
        versions => {
            let ids = versions
                .iter()
                .map(|version| version.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            Err(OrbitError::Other(anyhow::anyhow!(
                "multiple Minecraft versions are available for '{}': {ids}; pass --mc-version",
                instance_dir.display()
            )))
        }
    }
}

/// Returns every actual Minecraft client version visible to this one game
/// directory. Mod JARs are never scanned.
pub fn detect_mc_versions(
    instance_dir: &std::path::Path,
) -> Result<Vec<crate::metadata::mojang::McVersion>, OrbitError> {
    let layout = crate::launcher::LauncherLayout::discover(instance_dir)?;
    let configured_versions = layout.configured_minecraft_versions();
    let expected_version =
        (configured_versions.len() == 1).then(|| configured_versions[0].as_str());
    let mut jar_paths = Vec::new();
    for directory in &layout.game_jar_directories {
        collect_direct_jars(directory, &mut jar_paths)?;
    }
    for library_root in &layout.library_roots {
        let minecraft_root = library_root.join("com").join("mojang").join("minecraft");
        if !minecraft_root.is_dir() {
            continue;
        }
        for version_dir in std::fs::read_dir(&minecraft_root)? {
            let version_dir = version_dir?.path();
            if version_dir.is_dir() {
                collect_direct_jars(&version_dir, &mut jar_paths)?;
            }
        }
    }
    if let Some(version) = expected_version {
        for profile_path in &layout.profile_paths {
            if let Some(versions_root) =
                profile_path.parent().and_then(Path::parent).filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.eq_ignore_ascii_case("versions"))
                })
            {
                collect_direct_jars(&versions_root.join(version), &mut jar_paths)?;
            }
        }
    }
    jar_paths.sort();
    jar_paths.dedup();

    let mut versions = Vec::new();
    for path in jar_paths {
        let Ok(version) = read_version_json_from_jar(&path) else {
            continue;
        };
        if expected_version.is_some_and(|expected| expected != version.id) {
            continue;
        }
        if !versions
            .iter()
            .any(|existing: &crate::metadata::mojang::McVersion| existing.id == version.id)
        {
            versions.push(version);
        }
    }
    versions.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(versions)
}

fn collect_direct_jars(
    directory: &Path,
    paths: &mut Vec<std::path::PathBuf>,
) -> Result<(), OrbitError> {
    if !directory.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("jar"))
        {
            paths.push(path);
        }
    }
    Ok(())
}

/// 从游戏 JAR 中提取 version.json
pub(crate) fn read_version_json_from_jar(
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
    interaction: crate::installer::InstallInteraction,
) -> Result<InitOutput, OrbitError> {
    if input.instance_dir.join("orbit.toml").exists() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "orbit.toml already exists in this directory; use 'orbit sync' to reconcile it"
        )));
    }

    // Platform discovery is the validity gate for an instance. The caller's
    // values select a candidate; the paths and metadata always come from disk.
    let platform = crate::platform::discover_platform_for_init(
        &input.instance_dir,
        &input.mc_version,
        &input.modloader,
        &input.modloader_version,
    )?;
    let platform_artifacts = platform.artifacts(&input.instance_dir)?;

    // 1. 扫描 mods/
    let scanned = scan_mods_dir(&input.instance_dir, &platform.loader)?;

    // 2. Identify top-level package JARs. Modules contained in one package are
    // already represented by the JAR layer as bundled metadata.
    let identified = crate::identification::identify_mods(&scanned, providers).await?;

    // 3. Build root declarations and concrete top-level package candidates.
    let lock_entries: Vec<crate::lockfile::PackageEntry> = identified
        .iter()
        .map(crate::identification::IdentifiedMod::to_package_entry)
        .collect();

    let mc_ver = platform.minecraft_version.id.clone();
    let loader_name = platform.loader.clone();
    let loader_ver = platform.loader_version.clone();
    let mut dependencies = indexmap::IndexMap::new();
    for m in &identified {
        let key = m.package_id();
        let spec = DependencySpec::Full {
            version: Some("*".to_string()),
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
        platform: platform_artifacts,
        resolver: ResolverConfig::default(),
        dependencies,
        groups: Default::default(),
        overrides: Default::default(),
    };

    // 4. Resolve the local candidate set through the same portfolio path used
    // by sync. Duplicate files for one mod_id are versions of one package.
    let crate::installer::InstallInteraction {
        select_package: _,
        select_resolution,
        confirm_install,
        progress: _,
    } = interaction;
    let loader_package = platform.loader_package;
    let (selected_lock_entries, removed, dependency_error) =
        match crate::package_reconciliation::select_local_packages(
            &manifest,
            &lock_entries,
            loader_package,
            select_resolution,
        )
        .await
        {
            Ok(selection) => {
                if confirm_install.is_some_and(|confirm| {
                    !confirm(&crate::package_reconciliation::confirmation_report(
                        &selection,
                    ))
                }) {
                    return Err(OrbitError::Other(anyhow::anyhow!(
                        "initialization cancelled before removing unselected package versions"
                    )));
                }
                (selection.selected_entries, selection.removed, None)
            }
            Err(error) => (lock_entries, Vec::new(), Some(error)),
        };

    // 5. Write the selected package graph. If the graph is incomplete, retain
    // every local file and report the resolver error without guessing a cleanup.
    let lockfile = crate::lockfile::OrbitLockfile {
        meta: crate::lockfile::LockMeta {
            mc_version: mc_ver,
            modloader: loader_name,
            modloader_version: loader_ver,
        },
        packages: selected_lock_entries,
    };
    let locked_packages = lockfile.packages.len();

    let manifest_file = crate::workspace::ManifestFile::new(&input.instance_dir, manifest.clone());
    let lock = crate::workspace::Lockfile::new(&input.instance_dir, lockfile);
    if !input.dry_run {
        manifest_file.save()?;
        lock.save()?;
        crate::package_reconciliation::remove_unselected_packages(&input.instance_dir, &removed)?;
    }

    Ok(InitOutput {
        manifest,
        scanned_mods: scanned,
        removed,
        locked_packages,
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

    #[test]
    fn scan_mods_dir_rejects_a_top_level_jar_without_loader_metadata() {
        let instance = temp_instance_dir("invalid-top-level-package");
        let mods_dir = instance.join("mods");
        std::fs::create_dir_all(&mods_dir).unwrap();
        write_jar(
            &mods_dir.join("library-only.jar"),
            &jar_bytes(&[("example.txt", b"not a mod")]),
        );

        let error = scan_mods_dir(&instance, "fabric").unwrap_err();

        assert!(error.to_string().contains("top-level package"));
        assert!(error.to_string().contains("fabric"));
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
            dry_run: false,
        };

        let error =
            match run_init(input, &[], crate::installer::InstallInteraction::default()).await {
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
    async fn init_confirms_and_removes_unselected_versions_of_one_package() {
        let directory = temp_instance_dir("duplicate-packages");
        crate::platform::test_support::write_platform(&directory, "1.20.1", "fabric", "0.16.10");
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

        let preview = run_init(
            input(true),
            &[],
            crate::installer::InstallInteraction::default(),
        )
        .await
        .unwrap();
        assert_eq!(preview.removed[0].filename, "alpha-1.jar");
        assert!(!directory.join("orbit.toml").exists());
        assert!(mods.join("alpha-1.jar").exists());

        let error = match run_init(
            input(false),
            &[],
            crate::installer::InstallInteraction {
                select_package: None,
                select_resolution: None,
                confirm_install: Some(Box::new(|report| {
                    assert_eq!(report.removed.len(), 1);
                    assert_eq!(report.removed[0].filename, "alpha-1.jar");
                    false
                })),
                progress: None,
            },
        )
        .await
        {
            Ok(_) => panic!("init unexpectedly ignored rejected package cleanup"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("initialization cancelled"));
        assert!(!directory.join("orbit.toml").exists());
        assert!(mods.join("alpha-1.jar").exists());

        let output = run_init(
            input(false),
            &[],
            crate::installer::InstallInteraction {
                select_package: None,
                select_resolution: None,
                confirm_install: Some(Box::new(|_| true)),
                progress: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(output.locked_packages, 1);
        assert_eq!(output.removed[0].filename, "alpha-1.jar");
        assert!(!mods.join("alpha-1.jar").exists());
        assert!(mods.join("alpha-2.jar").exists());
        assert_eq!(
            crate::workspace::Lockfile::open(&directory)
                .unwrap()
                .inner
                .find("alpha")
                .unwrap()
                .version,
            "2"
        );
        assert_eq!(
            output.manifest.dependencies["alpha"].version_constraint(),
            Some("*")
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
