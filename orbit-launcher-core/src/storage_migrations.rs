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
            Self::Instance => (1, crate::instance::INSTANCE_SCHEMA.into()),
            Self::Lock => (6, crate::lockfile::LOCK_SCHEMA.into()),
        }
    }
    fn steps(self) -> &'static [MigrationStep] {
        match self {
            Self::Config => &[MigrationStep {
                from: 2,
                to: 3,
                apply: config_v2,
            }],
            Self::Instance => &[MigrationStep {
                from: 1,
                to: 2,
                apply: instance_v1,
            }],
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
