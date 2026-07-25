//! Installation workflow for `file:` dependencies.

use std::collections::HashSet;
use std::path::Path;

use crate::error::OrbitError;
use crate::lockfile::{BundledMod, FileInfo, LockMeta, PackageEntry};
use crate::progress::{
    ArtifactProgressState, ProgressEvent, ProgressReporter, emit as emit_progress,
};
use crate::providers::ModProvider;
use crate::workspace::{Lockfile, ManifestFile};

use super::{
    InstallInteraction, InstallOptions, InstallReport, InstalledMod, ensure_root_requirement,
    package_filename, package_is_present, package_removals, remove_packages, requested_requirement,
    resolve_missing_lock_entries, restore_package,
};

pub async fn install_local_file_to_instance(
    source: &Path,
    constraint: Option<&str>,
    instance_dir: &Path,
    providers: &[Box<dyn ModProvider>],
    jar_cache: &crate::jar_cache::JarCache,
    options: InstallOptions,
    interaction: InstallInteraction,
) -> Result<InstallReport, OrbitError> {
    let InstallInteraction {
        select_package: _,
        select_resolution,
        confirm_install,
        progress,
    } = interaction;
    let source = validate_source(source)?;
    let mut manifest = ManifestFile::open(instance_dir)?;
    let platform = crate::platform::discover_install_platform(
        instance_dir,
        &manifest.inner.project.mc_version,
    )?;
    crate::platform::apply_to_manifest(instance_dir, &mut manifest.inner, &platform)?;
    let loader = platform.loader.clone();
    let loader_package = platform.loader_package;
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

    let bundled: Vec<_> = metadata
        .bundled_mods
        .iter()
        .map(BundledMod::from_jar_metadata)
        .collect();
    lockfile.inner.packages.push(PackageEntry {
        mod_id: metadata.mod_id.clone(),
        version: metadata.version.clone(),
        sha1,
        sha256: sha256.clone(),
        sha512,
        filename: filename.clone(),
        provider: "file".to_string(),
        modrinth: None,
        curseforge: None,
        file: Some(FileInfo {
            path: format!("mods/{filename}"),
        }),
        dependencies: metadata.dependencies.clone(),
        environment: metadata.environment,
        provides: metadata.provides.clone(),
        language_loader: metadata.language_loader.clone(),
        embedded_artifacts: metadata.embedded_artifacts.clone(),
        bundled: bundled.clone(),
    });

    let resolution = resolve_dependencies(DependencyResolutionInput {
        manifest: &manifest,
        lockfile: &mut lockfile,
        providers,
        jar_cache,
        no_dependencies: options.no_deps,
        loader_package,
        selector: select_resolution,
        progress: progress.clone(),
    })
    .await?;
    let preview = build_preview(
        &metadata,
        &filename,
        PreviewContext {
            bundled,
            lockfile: &lockfile,
            no_dependencies: options.no_deps,
            resolution,
        },
    );
    if confirm_install.is_some_and(|prompt| !prompt(&preview)) {
        return Ok(InstallReport {
            installed: Vec::new(),
            removed: Vec::new(),
            changes: Vec::new(),
            already_satisfied: Vec::new(),
            skipped_optional: Vec::new(),
            diagnostics: preview.diagnostics,
            warnings: preview.warnings,
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
        planned_packages: preview
            .installed
            .iter()
            .map(|package| package.mod_id.clone())
            .collect(),
        providers,
        jar_cache,
        progress,
    };
    materialize_new_packages(materialize, &mut lockfile).await?;
    remove_packages(
        &instance_dir.join("mods"),
        &preview.removed,
        &preview.installed,
    )?;
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

struct DependencyResolutionInput<'a> {
    manifest: &'a ManifestFile,
    lockfile: &'a mut Lockfile,
    providers: &'a [Box<dyn ModProvider>],
    jar_cache: &'a crate::jar_cache::JarCache,
    no_dependencies: bool,
    loader_package: Option<crate::resolver::types::PlatformCandidate>,
    selector: Option<crate::resolver::types::ResolutionSelector>,
    progress: Option<ProgressReporter>,
}

async fn resolve_dependencies(
    input: DependencyResolutionInput<'_>,
) -> Result<crate::resolver::types::ResolutionReport, OrbitError> {
    let DependencyResolutionInput {
        manifest,
        lockfile,
        providers,
        jar_cache,
        no_dependencies,
        loader_package,
        selector,
        progress,
    } = input;
    if no_dependencies {
        return Ok(crate::resolver::types::ResolutionReport::default());
    }
    let resolution = resolve_missing_lock_entries(
        &manifest.inner,
        &mut lockfile.inner,
        providers,
        jar_cache,
        loader_package.clone(),
        selector,
        progress,
    )
    .await?;
    crate::resolver::check_lockfile_graph_with_loader(
        &manifest.inner,
        &lockfile.inner,
        loader_package.as_ref(),
    )
    .map_err(OrbitError::Conflict)?;
    Ok(resolution)
}

struct PreviewContext<'a> {
    bundled: Vec<BundledMod>,
    lockfile: &'a Lockfile,
    no_dependencies: bool,
    resolution: crate::resolver::types::ResolutionReport,
}

fn build_preview(
    metadata: &crate::jar::JarModMetadata,
    filename: &str,
    context: PreviewContext<'_>,
) -> InstallReport {
    let PreviewContext {
        bundled,
        lockfile,
        no_dependencies,
        resolution,
    } = context;
    let local = InstalledMod {
        candidate_id: None,
        slug: metadata.mod_id.clone(),
        mod_id: metadata.mod_id.clone(),
        version: metadata.version.clone(),
        filename: filename.to_string(),
        provider: "file".to_string(),
        modrinth: None,
        curseforge: None,
        dependencies: metadata.dependencies.clone(),
        environment: metadata.environment,
        provides: metadata.provides.clone(),
        language_loader: metadata.language_loader.clone(),
        embedded_artifacts: metadata.embedded_artifacts.clone(),
        bundled,
    };
    let mut planned = vec![local];
    if !no_dependencies {
        let selected_changes: HashSet<_> = resolution
            .changes
            .iter()
            .filter(|change| change.selected_version.is_some())
            .map(|change| change.package.as_str())
            .collect();
        planned.extend(
            lockfile
                .inner
                .packages
                .iter()
                .filter(|entry| {
                    entry.mod_id != metadata.mod_id
                        && selected_changes.contains(entry.mod_id.as_str())
                })
                .map(installed_mod_from_entry),
        );
    }
    planned.sort_by(|left, right| left.mod_id.cmp(&right.mod_id));
    let mut changes = resolution.changes;
    changes.push(crate::resolver::types::PackageChange {
        package: metadata.mod_id.clone(),
        current_version: None,
        selected_version: Some(metadata.version.clone()),
        filename: None,
        selected_filename: Some(filename.to_string()),
        kind: crate::resolver::types::PackageChangeKind::Install,
    });
    changes.sort_by(|left, right| left.package.cmp(&right.package));
    InstallReport {
        installed: planned,
        removed: package_removals(&changes),
        changes,
        already_satisfied: Vec::new(),
        skipped_optional: Vec::new(),
        diagnostics: resolution.diagnostics,
        warnings: resolution.warnings,
    }
}

fn installed_mod_from_entry(entry: &PackageEntry) -> InstalledMod {
    InstalledMod {
        candidate_id: None,
        slug: entry
            .source_slug()
            .map(str::to_string)
            .unwrap_or_else(|| entry.mod_id.clone()),
        mod_id: entry.mod_id.clone(),
        version: entry.version.clone(),
        filename: package_filename(entry),
        provider: entry.provider.clone(),
        modrinth: entry.modrinth.clone(),
        curseforge: entry.curseforge.clone(),
        dependencies: entry.dependencies.clone(),
        environment: entry.environment,
        provides: entry.provides.clone(),
        language_loader: entry.language_loader.clone(),
        embedded_artifacts: entry.embedded_artifacts.clone(),
        bundled: entry.bundled.clone(),
    }
}

struct LocalMaterialization<'a> {
    source: &'a Path,
    instance_dir: &'a Path,
    filename: &'a str,
    sha256: &'a str,
    package: &'a str,
    planned_packages: HashSet<String>,
    providers: &'a [Box<dyn ModProvider>],
    jar_cache: &'a crate::jar_cache::JarCache,
    progress: Option<ProgressReporter>,
}

