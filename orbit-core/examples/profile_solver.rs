//! Offline solver profile, with no downloads or instance modifications.
//!
//! `cargo run -p orbit-core --example profile_solver -- <packages> <runs> <max|min>`
//! Each package has two equal-numeric maximum versions, so the represented
//! solution space has 2^N assignments. The profile must not enumerate that product.
//! Timings include graph construction, factor proof and final assignment validation.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use orbit_core::lockfile::{LockMeta, OrbitLockfile, PackageEntry};
use orbit_core::manifest::{OrbitManifest, PackageSpec};
use orbit_core::metadata::Environment;
use orbit_core::progress::{ProgressEvent, ProgressReporter};
use orbit_core::resolver::types::{CandidateCatalog, CandidateVersion};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let count = args
        .first()
        .map_or(Ok(12), |value| value.parse::<usize>())?;
    let runs = args.get(1).map_or(Ok(5), |value| value.parse::<usize>())?;
    let minimize = match args.get(2).map(String::as_str).unwrap_or("max") {
        "max" => false,
        "min" => true,
        other => return Err(format!("unknown profile objective '{other}'").into()),
    };
    let mut manifest: OrbitManifest = toml::from_str(
        r#"
        [project]
        name = "offline-profile"
        mc_version = "1.21.1"
        modloader = "fabric"
        modloader_version = "0.19.2"
        [platform]
        minecraft_jar = { path = "minecraft.jar", sha256 = "profile" }
        loader_jar = { path = "loader.jar", sha256 = "profile" }
        runtime_jars = []
        physical_environment = "server"
    "#,
    )?;
    let mut lock = OrbitLockfile {
        meta: LockMeta {
            mc_version: "1.21.1".into(),
            modloader: "fabric".into(),
            modloader_version: "0.19.2".into(),
        },
        packages: Vec::new(),
    };
    let mut catalog = CandidateCatalog {
        java_feature: 21,
        loader_package: None,
        candidates: Default::default(),
        resolved: Default::default(),
        remote_packages: Default::default(),
        requested_packages: Default::default(),
        requested_remotes: Default::default(),
    };
    for index in 0..count {
        let id = format!("profile_{index}");
        manifest
            .packages
            .insert(id.clone(), PackageSpec::new("*", Vec::new()));
        lock.packages.push(PackageEntry {
            mod_id: id.clone(),
            version: "1.0.0".into(),
            filename: format!("{id}.jar"),
            sha1: String::new(),
            sha256: String::new(),
            sha512: String::new(),
            remotes: Vec::new(),
            artifact_sources: Vec::new(),
            dependencies: Vec::new(),
            environment: Environment::Both,
            provides: Vec::new(),
            language_loader: None,
            embedded_artifacts: Vec::new(),
            bundled: Vec::new(),
        });
        catalog.candidates.insert(
            id.clone(),
            ["1.0.0", "2.0.0-alpha", "2.0.0-beta"]
                .map(|version| CandidateVersion {
                    id: format!("{id}-{version}"),
                    filename: format!("{id}-{version}.jar"),
                    jar_version: version.into(),
                    display_sources: Vec::new(),
                    dependencies: Vec::new(),
                    environment: Environment::Both,
                    provides: Vec::new(),
                    language_loader: None,
                    embedded_artifacts: Vec::new(),
                    bundled: Vec::new(),
                })
                .to_vec(),
        );
    }
    for run in 0..runs {
        let metrics = Arc::new(Mutex::new(serde_json::Value::Null));
        let sink = Arc::clone(&metrics);
        let progress: ProgressReporter = Arc::new(move |event| {
            if matches!(event, ProgressEvent::ResolutionAdvanced { .. }) {
                *sink.lock().expect("profile metrics mutex") =
                    serde_json::to_value(event).expect("progress serialization");
            }
        });
        let start = Instant::now();
        let result = if minimize {
            orbit_core::resolver::resolve_minimal_change_portfolio_with_progress(
                &manifest,
                &lock,
                &catalog,
                Some(progress),
            )
            .await
        } else {
            orbit_core::resolver::resolve_candidate_portfolio_with_progress(
                &manifest,
                &lock,
                &catalog,
                Some(progress),
            )
            .await
        }
        .map_err(std::io::Error::other)?;
        let elapsed = start.elapsed();
        assert_eq!(result.alternatives.len(), 1);
        assert_eq!(result.alternatives[0].selected_versions.len(), count);
        println!(
            "{}",
            serde_json::json!({
                "run": run, "packages": count, "candidates": count * 3,
                "objective": if minimize { "min" } else { "max" },
                "elapsed_ms": elapsed.as_secs_f64() * 1000.0,
                "metrics": *metrics.lock().expect("profile metrics mutex"),
            })
        );
    }
    Ok(())
}
