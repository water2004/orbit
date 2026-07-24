//! Installation workflow for `file:` dependencies.

use std::collections::HashSet;
use std::path::Path;

use crate::error::OrbitError;
use crate::lockfile::{FileInfo, ImplantedMod, LockDependency, LockMeta, PackageEntry};
use crate::providers::ModProvider;
use crate::workspace::{Lockfile, ManifestFile};

use super::{
    InstallOptions, InstallPrompt, InstallReport, InstalledMod, ensure_root_requirement,
    package_filename, package_is_present, requested_requirement, resolve_missing_lock_entries,
    restore_package,
};

pub async fn install_local_file_to_instance(
    source: &Path,
    constraint: Option<&str>,
    instance_dir: &Path,
    providers: &[Box<dyn ModProvider>],
    options: InstallOptions,
    prompt_fn: Option<InstallPrompt>,
) -> Result<InstallReport, OrbitError> {
    let source = validate_source(source)?;
    let mut manifest = ManifestFile::open(instance_dir)?;
    let loader = manifest.inner.project.modloader.clone();
    let metadata = crate::jar::read_mod_metadata(&source, &loader)?;
    validate_metadata(&metadata, &loader, &manifest, instance_dir)?;

    let filename = source
        .file_name()
        .ok_or_else(|| OrbitError::Other(anyhow::anyhow!("local JAR has no filename")))?
        .to_string_lossy()
        .into_owned();
    let sha1 = crate::jar::compute_sha1(&source)?;
    let sha256 = crate::jar::compute_sha256(&source)?;
    let sha512 = crate::jar::compute_sha512(&source)?;
    let mut lockfile = Lockfile::open_or_default(
        instance_dir,
        LockMeta {
            mc_version: manifest.inner.project.mc_version.clone(),
            modloader: loader,
            modloader_version: manifest.inner.project.modloader_version.clone(),
        },
    );
    let requirement = requested_requirement(
        constraint
            .filter(|constraint| !constraint.trim().is_empty() && *constraint != "*")
            .unwrap_or(&metadata.version),
        options.optional,
        options.env.as_deref(),
    )?;
    ensure_root_requirement(&mut manifest.inner, &metadata.mod_id, requirement);

    let implanted = implanted_entries(&metadata.implanted_mods);
    lockfile.inner.packages.push(PackageEntry {
        mod_id: metadata.mod_id.clone(),
        version: metadata.version.clone(),
        sha1,
        sha256: sha256.clone(),
        sha512,
        filename: filename.clone(),
        provider: "file".to_string(),
        modrinth: None,
        file: Some(FileInfo {
            path: format!("mods/{filename}"),
        }),
        dependencies: dependency_entries(&metadata.dependencies),
        implanted: implanted.clone(),
    });

    let original_packages: HashSet<_> = lockfile
        .inner
        .packages
        .iter()
        .filter(|entry| entry.mod_id != metadata.mod_id)
        .map(|entry| entry.mod_id.clone())
        .collect();
    let diagnostics =
        resolve_dependencies(&manifest, &mut lockfile, providers, options.no_deps).await?;
    let preview = build_preview(
        &metadata,
        &filename,
        implanted,
        &lockfile,
        &original_packages,
        options.no_deps,
        diagnostics,
    );
    if prompt_fn.is_some_and(|prompt| !prompt(&preview)) {
        return Ok(InstallReport {
            installed: Vec::new(),
            already_satisfied: Vec::new(),
            skipped_optional: Vec::new(),
            diagnostics: preview.diagnostics,
        });
    }
    if options.dry_run {
        return Ok(preview);
    }

    let materialize = LocalMaterialization {
        source: &source,
        instance_dir,
        filename: &filename,
        sha256: &sha256,
        package: &metadata.mod_id,
        original_packages: &original_packages,
        providers,
    };
    materialize_new_packages(materialize, &mut lockfile).await?;
    manifest.save()?;
    lockfile.save()?;
    Ok(preview)
}

fn validate_source(source: &Path) -> Result<std::path::PathBuf, OrbitError> {
    if !source
        .extension()
        .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("jar"))
    {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "local mod must be a .jar file: {}",
            source.display()
        )));
    }
    let source = std::fs::canonicalize(source).map_err(|error| {
        OrbitError::Other(anyhow::anyhow!(
            "cannot open local mod {}: {error}",
            source.display()
        ))
    })?;
    if !source.is_file() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "local mod is not a file: {}",
            source.display()
        )));
    }
    Ok(source)
}