async fn materialize_new_packages(
    input: LocalMaterialization<'_>,
    lockfile: &mut Lockfile,
) -> Result<(), OrbitError> {
    let mods_dir = input.instance_dir.join("mods");
    let total = input.planned_packages.len();
    let mut completed = 0;
    emit_progress(
        input.progress.as_ref(),
        ProgressEvent::ApplyStarted { total },
    );
    std::fs::create_dir_all(&mods_dir)?;
    emit_progress(
        input.progress.as_ref(),
        ProgressEvent::ApplyArtifact {
            completed,
            total,
            filename: input.filename.to_string(),
            state: ArtifactProgressState::Started,
        },
    );
    copy_local_jar(input.source, &mods_dir.join(input.filename), input.sha256)?;
    completed += 1;
    emit_progress(
        input.progress.as_ref(),
        ProgressEvent::ApplyArtifact {
            completed,
            total,
            filename: input.filename.to_string(),
            state: ArtifactProgressState::Finished,
        },
    );
    for entry in &mut lockfile.inner.packages {
        if entry.mod_id == input.package || !input.planned_packages.contains(&entry.mod_id) {
            continue;
        }
        let filename = package_filename(entry);
        emit_progress(
            input.progress.as_ref(),
            ProgressEvent::ApplyArtifact {
                completed,
                total,
                filename: filename.clone(),
                state: ArtifactProgressState::Started,
            },
        );
        let state = if !package_is_present(entry, &mods_dir)? {
            restore_package(
                entry,
                input.instance_dir,
                &mods_dir,
                input.providers,
                input.jar_cache,
                false,
            )
            .await?;
            ArtifactProgressState::Finished
        } else {
            ArtifactProgressState::AlreadyPresent
        };
        completed += 1;
        emit_progress(
            input.progress.as_ref(),
            ProgressEvent::ApplyArtifact {
                completed,
                total,
                filename,
                state,
            },
        );
    }
    lockfile
        .inner
        .packages
        .sort_by(|left, right| left.mod_id.cmp(&right.mod_id));
    emit_progress(
        input.progress.as_ref(),
        ProgressEvent::ApplyFinished { total },
    );
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
    use crate::lockfile::OrbitLockfile;
    use crate::manifest::OrbitManifest;
    use crate::{PackageChange, PackageChangeKind};

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
        crate::platform::test_support::write_platform(&directory, "1", "forge", "1");
        let manifest: OrbitManifest = toml::from_str(
            r#"
[project]
name = "test"
mc_version = "1"
modloader = "forge"
modloader_version = "1"
[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }
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
modLoader = "javafml"
loaderVersion = "[1,)"
license = "MIT"
[[mods]]
modId = "local_example"
version = "1.2.3"
displayName = "Local Example"
[[dependencies.local_example]]
modId = "forge"
mandatory = true
versionRange = "[1,)"
"#,
        )
        .unwrap();
        jar.finish().unwrap();
        let providers: Vec<Box<dyn ModProvider>> = Vec::new();
        let cache = crate::jar_cache::JarCache::open(directory.join(".test-cache")).unwrap();

        let report = install_local_file_to_instance(
            &source,
            None,
            &directory,
            &providers,
            &cache,
            InstallOptions {
                no_deps: true,
                optional: true,
                env: Some("client".to_string()),
                ..InstallOptions::default()
            },
            InstallInteraction::default(),
        )
        .await
        .unwrap();

        assert_eq!(report.installed.len(), 1);
        assert!(directory.join("mods").join("example.jar").is_file());
        let saved_manifest = ManifestFile::open(&directory).unwrap();
        let requirement = &saved_manifest.inner.dependencies["local_example"];
        assert_eq!(requirement.version_constraint(), Some("1.2.3"));
        assert_eq!(requirement.env(), Some("client"));
        assert!(requirement.optional());
        let lockfile = Lockfile::open(&directory).unwrap();
        let entry = lockfile.find("local_example").unwrap();
        assert_eq!(entry.provider, "file");
        assert_eq!(
            entry.file.as_ref().map(|file| file.path.as_str()),
            Some("mods/example.jar")
        );
        assert!(matches!(
            entry.dependencies[0],
            crate::metadata::DependencyExpression::Only(crate::metadata::ModDependency {
                ref id,
                ..
            }) if id == "forge"
        ));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn local_add_preview_includes_selected_changes_to_existing_packages() {
        let directory = test_directory("local-preview");
        let selected_dependency = PackageEntry {
            mod_id: "dependency".to_string(),
            version: "1".to_string(),
            sha1: String::new(),
            sha256: String::new(),
            sha512: String::new(),
            filename: "dependency-1.jar".to_string(),
            provider: "file".to_string(),
            modrinth: None,
            curseforge: None,
            file: Some(FileInfo {
                path: "mods/dependency-1.jar".to_string(),
            }),
            dependencies: Vec::new(),
            environment: crate::metadata::Environment::Both,
            provides: Vec::new(),
            language_loader: None,
            embedded_artifacts: Vec::new(),
            bundled: Vec::new(),
        };
        let lockfile = Lockfile::new(
            &directory,
            OrbitLockfile {
                meta: LockMeta {
                    mc_version: "1".to_string(),
                    modloader: "fabric".to_string(),
                    modloader_version: "1".to_string(),
                },
                packages: vec![selected_dependency],
            },
        );
        let metadata = crate::jar::JarModMetadata {
            mod_id: "local".to_string(),
            name: "Local".to_string(),
            version: "1".to_string(),
            environment: crate::metadata::Environment::Both,
            dependencies: Vec::new(),
            provides: Vec::new(),
            language_loader: None,
            load_condition: crate::metadata::ModLoadCondition::Always,
            origin: crate::jar::JarModOrigin::Root,
            embedded_jars: Vec::new(),
            embedded_artifacts: Vec::new(),
            bundled_mods: Vec::new(),
        };
        let resolution = crate::resolver::types::ResolutionReport {
            changes: vec![PackageChange {
                package: "dependency".to_string(),
                current_version: Some("2".to_string()),
                selected_version: Some("1".to_string()),
                filename: Some("dependency-2.jar".to_string()),
                selected_filename: Some("dependency-1.jar".to_string()),
                kind: PackageChangeKind::Downgrade,
            }],
            ..Default::default()
        };

        let preview = build_preview(
            &metadata,
            "local.jar",
            PreviewContext {
                bundled: Vec::new(),
                lockfile: &lockfile,
                no_dependencies: false,
                resolution,
            },
        );

        assert!(
            preview
                .installed
                .iter()
                .any(|package| { package.mod_id == "dependency" && package.version == "1" })
        );
        assert_eq!(preview.removed[0].filename, "dependency-2.jar");
        std::fs::remove_dir_all(directory).unwrap();
    }
}
