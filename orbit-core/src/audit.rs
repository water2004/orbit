//! Read-only assembly of the exact runtime inputs for bytecode audit.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use orbit_bytecode_audit::{
    AnalysisLimits, ArtifactInput, ArtifactKind, AuditEnvironment, AuditProgressEvent,
    AuditProgressReporter, AuditProgressStage, AuditReport, AuditRequest, NestedJarPolicy,
    PhysicalSide,
};

use crate::error::OrbitError;

pub fn audit_instance(instance_dir: &Path) -> Result<AuditReport, OrbitError> {
    audit_instance_with_progress(instance_dir, None)
}

pub fn audit_instance_with_progress(
    instance_dir: &Path,
    progress: Option<AuditProgressReporter>,
) -> Result<AuditReport, OrbitError> {
    let emit = |event| {
        if let Some(progress) = progress.as_ref() {
            progress(event);
        }
    };
    emit(AuditProgressEvent::StageStarted {
        stage: AuditProgressStage::PrepareInputs,
        total: Some(5),
    });
    let manifest = crate::manifest::OrbitManifest::from_dir(instance_dir)?;
    emit(AuditProgressEvent::Advanced {
        stage: AuditProgressStage::PrepareInputs,
        completed: 1,
        total: Some(5),
    });
    let discovered = crate::platform::rediscover_current_platform(instance_dir)?;
    emit(AuditProgressEvent::Advanced {
        stage: AuditProgressStage::PrepareInputs,
        completed: 2,
        total: Some(5),
    });
    let runtime = crate::platform::discover_runtime_classpath(instance_dir, &discovered)?;
    emit(AuditProgressEvent::Advanced {
        stage: AuditProgressStage::PrepareInputs,
        completed: 3,
        total: Some(5),
    });
    let lockfile = crate::lockfile::OrbitLockfile::from_dir(instance_dir)?;
    let selected_runtime = crate::resolver::selected_runtime_load(
        &manifest,
        &lockfile,
        discovered.loader_package.as_ref(),
        discovered.physical_environment,
    )
    .map_err(|error| {
        OrbitError::Conflict(format!(
            "cannot construct the Loader-selected runtime content for audit: {error}"
        ))
    })?;
    emit(AuditProgressEvent::Advanced {
        stage: AuditProgressStage::PrepareInputs,
        completed: 4,
        total: Some(5),
    });
    let labels = lockfile_labels(&lockfile);

    let mut artifacts = vec![
        ArtifactInput {
            id: "minecraft".to_string(),
            display_name: format!("Minecraft {}", discovered.minecraft_version.id),
            path: discovered.minecraft_jar.clone(),
            kind: ArtifactKind::Minecraft,
            nested_jars: NestedJarPolicy::None,
        },
        ArtifactInput {
            id: format!("loader:{}", discovered.loader),
            display_name: format!("{} {}", discovered.loader, discovered.loader_version),
            path: discovered.loader_jar.clone(),
            kind: ArtifactKind::Loader,
            nested_jars: if discovered.loader_package.is_some() {
                NestedJarPolicy::Selected(selected_runtime.loader_nested_jars.clone())
            } else {
                // Without parseable Loader metadata there is no trustworthy
                // archive selection to consume, so retain conservative
                // visibility rather than hiding unknown runtime code.
                NestedJarPolicy::All
            },
        },
    ];
    artifacts.extend(runtime.into_iter().enumerate().map(|(index, path)| {
        let display_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("runtime.jar")
            .to_string();
        ArtifactInput {
            id: format!("runtime:{index}:{display_name}"),
            display_name,
            path,
            kind: ArtifactKind::Runtime,
            nested_jars: NestedJarPolicy::None,
        }
    }));
    let known_package_files = lockfile
        .packages
        .iter()
        .filter_map(|package| (!package.filename.is_empty()).then_some(package.filename.as_str()))
        .collect::<std::collections::BTreeSet<_>>();
    artifacts.extend(
        mod_jars(instance_dir)?
            .into_iter()
            .filter(|path| {
                let Some(filename) = path.file_name().and_then(|name| name.to_str()) else {
                    return true;
                };
                !known_package_files.contains(filename)
                    || selected_runtime.top_level_jars.contains(filename)
            })
            .enumerate()
            .map(|(index, path)| {
                let filename = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("mod.jar")
                    .to_string();
                let display_name = labels
                    .get(&filename)
                    .cloned()
                    .unwrap_or_else(|| filename.clone());
                let nested_jars = selected_runtime
                    .nested_jars
                    .get(&filename)
                    .cloned()
                    .unwrap_or_default();
                ArtifactInput {
                    // The filename is evidence-neutral presentation data. The
                    // index keeps duplicate names from distinct paths separate.
                    id: format!("mod:{index}:{filename}"),
                    display_name,
                    path,
                    kind: ArtifactKind::Mod,
                    nested_jars: NestedJarPolicy::Selected(nested_jars),
                }
            }),
    );
    emit(AuditProgressEvent::Advanced {
        stage: AuditProgressStage::PrepareInputs,
        completed: 5,
        total: Some(5),
    });
    emit(AuditProgressEvent::StageFinished {
        stage: AuditProgressStage::PrepareInputs,
        completed: 5,
    });

    orbit_bytecode_audit::analyze_with_progress(
        &AuditRequest {
            environment: AuditEnvironment {
                minecraft_version: discovered.minecraft_version.id,
                declared_loader: manifest.project.modloader,
                detected_loader: discovered.loader,
                loader_version: discovered.loader_version,
                physical_side: match discovered.physical_environment {
                    crate::metadata::Environment::Client => PhysicalSide::Client,
                    crate::metadata::Environment::Server => PhysicalSide::DedicatedServer,
                    crate::metadata::Environment::Both => PhysicalSide::Unknown,
                },
                java_feature: discovered.minecraft_version.java_version,
            },
            artifacts,
            active_mod_ids: selected_runtime.active_mod_ids,
            limits: AnalysisLimits::default(),
        },
        progress.as_ref(),
    )
    .map_err(|error| match error {
        orbit_bytecode_audit::AuditError::NotReady(readiness) => {
            OrbitError::Other(anyhow::anyhow!(readiness.message))
        }
        error => OrbitError::Other(anyhow::anyhow!(error)),
    })
}

fn mod_jars(instance_dir: &Path) -> Result<Vec<PathBuf>, OrbitError> {
    let mods = instance_dir.join("mods");
    if !mods.is_dir() {
        return Ok(Vec::new());
    }
    let mut jars = std::fs::read_dir(mods)?
        .filter_map(|entry| match entry {
            Ok(entry) => {
                let path = entry.path();
                (path.is_file()
                    && path
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("jar")))
                .then_some(Ok(path))
            }
            Err(error) => Some(Err(OrbitError::Io(error))),
        })
        .collect::<Result<Vec<_>, _>>()?;
    jars.sort();
    Ok(jars)
}

fn lockfile_labels(lockfile: &crate::lockfile::OrbitLockfile) -> HashMap<String, String> {
    lockfile
        .packages
        .iter()
        .filter(|package| !package.filename.is_empty())
        .map(|package| {
            (
                package.filename.clone(),
                format!("{} {}", package.mod_id, package.version),
            )
        })
        .collect()
}