fn validate_metadata(
    metadata: &crate::jar::JarModMetadata,
    loader: &str,
    manifest: &ManifestFile,
    instance_dir: &Path,
) -> Result<(), OrbitError> {
    if metadata.mod_id.is_empty() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "local JAR has no mod id in its {loader} metadata"
        )));
    }
    if metadata.version.is_empty() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "local JAR '{}' has no version in its {loader} metadata",
            metadata.mod_id
        )));
    }
    if manifest.inner.dependencies.contains_key(&metadata.mod_id) {
        return Err(OrbitError::Conflict(format!(
            "'{}' already exists. Use 'orbit remove {}' before replacing a local file.",
            metadata.mod_id, metadata.mod_id
        )));
    }
    if Lockfile::open(instance_dir)
        .ok()
        .is_some_and(|lockfile| lockfile.inner.find(&metadata.mod_id).is_some())
    {
        return Err(OrbitError::Conflict(format!(
            "'{}' already exists in orbit.lock",
            metadata.mod_id
        )));
    }
    Ok(())
}

fn dependency_entries(dependencies: &[(String, String, bool)]) -> Vec<LockDependency> {
    dependencies
        .iter()
        .filter(|(_, _, required)| *required)
        .map(|(name, version, _)| LockDependency {
            name: name.clone(),
            version: if version.is_empty() {
                "*".to_string()
            } else {
                version.clone()
            },
        })
        .collect()
}

fn implanted_entries(implanted: &[crate::jar::JarModMetadata]) -> Vec<ImplantedMod> {
    implanted
        .iter()
        .map(|implanted| ImplantedMod {
            name: if implanted.mod_id.is_empty() {
                implanted.name.clone()
            } else {
                implanted.mod_id.clone()
            },
            version: implanted.version.clone(),
            sha256: String::new(),
            filename: String::new(),
            dependencies: dependency_entries(&implanted.dependencies),
        })
        .collect()
}

async fn resolve_dependencies(
    manifest: &ManifestFile,
    lockfile: &mut Lockfile,
    providers: &[Box<dyn ModProvider>],
    no_dependencies: bool,
) -> Result<Vec<crate::resolver::types::CandidateDiagnostic>, OrbitError> {
    if no_dependencies {
        return Ok(Vec::new());
    }
    let resolution =
        resolve_missing_lock_entries(&manifest.inner, &mut lockfile.inner, providers).await?;
    crate::resolver::check_lockfile_graph(&manifest.inner, &lockfile.inner)
        .map_err(OrbitError::Conflict)?;
    Ok(resolution.diagnostics)
}

fn build_preview(
    metadata: &crate::jar::JarModMetadata,
    filename: &str,
    implanted: Vec<ImplantedMod>,
    lockfile: &Lockfile,
    original_packages: &HashSet<String>,
    no_dependencies: bool,
    diagnostics: Vec<crate::resolver::types::CandidateDiagnostic>,
) -> InstallReport {
    let local = InstalledMod {
        slug: metadata.mod_id.clone(),
        mod_id: metadata.mod_id.clone(),
        version: metadata.version.clone(),
        filename: filename.to_string(),
        provider: "file".to_string(),
        project_id: String::new(),
        version_id: String::new(),
        modrinth_version: String::new(),
        download_url: String::new(),
        jar_deps: metadata.dependencies.clone(),
        implanted,
    };
    let mut planned = vec![local];
    if !no_dependencies {
        planned.extend(
            lockfile
                .inner
                .packages
                .iter()
                .filter(|entry| {
                    entry.mod_id != metadata.mod_id && !original_packages.contains(&entry.mod_id)
                })
                .map(installed_mod_from_entry),
        );
    }
    planned.sort_by(|left, right| left.mod_id.cmp(&right.mod_id));
    InstallReport {
        installed: planned,
        already_satisfied: Vec::new(),
        skipped_optional: Vec::new(),
        diagnostics,
    }
}

fn installed_mod_from_entry(entry: &PackageEntry) -> InstalledMod {
    let modrinth = entry.modrinth.as_ref();
    InstalledMod {
        slug: modrinth
            .map(|metadata| metadata.slug.clone())
            .unwrap_or_else(|| entry.mod_id.clone()),
        mod_id: entry.mod_id.clone(),
        version: entry.version.clone(),
        filename: package_filename(entry),
        provider: entry.provider.clone(),
        project_id: modrinth
            .map(|metadata| metadata.project_id.clone())
            .unwrap_or_default(),
        version_id: modrinth
            .map(|metadata| metadata.version_id.clone())
            .unwrap_or_default(),
        modrinth_version: modrinth
            .map(|metadata| metadata.version.clone())
            .unwrap_or_default(),
        download_url: modrinth
            .map(|metadata| metadata.download_url.clone())
            .unwrap_or_default(),
        jar_deps: entry
            .dependencies
            .iter()
            .map(|dependency| (dependency.name.clone(), dependency.version.clone(), true))
            .collect(),
        implanted: entry.implanted.clone(),
    }
}

