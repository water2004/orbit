//! Manifest import and instance archive export.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;

use crate::error::OrbitError;
use crate::installer::RestoreOptions;
use crate::manifest::DependencySpec;
use crate::workspace::{Lockfile, ManifestFile};

mod mrpack;
pub use mrpack::import_mrpack;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportMergeStrategy {
    PreferExisting,
    PreferImport,
    Interactive,
}

#[derive(Debug, Clone, Default)]
pub struct ImportReport {
    pub added: Vec<String>,
    pub replaced: Vec<String>,
    pub kept: Vec<String>,
    pub extracted: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ExportReport {
    pub path: PathBuf,
    pub packages: usize,
    pub bytes: u64,
}

pub fn import_manifest<F>(
    instance_dir: &Path,
    source: &Path,
    strategy: ImportMergeStrategy,
    dry_run: bool,
    mut resolve_conflict: F,
) -> Result<ImportReport, OrbitError>
where
    F: FnMut(&str, &DependencySpec, &DependencySpec) -> Result<bool, OrbitError>,
{
    if !source.is_file() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "import file not found: {}",
            source.display()
        )));
    }
    let incoming = crate::manifest::OrbitManifest::from_path(source)?;
    let mut current = ManifestFile::open(instance_dir)?;
    let mut report = ImportReport::default();
    for (package, requirement) in incoming.dependencies {
        match current.inner.dependencies.get(&package) {
            None => {
                report.added.push(package.clone());
                current.inner.dependencies.insert(package, requirement);
            }
            Some(existing) if existing == &requirement => {
                report.kept.push(package);
            }
            Some(existing) => {
                let replace = match strategy {
                    ImportMergeStrategy::PreferExisting => false,
                    ImportMergeStrategy::PreferImport => true,
                    ImportMergeStrategy::Interactive => {
                        resolve_conflict(&package, existing, &requirement)?
                    }
                };
                if replace {
                    report.replaced.push(package.clone());
                    current.inner.dependencies.insert(package, requirement);
                } else {
                    report.kept.push(package);
                }
            }
        }
    }
    report.added.sort();
    report.replaced.sort();
    report.kept.sort();
    if !dry_run && (!report.added.is_empty() || !report.replaced.is_empty()) {
        current.save()?;
    }
    Ok(report)
}

pub fn import_archive(
    instance_dir: &Path,
    source: &Path,
    overwrite: bool,
    dry_run: bool,
) -> Result<ImportReport, OrbitError> {
    if !source.is_file() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "import archive not found: {}",
            source.display()
        )));
    }
    let file = std::fs::File::open(source)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let mods_dir = instance_dir.join("mods");
    let mut report = ImportReport::default();
    let mut total_size = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let Some(enclosed) = entry.enclosed_name() else {
            continue;
        };
        let is_jar = enclosed
            .extension()
            .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("jar"));
        let under_mods = enclosed.components().any(|component| {
            component
                .as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case("mods")
        });
        if !entry.is_file() || !is_jar || !under_mods {
            continue;
        }
        total_size = total_size.saturating_add(entry.size());
        if total_size > 4 * 1024 * 1024 * 1024 {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "archive contains more than 4 GiB of mod files"
            )));
        }
        let filename = enclosed
            .file_name()
            .ok_or_else(|| OrbitError::Other(anyhow::anyhow!("invalid JAR path in archive")))?
            .to_string_lossy()
            .into_owned();
        let destination = mods_dir.join(&filename);
        if destination.exists() && !overwrite {
            report.kept.push(filename);
            continue;
        }
        report.extracted.push(filename.clone());
        if dry_run {
            continue;
        }
        std::fs::create_dir_all(&mods_dir)?;
        let temporary = mods_dir.join(format!(".{filename}.importing"));
        let mut output = std::fs::File::create(&temporary)?;
        std::io::copy(&mut entry, &mut output)?;
        output.sync_all()?;
        if destination.exists() {
            std::fs::remove_file(&destination)?;
        }
        std::fs::rename(temporary, destination)?;
    }
    report.extracted.sort();
    report.kept.sort();
    Ok(report)
}

