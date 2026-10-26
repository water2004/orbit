//! On-disk upgrades live here, never in runtime business paths.
//! Only published, understood source schemas are accepted. Upgrades preserve
//! comments, validate the complete new document, and keep the exact old bytes.
use std::io::Write;
use std::path::Path;

use fs2::FileExt;
use sha2::{Digest, Sha256};
use toml_edit::{DocumentMut, value};

use crate::atomic_io::write_atomic;
use crate::error::LauncherError;

#[derive(Clone, Copy)]
pub(crate) enum DocumentKind {
    Config,
    Instance,
    Lock,
}
impl DocumentKind {
    fn schemas(self) -> (i64, i64) {
        match self {
            // Baselines are the formats shipped in v0.7.0, not development snapshots.
            Self::Config => (2, crate::config::CONFIG_SCHEMA.into()),
            Self::Instance => (
                orbit_launcher_format::V0_7_MANIFEST_SCHEMA.into(),
                crate::instance::INSTANCE_SCHEMA.into(),
            ),
            Self::Lock => (
                orbit_launcher_format::V0_7_LOCK_SCHEMA.into(),
                crate::lockfile::LOCK_SCHEMA.into(),
            ),
        }
    }
    fn steps(self) -> &'static [MigrationStep] {
        match self {
            Self::Config => &[MigrationStep {
                from: 2,
                to: 3,
                apply: config_v2,
            }],
            Self::Instance => &[
                MigrationStep {
                    from: 1,
                    to: 2,
                    apply: instance_v1,
                },
                MigrationStep {
                    from: 2,
                    to: 3,
                    apply: instance_v2,
                },
            ],
            Self::Lock => &[MigrationStep {
                from: 6,
                to: 7,
                apply: lock_v6,
            }],
        }
    }
    fn validate(self, text: &str) -> Result<(), LauncherError> {
        match self {
            Self::Config => toml::from_str::<crate::config::GlobalConfig>(text)
                .map_err(LauncherError::ConfigParse)?
                .validate(),
            Self::Instance => toml::from_str::<crate::instance::InstanceManifest>(text)
                .map_err(LauncherError::ManifestParse)?
                .validate(),
            Self::Lock => toml::from_str::<crate::lockfile::LauncherLock>(text)
                .map_err(LauncherError::LockParse)?
                .validate(),
        }
    }
}

