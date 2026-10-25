//! Cross-program contract checks. Only this test target links Launcher core;
//! Orbit's production adapter remains a wholly independent, read-only parser.
use super::{discover, format, minecraft_version};
use crate::loader::LoaderKind;
use crate::metadata::Environment;
use launcher::lockfile::{JavaProvider, LockedAssetIndex};
use orbit_launcher_core as launcher;
use std::io::Write;
use std::path::{Path, PathBuf};

struct Fixture {
    _root: tempfile::TempDir,
    instance: PathBuf,
    manifest: launcher::InstanceManifest,
    lock: launcher::LauncherLock,
}

fn jar(path: &Path, entries: &[(&str, &str)]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, body) in entries {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

impl Fixture {
    fn new(kind: launcher::InstanceKind, provider: JavaProvider) -> Self {
        let root = tempfile::tempdir().unwrap();
        let client = kind == launcher::InstanceKind::Client;
        let repository = root.path().join("minecraft");
        let instance = if client {
            repository.join("instances/test")
        } else {
            root.path().join("server")
        };
        std::fs::create_dir_all(&instance).unwrap();
        let artifacts_root = if client { &repository } else { &instance };
        let mc = if client {
            "instances/test/minecraft.jar"
        } else {
            "server.jar"
        };
        let loader = "libraries/net/fabricmc/fabric-loader/0.19.3/fabric-loader-0.19.3.jar";
        jar(
            &artifacts_root.join(mc),
            &[(
                "version.json",
                r#"{"id":"26.2","name":"26.2","world_version":5000,"protocol_version":800,"pack_version":{"resource_major":80,"resource_minor":0,"data_major":100,"data_minor":0},"java_version":25,"stable":true}"#,
            )],
        );
        jar(
            &artifacts_root.join(loader),
            &[(
                "fabric.mod.json",
                r#"{"schemaVersion":1,"id":"fabricloader","version":"0.19.3","name":"Fabric Loader"}"#,
            )],
        );
        let artifact = |path: &str, owner| launcher::LockedArtifact {
            logical_name: path.into(),
            owner,
            source: launcher::LockedArtifactSource::Download {
                url: "https://example.invalid/verified-artifact".into(),
                upstream_sha1: Some("a".repeat(40)),
            },
            sha256: launcher::hash_file_sha256(&artifacts_root.join(path)).unwrap(),
            size: std::fs::metadata(artifacts_root.join(path)).unwrap().len(),
            path: path.into(),
            native_extraction: None,
        };
        let mut artifacts = vec![
            artifact(mc, launcher::ArtifactOwner::Minecraft),
            artifact(loader, launcher::ArtifactOwner::Loader),
        ];
        if client {
            for (path, content) in [
                (
                    "assets/indexes/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.json",
                    "{}",
                ),
                ("libraries/authlib-injector.jar", "opaque injector"),
            ] {
                let target = artifacts_root.join(path);
                std::fs::create_dir_all(target.parent().unwrap()).unwrap();
                std::fs::write(target, content).unwrap();
                artifacts.push(artifact(
                    path,
                    if path.starts_with("assets/") {
                        launcher::ArtifactOwner::Minecraft
                    } else {
                        launcher::ArtifactOwner::AuthlibInjector
                    },
                ));
            }
        }
        let mut manifest = launcher::InstanceManifest::new(
            "f42a6bda-35dc-4ca7-8883-ec74814f0b8f".parse().unwrap(),
            "test",
            kind,
            "26.2",
            launcher::LoaderKind::Fabric,
            Some("0.19.3".into()),
        )
        .unwrap();
        manifest.icon = Some(launcher::presentation::INSTANCE_ICON.into());
        manifest.launch.java = Some(launcher::JavaSelection::Executable(
            root.path().join("local-java"),
        ));
        let lock = launcher::LauncherLock {
            schema: launcher::LOCK_SCHEMA,
            instance_id: manifest.id,
            kind,
            minecraft: launcher::LockedMinecraft {
                version: "26.2".into(),
                version_type: "release".into(),
                asset_index: client.then(|| LockedAssetIndex {
                    upstream_id: "32".into(),
                    runtime_name: "a".repeat(40),
                }),
                version_manifest_url: "https://example.invalid/manifest.json".into(),
                version_manifest_sha256: "b".repeat(64),
                version_json_url: "https://example.invalid/version.json".into(),
                version_json_sha1: "c".repeat(40),
            },
            loader: launcher::LockedLoader::profile(
                launcher::LoaderKind::Fabric,
                "0.19.3".into(),
                "https://example.invalid/loader.json".into(),
                "d".repeat(64),
            ),
            java: Some(launcher::LockedJavaRuntime {
                runtime_id: "test-java".into(),
                provider,
                version: "25.0.1".into(),
                major: 25,
                platform: "test".into(),
                executable: if provider == JavaProvider::System {
                    root.path()
                        .join("local-java")
                        .to_string_lossy()
                        .into_owned()
                } else {
                    "bin/java".into()
                },
            }),
            authlib_injector: client.then(|| launcher::LockedAuthlibInjector {
                version: "1.2.8".into(),
                build_number: 1,
                path: "libraries/authlib-injector.jar".into(),
            }),
            entrypoint: launcher::LockedEntrypoint::Classpath {
                main_class: if client {
                    "net.fabricmc.loader.impl.launch.knot.KnotClient"
                } else {
                    "net.fabricmc.loader.impl.launch.knot.KnotServer"
                }
                .into(),
                classpath: vec![loader.into(), mc.into()],
            },
            arguments: launcher::LockedArguments::default(),
            artifacts,
            generated_files: vec![],
            eula: (!client).then(|| launcher::EulaAcceptance {
                url: launcher::MINECRAFT_EULA_URL.into(),
                digest_sha256: "e".repeat(64),
                accepted_at_unix_seconds: 1,
                method: launcher::EulaAcceptanceMethod::DigestCommand,
            }),
        };
        let fixture = Self {
            _root: root,
            instance,
            manifest,
            lock,
        };
        fixture.save();
        fixture
    }
    fn save(&self) {
        launcher::ManifestFile::new(&self.instance, self.manifest.clone())
            .save()
            .unwrap();
        launcher::LockFile::new(&self.instance, self.lock.clone())
            .save()
            .unwrap();
    }
    fn bytes(&self) -> [Vec<u8>; 2] {
        [format::MANIFEST_FILE, format::LOCK_FILE]
            .map(|name| std::fs::read(self.instance.join(name)).unwrap())
    }
    fn set_schema(&self, file: &str, schema: u32) {
        let path = self.instance.join(file);
        let mut document = std::fs::read_to_string(&path)
            .unwrap()
            .parse::<toml_edit::DocumentMut>()
            .unwrap();
        document["schema"] = toml_edit::value(i64::from(schema));
        std::fs::write(path, document.to_string()).unwrap();
    }
    fn verify(&self) {
        let before = self.bytes();
        let runtime = discover(
            &self.instance,
            Some("26.2"),
            Some(LoaderKind::Fabric),
            Some("0.19.3"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(runtime.minecraft_version.id, "26.2");
        assert_eq!(
            runtime.physical_environment,
            if self.manifest.kind == launcher::InstanceKind::Client {
                Environment::Client
            } else {
                Environment::Server
            }
        );
        assert_eq!(
            minecraft_version(&self.instance).unwrap().unwrap().id,
            "26.2"
        );
        assert_eq!(
            self.bytes(),
            before,
            "Orbit discovery must never upgrade Launcher files"
        );
        assert!(
            !std::fs::read_dir(&self.instance)
                .unwrap()
                .flatten()
                .any(|e| e.path().extension().is_some_and(|v| v == "bak"))
        );
    }
}

#[test]
fn actual_launcher_writers_are_readable_with_both_java_providers_and_physical_sides() {
    for kind in [
        launcher::InstanceKind::Client,
        launcher::InstanceKind::Server,
    ] {
        for provider in [JavaProvider::System, JavaProvider::Mojang] {
            Fixture::new(kind, provider).verify();
        }
    }
}

#[tokio::test]
async fn composed_bundle_migrates_world_and_selected_package_data_without_overlap() {
    use crate::runtime_data::{
        DataOwnershipEntry, DataOwnershipLedger, OwnedDataKind, OwnedDataPath,
    };
    for kind in [
        launcher::InstanceKind::Client,
        launcher::InstanceKind::Server,
    ] {
        let mut source = Fixture::new(kind, JavaProvider::System);
        let mut target = Fixture::new(kind, JavaProvider::System);
        source.manifest.icon = None;
        target.manifest.icon = None;
        source.save();
        target.manifest.minecraft.requirement = "26.3".into();
        target.lock.minecraft.version = "26.3".into();
        let artifacts = if kind == launcher::InstanceKind::Client {
            target._root.path().join("minecraft")
        } else {
            target.instance.clone()
        };
        let mc = target
            .lock
            .artifacts
            .iter_mut()
            .find(|file| {
                file.path.ends_with(".jar") && file.owner == launcher::ArtifactOwner::Minecraft
            })
            .unwrap();
        let path = artifacts.join(&mc.path);
        jar(
            &path,
            &[(
                "version.json",
                r#"{"id":"26.3","name":"26.3","world_version":5001,"protocol_version":801,"pack_version":{"resource_major":80,"resource_minor":0,"data_major":100,"data_minor":0},"java_version":25,"stable":true}"#,
            )],
        );
        mc.sha256 = launcher::hash_file_sha256(&path).unwrap();
        mc.size = std::fs::metadata(path).unwrap().len();
        target.save();
        for (id, requirement) in [("kept", ">=26.1"), ("removed", "=26.2")] {
            jar(
                &source.instance.join(format!("mods/{id}.jar")),
                &[(
                    "fabric.mod.json",
                    &format!(
                        r#"{{"schemaVersion":1,"id":"{id}","version":"1","depends":{{"minecraft":"{requirement}"}}}}"#
                    ),
                )],
            );
        }
        crate::init::run_init(
            crate::init::InitInput {
                name: "Source".into(),
                mc_version: "26.2".into(),
                modloader: "fabric".into(),
                modloader_version: "0.19.3".into(),
                instance_dir: source.instance.clone(),
                dry_run: false,
            },
            &[],
        )
        .await
        .unwrap();
        let world = if kind == launcher::InstanceKind::Client {
            "saves/buildings"
        } else {
            "custom/world"
        };
        if kind == launcher::InstanceKind::Server {
            for fixture in [&source, &target] {
                std::fs::write(
                    fixture.instance.join("server.properties"),
                    "level-name=custom/world\n",
                )
                .unwrap();
            }
        }
        let mut entries = Vec::new();
        for (owner, directory) in [("kept", "syncmatica"), ("removed", "removed")] {
            let relative = format!("{world}/{directory}");
            std::fs::create_dir_all(source.instance.join(&relative)).unwrap();
            std::fs::write(source.instance.join(&relative).join("config.json"), owner).unwrap();
            entries.push(DataOwnershipEntry {
                path: OwnedDataPath::Instance { relative },
                kind: OwnedDataKind::Tree,
                owner: Some(owner.into()),
            });
        }
        std::fs::write(
            source.instance.join(world).join("level.dat"),
            b"world state",
        )
        .unwrap();
        crate::runtime_data::save_ledger(
            &source.instance,
            &DataOwnershipLedger::from_entries(entries),
        )
        .unwrap();
        let pack = source._root.path().join("migration.orbitbundle");
        crate::archive::export_instance(
            &source.instance,
            &pack,
            None,
            "orbit",
            crate::archive::ExportContent::ModsAndData,
            false,
            None,
        )
        .unwrap();
        launcher::state_archive::export_launcher_state_with_base(
            &source.instance,
            Some(&pack),
            &pack,
            |_| {},
        )
        .unwrap();
        launcher::state_archive::restore_launcher_state(&target.instance, &pack, |_| {}).unwrap();
        let portable = crate::archive::extract_portable_instance(&pack).unwrap();
        let cache = crate::jar_cache::JarCache::open(source._root.path().join("cache")).unwrap();
        let repository = crate::version_repository::VersionRepository::open(
            source._root.path().join("repository"),
        )
        .unwrap();
        let plan = crate::migration::plan_migration_from_portable(
            portable,
            &target.instance,
            &[],
            crate::version_repository::CandidateStorage::new(&cache, &repository),
            crate::migration::MigrationOptions {
                allow_package_removals: true,
            },
            crate::migration::MigrationInteraction::default(),
        )
        .await
        .unwrap();
        assert!(plan.target_lockfile().find("kept").is_some());
        assert!(plan.target_lockfile().find("removed").is_none());
        crate::migration::export_migration(&plan, true).unwrap();
        crate::migration::export_migration(&plan, false).unwrap();
        assert_eq!(
            std::fs::read(target.instance.join(world).join("syncmatica/config.json")).unwrap(),
            b"kept"
        );
        assert_eq!(
            std::fs::read(target.instance.join(world).join("level.dat")).unwrap(),
            b"world state"
        );
        assert!(
            !target
                .instance
                .join(world)
                .join("removed/config.json")
                .exists()
        );
        assert!(
            source
                .instance
                .join(world)
                .join("removed/config.json")
                .is_file()
        );
        assert_eq!(
            crate::runtime_data::effective_owner_for_relative(
                &crate::runtime_data::load_ledger(&target.instance)
                    .unwrap()
                    .entries,
                Path::new(&format!("{world}/syncmatica/config.json"))
            ),
            Some("kept".into())
        );
    }
}

#[test]
fn released_and_partially_upgraded_storage_use_one_read_only_projection() {
    let mut fixture = Fixture::new(launcher::InstanceKind::Client, JavaProvider::Mojang);
    // These fields were not part of the published v0.7.0 document.
    fixture.manifest.icon = None;
    fixture.manifest.launch.java = None;
    fixture.save();
    for manifest in [1, 2] {
        for lock in [6, 7] {
            fixture.set_schema(format::MANIFEST_FILE, manifest);
            fixture.set_schema(format::LOCK_FILE, lock);
            fixture.verify();
        }
    }
    // Let Launcher perform its own real migration; Orbit must read that output too.
    fixture.set_schema(format::MANIFEST_FILE, 1);
    fixture.set_schema(format::LOCK_FILE, 6);
    launcher::ManifestFile::open(&fixture.instance).unwrap();
    launcher::LockFile::open(&fixture.instance).unwrap();
    discover(&fixture.instance, None, None, None)
        .unwrap()
        .unwrap();
}

#[test]
fn unknown_schemas_are_rejected_without_reset_migration_or_fallback() {
    for (file, schemas) in [
        (format::MANIFEST_FILE, vec![0, 3, u32::MAX]),
        (format::LOCK_FILE, vec![0, 5, 8, u32::MAX]),
    ] {
        let fixture = Fixture::new(launcher::InstanceKind::Client, JavaProvider::System);
        for schema in schemas {
            fixture.set_schema(file, schema);
            let before = fixture.bytes();
            let error = discover(&fixture.instance, None, None, None)
                .err()
                .unwrap()
                .to_string();
            assert!(
                error.contains("schema") && error.contains("unsupported"),
                "{error}"
            );
            assert_eq!(fixture.bytes(), before);
        }
    }
}

#[test]
fn actual_writer_output_still_requires_matching_identity_and_verified_runtime_bytes() {
    let mut fixture = Fixture::new(launcher::InstanceKind::Client, JavaProvider::System);
    fixture.lock.instance_id = "ea0b515c-4df9-4d43-9637-dab49f21de90".parse().unwrap();
    fixture.save();
    assert!(
        discover(&fixture.instance, None, None, None)
            .err()
            .unwrap()
            .to_string()
            .contains("different instances")
    );
    fixture.lock.instance_id = fixture.manifest.id;
    fixture.save();
    std::fs::write(fixture.instance.join("minecraft.jar"), "modified").unwrap();
    assert!(
        discover(&fixture.instance, None, None, None)
            .err()
            .unwrap()
            .to_string()
            .contains("SHA-256")
    );
}

#[test]
fn io_errors_do_not_masquerade_as_an_absent_launcher_runtime() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join(format::MANIFEST_FILE)).unwrap();
    assert!(minecraft_version(root.path()).is_err());
}