pub fn export_instance(
    instance_dir: &Path,
    output: &Path,
    target: Option<String>,
    format: &str,
    dry_run: bool,
) -> Result<ExportReport, OrbitError> {
    if !matches!(format, "zip" | "mrpack") {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "unsupported export format '{format}'; expected zip or mrpack"
        )));
    }
    let manifest = ManifestFile::open(instance_dir)?;
    let lockfile = Lockfile::open(instance_dir)?;
    let (selected, _) = crate::installer::selected_packages(
        &manifest.inner,
        &lockfile.inner,
        &RestoreOptions {
            target,
            ..RestoreOptions::default()
        },
    )?;
    let mut sources = Vec::new();
    for package in selected {
        let entry = lockfile.inner.find(&package).ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!(
                "orbit.lock is missing export package '{package}'"
            ))
        })?;
        if entry.filename.is_empty() {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "no filename recorded for export package '{package}'"
            )));
        }
        let source = instance_dir.join("mods").join(&entry.filename);
        if !source.is_file() {
            return Err(OrbitError::Other(anyhow::anyhow!(
                "JAR for export package '{package}' is missing: {}",
                source.display()
            )));
        }
        if !entry.sha256.is_empty() {
            let actual = crate::jar::compute_sha256(&source)?;
            if actual != entry.sha256 {
                return Err(OrbitError::ChecksumMismatch {
                    name: entry.filename.clone(),
                    expected: entry.sha256.clone(),
                    actual,
                });
            }
        }
        sources.push((entry, source));
    }
    let bytes = sources
        .iter()
        .map(|(_, path)| std::fs::metadata(path).map(|metadata| metadata.len()))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .sum();
    let report = ExportReport {
        path: output.to_path_buf(),
        packages: sources.len(),
        bytes,
    };
    if dry_run {
        return Ok(report);
    }
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = temporary_output_path(output);
    let file = std::fs::File::create(&temporary)?;
    let mut archive = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    if format == "mrpack" {
        mrpack::write_contents(
            &mut archive,
            options,
            &manifest.inner,
            &sources,
            instance_dir,
        )?;
    } else {
        add_file(
            &mut archive,
            "orbit.toml",
            &instance_dir.join("orbit.toml"),
            options,
        )?;
        add_file(
            &mut archive,
            "orbit.lock",
            &instance_dir.join("orbit.lock"),
            options,
        )?;
        for (entry, source) in &sources {
            add_file(
                &mut archive,
                &format!("mods/{}", entry.filename),
                source,
                options,
            )?;
        }
    }
    archive.finish()?;
    if output.exists() {
        std::fs::remove_file(output)?;
    }
    std::fs::rename(temporary, output)?;
    Ok(report)
}

fn add_file(
    archive: &mut zip::ZipWriter<std::fs::File>,
    archive_path: &str,
    source: &Path,
    options: SimpleFileOptions,
) -> Result<(), OrbitError> {
    archive.start_file(archive_path, options)?;
    let mut input = std::fs::File::open(source)?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        archive.write_all(&buffer[..read])?;
    }
    Ok(())
}