struct MigrationStep {
    from: i64,
    to: i64,
    apply: fn(&mut DocumentMut) -> Result<(), LauncherError>,
}
fn instance_v2(document: &mut DocumentMut) -> Result<(), LauncherError> {
    let mut launch = match document.remove("launch") {
        None => toml_edit::Table::new(),
        Some(toml_edit::Item::Table(table)) => table,
        Some(toml_edit::Item::Value(toml_edit::Value::InlineTable(table))) => table.into_table(),
        _ => {
            return Err(LauncherError::InvalidManifest(
                "launch must be a table".into(),
            ));
        }
    };
    if launch.contains_key("memory") {
        return Err(LauncherError::InvalidManifest(
            "schema 2 did not support launch.memory; original file was not changed".into(),
        ));
    }
    let defaults = crate::instance::MemoryConfig::default();
    let mut memory = toml_edit::Table::new();
    for (old, new, default) in [
        ("min_memory_mib", "min_mib", defaults.min_mib),
        ("max_memory_mib", "max_mib", defaults.max_mib),
    ] {
        if let Some((key, item)) = launch.remove_entry(old) {
            let renamed = toml_edit::Key::new(new).with_leaf_decor(key.leaf_decor().clone());
            memory.insert_formatted(&renamed, item);
        } else {
            memory.insert(new, value(i64::from(default)));
        }
    }
    if let Some(arguments) = launch.get_mut("jvm_args") {
        let arguments = arguments.as_array_mut().ok_or_else(|| {
            LauncherError::InvalidManifest("launch.jvm_args must be an array".into())
        })?;
        let mut removed = Vec::new();
        for (index, argument) in arguments.iter().enumerate() {
            if let Some((field, size)) = argument.as_str().and_then(crate::instance::heap_option) {
                // Preserve the old command-line precedence: the last explicit
                // JVM heap argument overrode the old scalar config fields.
                let (number, multiplier) = match size.as_bytes().last() {
                    Some(b'k' | b'K') => (&size[..size.len() - 1], 1024_u64),
                    Some(b'm' | b'M') => (&size[..size.len() - 1], 1024 * 1024),
                    Some(b'g' | b'G') => (&size[..size.len() - 1], 1024 * 1024 * 1024),
                    _ => (size, 1),
                };
                let mib = number.parse::<u64>().ok().and_then(|number| number.checked_mul(multiplier))
                    .filter(|bytes| *bytes > 0 && bytes % (1024 * 1024) == 0)
                    .map(|bytes| bytes / (1024 * 1024)).and_then(|mib| u32::try_from(mib).ok())
                    .ok_or_else(|| LauncherError::InvalidManifest("legacy JVM heap option cannot be represented in whole MiB; original file was not changed".into()))?;
                memory[field] = value(i64::from(mib));
                removed.push(index);
            }
        }
        for index in removed.into_iter().rev() {
            arguments.remove(index);
        }
    }
    launch.insert("memory", toml_edit::Item::Table(memory));
    document.insert("launch", toml_edit::Item::Table(launch));
    Ok(())
}
fn instance_v1(document: &mut DocumentMut) -> Result<(), LauncherError> {
    if document.contains_key("icon")
        || document
            .get("launch")
            .and_then(|item| item.get("java"))
            .is_some()
    {
        return Err(LauncherError::InvalidManifest("schema 1 did not support instance icons or Java overrides; original file was not changed".into()));
    }
    Ok(())
}
fn lock_v6(document: &mut DocumentMut) -> Result<(), LauncherError> {
    if let Some(java) = document.get("java")
        && java.get("provider").and_then(|provider| provider.as_str()) != Some("mojang")
    {
        return Err(LauncherError::InvalidLock(
            "schema 6 supported only Mojang-managed Java; original file was not changed".into(),
        ));
    }
    Ok(())
}
fn config_v2(document: &mut DocumentMut) -> Result<(), LauncherError> {
    if document.contains_key("java") {
        return Err(LauncherError::InvalidConfig(
            "schema 2 did not support a java table; original file was not changed".into(),
        ));
    }
    document["java"]["selection"] = value("auto");
    Ok(())
}

fn upgrade(document: &mut DocumentMut, kind: DocumentKind) -> Result<(), LauncherError> {
    let (baseline, current) = kind.schemas();
    let mut schema = document
        .get("schema")
        .and_then(|v| v.as_integer())
        .ok_or_else(|| {
            LauncherError::Transaction("stored document has no integer schema".into())
        })?;
    if schema < baseline || schema > current {
        return Err(LauncherError::Transaction(format!(
            "unsupported schema {schema}; migration starts at the v0.7.0 schema {baseline}"
        )));
    }
    while schema < current {
        let step = kind
            .steps()
            .iter()
            .find(|step| step.from == schema)
            .ok_or_else(|| {
                LauncherError::Transaction(format!("no released migration from schema {schema}"))
            })?;
        if step.to <= schema {
            return Err(LauncherError::Transaction(
                "invalid storage migration sequence".into(),
            ));
        }
        (step.apply)(document)?;
        document["schema"] = value(step.to);
        schema = step.to;
    }
    Ok(())
}

/// Read-only current-model projection for destructive-operation previews.
pub(crate) fn inspect_current(path: &Path, kind: DocumentKind) -> Result<String, LauncherError> {
    let original = std::fs::read_to_string(path)?;
    let mut document = original.parse::<DocumentMut>().map_err(|error| {
        LauncherError::Transaction(format!("cannot inspect stored schema: {error}"))
    })?;
    upgrade(&mut document, kind)?;
    let projected = document.to_string();
    kind.validate(&projected)?;
    Ok(projected)
}