struct LocalMaterialization<'a> {
    source: &'a Path,
    instance_dir: &'a Path,
    filename: &'a str,
    sha256: &'a str,
    package: &'a str,
    original_packages: &'a HashSet<String>,
    providers: &'a [Box<dyn ModProvider>],
}

async fn materialize_new_packages(
    input: LocalMaterialization<'_>,
    lockfile: &mut Lockfile,
) -> Result<(), OrbitError> {
    let mods_dir = input.instance_dir.join("mods");
    std::fs::create_dir_all(&mods_dir)?;
    copy_local_jar(input.source, &mods_dir.join(input.filename), input.sha256)?;
    for entry in &mut lockfile.inner.packages {
        if entry.mod_id == input.package || input.original_packages.contains(&entry.mod_id) {
            continue;
        }
        if !package_is_present(entry, &mods_dir)? {
            restore_package(entry, input.instance_dir, &mods_dir, input.providers, false).await?;
        }
    }
    lockfile
        .inner
        .packages
        .sort_by(|left, right| left.mod_id.cmp(&right.mod_id));
    Ok(())
}

fn copy_local_jar(
    source: &Path,
    destination: &Path,
    expected_sha256: &str,
) -> Result<(), OrbitError> {
    if destination.is_file() {
        let destination_sha256 = crate::jar::compute_sha256(destination)?;
        if destination_sha256 == expected_sha256 {
            return Ok(());
        }
        return Err(OrbitError::Conflict(format!(
            "destination '{}' already exists with different contents",
            destination.display()
        )));
    }
    let temporary = destination.with_extension("jar.orbit-tmp");
    std::fs::copy(source, &temporary)?;
    if let Err(error) = std::fs::rename(&temporary, destination) {
        let _ = std::fs::remove_file(&temporary);
        return Err(OrbitError::Io(error));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::manifest::OrbitManifest;

    static NEXT_TEST_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

    fn test_directory(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "orbit-installer-{name}-{}-{}",
            std::process::id(),
            NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[tokio::test]
    async fn local_file_add_copies_jar_and_records_metadata() {
        let directory = test_directory("local-add");
        let manifest: OrbitManifest = toml::from_str(
            r#"
[project]
name = "test"
mc_version = "1"
modloader = "forge"
modloader_version = "1"
"#,
        )
        .unwrap();
        ManifestFile::new(&directory, manifest).save().unwrap();
        let input_dir = directory.join("input");
        std::fs::create_dir_all(&input_dir).unwrap();
        let source = input_dir.join("example.jar");
        let file = std::fs::File::create(&source).unwrap();
        let mut jar = zip::ZipWriter::new(file);
        jar.start_file(
            "META-INF/mods.toml",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        jar.write_all(
            br#"
license = "MIT"
[[mods]]
modId = "local-example"
version = "1.2.3"
displayName = "Local Example"
[[dependencies.local-example]]
modId = "forge"
mandatory = true
versionRange = "[1,)"
"#,
        )
        .unwrap();
        jar.finish().unwrap();
        let providers: Vec<Box<dyn ModProvider>> = Vec::new();

        let report = install_local_file_to_instance(
            &source,
            None,
            &directory,
            &providers,
            InstallOptions {
                no_deps: true,
                optional: true,
                env: Some("client".to_string()),
                ..InstallOptions::default()
            },
            None,
        )
        .await
        .unwrap();

        assert_eq!(report.installed.len(), 1);
        assert!(directory.join("mods").join("example.jar").is_file());
        let saved_manifest = ManifestFile::open(&directory).unwrap();
        let requirement = &saved_manifest.inner.dependencies["local-example"];
        assert_eq!(requirement.version_constraint(), Some("1.2.3"));
        assert_eq!(requirement.env(), Some("client"));
        assert!(requirement.optional());
        let lockfile = Lockfile::open(&directory).unwrap();
        let entry = lockfile.find("local-example").unwrap();
        assert_eq!(entry.provider, "file");
        assert_eq!(
            entry.file.as_ref().map(|file| file.path.as_str()),
            Some("mods/example.jar")
        );
        assert_eq!(entry.dependencies[0].name, "forge");
        std::fs::remove_dir_all(directory).unwrap();
    }
}