fn temporary_output_path(output: &Path) -> PathBuf {
    let filename = output
        .file_name()
        .map(|filename| filename.to_string_lossy())
        .unwrap_or_default();
    output.with_file_name(format!(".{filename}.tmp"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockfile::{FileInfo, LockMeta, ModrinthInfo, OrbitLockfile, PackageEntry};
    use crate::manifest::{OrbitManifest, ProjectMeta, ResolverConfig};
    use serde_json::json;

    fn test_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("orbit-archive-test-{name}-{}", std::process::id()))
    }

    fn manifest(dependency: &str) -> OrbitManifest {
        OrbitManifest {
            project: ProjectMeta {
                name: "test".to_string(),
                mc_version: "1".to_string(),
                modloader: "fabric".to_string(),
                modloader_version: "1".to_string(),
                description: None,
                authors: None,
                version: None,
            },
            resolver: ResolverConfig::default(),
            dependencies: indexmap::IndexMap::from([(
                dependency.to_string(),
                DependencySpec::Short("*".to_string()),
            )]),
            groups: indexmap::IndexMap::new(),
            overrides: indexmap::IndexMap::new(),
        }
    }

    #[test]
    fn manifest_import_respects_prefer_existing() {
        let directory = test_dir("manifest");
        std::fs::create_dir_all(&directory).unwrap();
        ManifestFile::new(&directory, manifest("existing"))
            .save()
            .unwrap();
        let source = directory.join("incoming.toml");
        let mut incoming = manifest("added");
        incoming.dependencies.insert(
            "existing".to_string(),
            DependencySpec::Short("2".to_string()),
        );
        std::fs::write(&source, incoming.to_toml_string().unwrap()).unwrap();

        let report = import_manifest(
            &directory,
            &source,
            ImportMergeStrategy::PreferExisting,
            false,
            |_, _, _| Ok(false),
        )
        .unwrap();

        assert_eq!(report.added, vec!["added"]);
        assert_eq!(
            ManifestFile::open(&directory).unwrap().inner.dependencies["existing"]
                .version_constraint(),
            Some("*")
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn archive_import_extracts_only_safe_mod_paths() {
        let directory = test_dir("zip-import");
        std::fs::create_dir_all(&directory).unwrap();
        let source = directory.join("pack.zip");
        let file = std::fs::File::create(&source).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        zip.start_file("mods/example.jar", options).unwrap();
        zip.write_all(b"example").unwrap();
        zip.start_file("../mods/escape.jar", options).unwrap();
        zip.write_all(b"escape").unwrap();
        zip.finish().unwrap();

        let report = import_archive(&directory, &source, false, false).unwrap();

        assert_eq!(report.extracted, vec!["example.jar"]);
        assert!(directory.join("mods/example.jar").is_file());
        assert!(!directory.join("escape.jar").exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn zip_export_contains_manifest_lock_and_selected_jar() {
        let directory = test_dir("zip-export");
        std::fs::create_dir_all(directory.join("mods")).unwrap();
        ManifestFile::new(&directory, manifest("example"))
            .save()
            .unwrap();
        let jar = directory.join("mods/example.jar");
        std::fs::write(&jar, b"example").unwrap();
        Lockfile::new(
            &directory,
            OrbitLockfile {
                meta: LockMeta {
                    mc_version: "1".to_string(),
                    modloader: "fabric".to_string(),
                    modloader_version: "1".to_string(),
                },
                packages: vec![PackageEntry {
                    mod_id: "example".to_string(),
                    version: "1".to_string(),
                    sha1: String::new(),
                    sha256: crate::jar::compute_sha256(&jar).unwrap(),
                    sha512: String::new(),
                    filename: "example.jar".to_string(),
                    provider: "file".to_string(),
                    modrinth: None,
                    curseforge: None,
                    file: Some(FileInfo {
                        path: "mods/example.jar".to_string(),
                    }),
                    dependencies: Vec::new(),
                    environment: crate::metadata::Environment::Both,
                    provides: Vec::new(),
                    language_loader: None,
                    embedded_artifacts: Vec::new(),
                    bundled: Vec::new(),
                }],
            },
        )
        .save()
        .unwrap();
        let output = directory.join("pack.zip");

        let report = export_instance(&directory, &output, None, "zip", false).unwrap();
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();

        assert_eq!(report.packages, 1);
        assert!(archive.by_name("orbit.toml").is_ok());
        assert!(archive.by_name("orbit.lock").is_ok());
        assert!(archive.by_name("mods/example.jar").is_ok());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn mrpack_references_online_files_and_embeds_local_overrides() {
        let directory = test_dir("mrpack-export");
        std::fs::create_dir_all(directory.join("mods")).unwrap();
        let mut project = manifest("online");
        project
            .dependencies
            .insert("local".to_string(), DependencySpec::Short("*".to_string()));
        project.dependencies.insert(
            "online".to_string(),
            DependencySpec::Full {
                version: Some("*".to_string()),
                optional: Some(true),
                env: Some("client".to_string()),
                exclude: None,
            },
        );
        ManifestFile::new(&directory, project).save().unwrap();
        let online_jar = directory.join("mods/online.jar");
        let local_jar = directory.join("mods/local.jar");
        std::fs::write(&online_jar, b"online").unwrap();
        std::fs::write(&local_jar, b"local").unwrap();
        let online_bytes = std::fs::read(&online_jar).unwrap();
        let local_bytes = std::fs::read(&local_jar).unwrap();
        Lockfile::new(
            &directory,
            OrbitLockfile {
                meta: LockMeta {
                    mc_version: "1".to_string(),
                    modloader: "fabric".to_string(),
                    modloader_version: "1".to_string(),
                },
                packages: vec![
                    PackageEntry {
                        mod_id: "online".to_string(),
                        version: "1".to_string(),
                        sha1: crate::jar::sha1_digest(&online_bytes),
                        sha256: crate::jar::sha256_digest(&online_bytes),
                        sha512: crate::jar::sha512_digest(&online_bytes),
                        filename: "online.jar".to_string(),
                        provider: "modrinth".to_string(),
                        modrinth: Some(ModrinthInfo {
                            project_id: "online-project".to_string(),
                            version_id: "online-version".to_string(),
                            version: "1".to_string(),
                            slug: "online".to_string(),
                            download_url: "https://cdn.modrinth.com/online.jar".to_string(),
                        }),
                        curseforge: None,
                        file: None,
                        dependencies: Vec::new(),
                        environment: crate::metadata::Environment::Both,
                        provides: Vec::new(),
                        language_loader: None,
                        embedded_artifacts: Vec::new(),
                        bundled: Vec::new(),
                    },
                    PackageEntry {
                        mod_id: "local".to_string(),
                        version: "1".to_string(),
                        sha1: crate::jar::sha1_digest(&local_bytes),
                        sha256: crate::jar::sha256_digest(&local_bytes),
                        sha512: crate::jar::sha512_digest(&local_bytes),
                        filename: "local.jar".to_string(),
                        provider: "file".to_string(),
                        modrinth: None,
                        curseforge: None,
                        file: Some(FileInfo {
                            path: "mods/local.jar".to_string(),
                        }),
                        dependencies: Vec::new(),
                        environment: crate::metadata::Environment::Both,
                        provides: Vec::new(),
                        language_loader: None,
                        embedded_artifacts: Vec::new(),
                        bundled: Vec::new(),
                    },
                ],
            },
        )
        .save()
        .unwrap();
        let output = directory.join("pack.mrpack");

        export_instance(&directory, &output, None, "mrpack", false).unwrap();
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();
        let index: serde_json::Value = {
            let mut entry = archive.by_name("modrinth.index.json").unwrap();
            serde_json::from_reader(&mut entry).unwrap()
        };

        assert_eq!(index["files"].as_array().unwrap().len(), 1);
        assert_eq!(index["files"][0]["path"], "mods/online.jar");
        assert_eq!(index["files"][0]["env"]["client"], "optional");
        assert_eq!(index["files"][0]["env"]["server"], "unsupported");
        assert!(archive.by_name("mods/online.jar").is_err());
        assert!(archive.by_name("overrides/mods/local.jar").is_ok());
        assert!(archive.by_name("overrides/orbit.toml").is_ok());
        assert!(archive.by_name("overrides/orbit.lock").is_ok());
        assert!(archive.by_name("orbit.toml").is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn mrpack_import_prefers_a_bundled_override_to_its_download_entry() {
        let directory = test_dir("mrpack-import-override");
        std::fs::create_dir_all(&directory).unwrap();
        let source = directory.join("input.mrpack");
        let file = std::fs::File::create(&source).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        archive.start_file("modrinth.index.json", options).unwrap();
        archive
            .write_all(
                serde_json::to_string(&json!({
                    "formatVersion": 1,
                    "game": "minecraft",
                    "versionId": "test",
                    "name": "test",
                    "files": [{
                        "path": "mods/bundled.jar",
                        "hashes": { "sha1": "unused", "sha512": "unused" },
                        "downloads": ["https://invalid.example/bundled.jar"],
                        "fileSize": 999
                    }],
                    "dependencies": { "minecraft": "1.21.1" }
                }))
                .unwrap()
                .as_bytes(),
            )
            .unwrap();
        archive
            .start_file("overrides/mods/bundled.jar", options)
            .unwrap();
        archive.write_all(b"bundled bytes").unwrap();
        archive.finish().unwrap();

        let report = import_mrpack(&directory, &source, false, false)
            .await
            .unwrap();

        assert_eq!(report.extracted, vec!["bundled.jar"]);
        assert_eq!(
            std::fs::read(directory.join("mods/bundled.jar")).unwrap(),
            b"bundled bytes"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn mrpack_import_rejects_unsafe_index_paths() {
        let directory = test_dir("mrpack-import-unsafe");
        std::fs::create_dir_all(&directory).unwrap();
        let source = directory.join("unsafe.mrpack");
        let file = std::fs::File::create(&source).unwrap();
        let mut archive = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        archive.start_file("modrinth.index.json", options).unwrap();
        archive
            .write_all(
                serde_json::to_string(&json!({
                    "formatVersion": 1,
                    "game": "minecraft",
                    "versionId": "test",
                    "name": "test",
                    "files": [{
                        "path": "../mods/escape.jar",
                        "hashes": { "sha1": "unused", "sha512": "unused" },
                        "downloads": ["https://cdn.modrinth.com/escape.jar"],
                        "fileSize": 1
                    }],
                    "dependencies": { "minecraft": "1.21.1" }
                }))
                .unwrap()
                .as_bytes(),
            )
            .unwrap();
        archive.finish().unwrap();

        let error = import_mrpack(&directory, &source, false, false)
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("unsafe path"));
        assert!(!directory.join("escape.jar").exists());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