pub(crate) fn read_current(path: &Path, kind: DocumentKind) -> Result<String, LauncherError> {
    let original = std::fs::read_to_string(path)?;
    let (previous, current) = kind.schemas();
    let parse = |text: &str| {
        text.parse::<DocumentMut>().map_err(|e| {
            LauncherError::Transaction(format!(
                "cannot inspect schema in '{}': {e}",
                path.display()
            ))
        })
    };
    let mut document = parse(&original)?;
    if document.get("schema").and_then(|v| v.as_integer()) == Some(current) {
        return Ok(original);
    }
    if !document
        .get("schema")
        .and_then(|v| v.as_integer())
        .is_some_and(|schema| (previous..current).contains(&schema))
    {
        return Err(LauncherError::Transaction(format!(
            "unsupported schema {:?} in '{}'; expected {previous} or {current}; original file was not changed",
            document.get("schema").and_then(|v| v.as_integer()),
            path.display()
        )));
    }
    let lock_path = path.with_extension(format!(
        "{}.migration-lock",
        path.extension().and_then(|s| s.to_str()).unwrap_or("file")
    ));
    let guard = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    guard.lock_exclusive()?;
    // A second process may have completed the same migration while we waited.
    let original = std::fs::read_to_string(path)?;
    document = parse(&original)?;
    if document.get("schema").and_then(|v| v.as_integer()) == Some(current) {
        return Ok(original);
    }
    let source_schema = document.get("schema").and_then(|v| v.as_integer());
    if !source_schema.is_some_and(|schema| (previous..current).contains(&schema)) {
        return Err(LauncherError::Transaction(format!(
            "schema changed while migrating '{}'",
            path.display()
        )));
    }
    upgrade(&mut document, kind)?;
    let upgraded = document.to_string();
    kind.validate(&upgraded)?;
    let digest = hex::encode(Sha256::digest(original.as_bytes()));
    let name = path.file_name().and_then(|n| n.to_str()).ok_or_else(|| {
        LauncherError::Transaction("migration path has no UTF-8 file name".into())
    })?;
    let backup = path.with_file_name(format!(
        "{name}.schema-{}.{}.bak",
        source_schema.unwrap_or(previous),
        &digest[..16]
    ));
    let mut saved = tempfile::NamedTempFile::new_in(
        path.parent()
            .ok_or_else(|| LauncherError::Transaction("migration path has no parent".into()))?,
    )?;
    saved.write_all(original.as_bytes())?;
    saved.flush()?;
    saved.as_file().sync_all()?;
    match saved.persist_noclobber(&backup) {
        Ok(_) => {}
        Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => {
            if std::fs::read(&backup)? != original.as_bytes() {
                return Err(LauncherError::Transaction(format!(
                    "migration backup '{}' conflicts with original data",
                    backup.display()
                )));
            }
        }
        Err(e) => return Err(e.error.into()),
    }
    write_atomic(path, upgraded.as_bytes())?;
    Ok(upgraded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::GlobalConfig;
    #[test]
    fn released_instance_memory_is_migrated_once_with_exact_backup() {
        for schema in [1, 2] {
            for launch in [
                "[launch]\n# Memory comment\nmin_memory_mib = 768\nmax_memory_mib = 6144\njvm_args = ['-Dkeep=true']\n",
                "launch = { min_memory_mib = 768, max_memory_mib = 6144, jvm_args = ['-Dkeep=true'] }\n",
            ] {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join(crate::instance::INSTANCE_MANIFEST_FILE);
                let prefix = format!(
                    "# Original\nschema = {schema}\nid = 'b895cb9a-d43e-4a76-9fc4-a5b8d05c4385'\nname = 'client'\nkind = 'client'\n"
                );
                let suffix = "[minecraft]\nrequirement = '26.2'\n[loader]\nkind = 'vanilla'\n";
                let old = format!("{prefix}{launch}{suffix}");
                std::fs::write(&path, &old).unwrap();
                let file = crate::instance::ManifestFile::open(dir.path()).unwrap();
                assert_eq!(file.inner.schema, 3);
                assert_eq!(
                    file.inner.launch.memory,
                    crate::instance::MemoryConfig {
                        min_mib: 768,
                        max_mib: 6144
                    }
                );
                assert_eq!(file.inner.launch.jvm_args, ["-Dkeep=true"]);
                let migrated = std::fs::read_to_string(&path).unwrap();
                if launch.contains("# Memory comment") {
                    assert!(migrated.contains("# Memory comment"));
                }
                assert!(!migrated.contains("max_memory_mib"));
                assert!(migrated.contains("[launch.memory]"));
                crate::instance::ManifestFile::open(dir.path()).unwrap();
                assert_eq!(std::fs::read_to_string(&path).unwrap(), migrated);
                let backups: Vec<_> = std::fs::read_dir(dir.path())
                    .unwrap()
                    .flatten()
                    .filter(|entry| {
                        entry
                            .path()
                            .extension()
                            .is_some_and(|extension| extension == "bak")
                    })
                    .collect();
                assert_eq!(backups.len(), 1);
                assert_eq!(std::fs::read_to_string(backups[0].path()).unwrap(), old);
            }
        }
    }
    #[test]
    fn legacy_heap_arguments_are_consolidated_without_changing_effective_values() {
        let source = "schema = 2\n[launch]\nmin_memory_mib = 512\nmax_memory_mib = 4096\njvm_args = ['-Xmx6G', '-Dkeep=true', '-Xms768m', '-XX:MaxHeapSize=8589934592']\n";
        let mut document = source.parse::<DocumentMut>().unwrap();
        upgrade(&mut document, DocumentKind::Instance).unwrap();
        assert_eq!(
            document["launch"]["memory"]["min_mib"].as_integer(),
            Some(768)
        );
        assert_eq!(
            document["launch"]["memory"]["max_mib"].as_integer(),
            Some(8192)
        );
        assert_eq!(document["launch"]["jvm_args"].as_array().unwrap().len(), 1);
        assert_eq!(
            document["launch"]["jvm_args"][0].as_str(),
            Some("-Dkeep=true")
        );
    }
    #[test]
    fn upgrade_keeps_exact_backup_and_comments_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let old =
            "# User configuration\nschema = 2\n[network]\n# Keep this value\nconcurrency = 12\n";
        std::fs::write(&path, old).unwrap();
        let config = GlobalConfig::load(&path).unwrap();
        assert_eq!(config.schema, 3);
        assert_eq!(config.network.concurrency, 12);
        let new = std::fs::read_to_string(&path).unwrap();
        assert!(new.contains("# Keep this value"));
        assert!(new.contains("selection = \"auto\""));
        GlobalConfig::load(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), new);
        let backups: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|s| s == "bak"))
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(std::fs::read_to_string(backups[0].path()).unwrap(), old);
    }
    #[test]
    fn failed_validation_and_unknown_schemas_leave_original_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        for text in [
            "schema = 1\n",
            "schema = 2\nunknown = true\n",
            "schema = 999\n",
            "# no schema\n",
        ] {
            std::fs::write(&path, text).unwrap();
            assert!(GlobalConfig::load(&path).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        }
    }

    #[test]
    fn release_sequences_start_at_0_7_and_have_no_gaps() {
        for kind in [
            DocumentKind::Config,
            DocumentKind::Instance,
            DocumentKind::Lock,
        ] {
            let (mut source, target) = kind.schemas();
            for step in kind.steps() {
                assert_eq!(step.from, source);
                assert!(step.to > source);
                source = step.to;
            }
            assert_eq!(source, target);
        }
    }
    #[test]
    fn concurrent_readers_perform_one_upgrade() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "schema = 2\n").unwrap();
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || GlobalConfig::load(&path).unwrap().schema)
            })
            .collect();
        for thread in threads {
            assert_eq!(thread.join().unwrap(), 3);
        }
        assert_eq!(
            std::fs::read_dir(dir.path())
                .unwrap()
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|s| s == "bak"))
                .count(),
            1
        );
    }
    #[test]
    fn backup_failure_prevents_replacing_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let old = "schema = 2\n";
        std::fs::write(&path, old).unwrap();
        let digest = hex::encode(Sha256::digest(old.as_bytes()));
        std::fs::create_dir(
            path.with_file_name(format!("config.toml.schema-2.{}.bak", &digest[..16])),
        )
        .unwrap();
        assert!(GlobalConfig::load(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), old);
    }
}
