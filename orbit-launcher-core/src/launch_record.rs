//! Launcher-owned records and bounded, already-redacted process logs.
use crate::{
    LauncherError,
    launch::{LaunchOutputStream, LaunchResult},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

const MAX_STREAM_BYTES: usize = 2 * 1024 * 1024;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchRecord {
    pub schema: u32,
    pub session_id: Uuid,
    pub pid: Option<u32>,
    pub stdout_log: PathBuf,
    pub stderr_log: PathBuf,
    pub result: Option<LaunchResult>,
}
pub fn latest_launch(instance: &Path) -> Result<Option<LaunchRecord>, LauncherError> {
    let file = match File::open(instance.join(".orbit-launcher/latest-launch.json")) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(LauncherError::Launch(
            "launch record exceeds its size bound".into(),
        ));
    }
    let record: LaunchRecord = serde_json::from_slice(&bytes)
        .map_err(|error| LauncherError::Launch(format!("invalid launch record: {error}")))?;
    if record.schema != 1 {
        return Err(LauncherError::Launch(
            "unsupported launch record schema".into(),
        ));
    }
    Ok(Some(record))
}
pub(crate) struct Recording {
    instance: PathBuf,
    pub record: LaunchRecord,
    files: [(File, usize); 2],
    _lease: File,
}
impl Recording {
    pub(crate) fn new(instance: &Path) -> Result<Self, LauncherError> {
        let root = instance.join(".orbit-launcher");
        match std::fs::symlink_metadata(&root) {
            Ok(metadata)
                if !metadata.is_dir() || crate::instance_lifecycle::is_redirected(&metadata) =>
            {
                return Err(LauncherError::Launch(
                    "launcher state directory must not be redirected".into(),
                ));
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
        std::fs::create_dir_all(&root)?;
        prune_logs(&root)?;
        let session_id = Uuid::new_v4();
        let lease = File::create_new(root.join(format!("launch-{session_id}.lock")))?;
        fs2::FileExt::try_lock_exclusive(&lease)?;
        let stdout_log = root.join(format!("launch-{session_id}.stdout.log"));
        let stderr_log = root.join(format!("launch-{session_id}.stderr.log"));
        let files = [
            (File::create_new(&stdout_log)?, 0),
            (File::create_new(&stderr_log)?, 0),
        ];
        let record = LaunchRecord {
            schema: 1,
            session_id,
            pid: None,
            stdout_log,
            stderr_log,
            result: None,
        };
        let recording = Self {
            instance: instance.into(),
            record,
            files,
            _lease: lease,
        };
        recording.save()?;
        Ok(recording)
    }
    fn save(&self) -> Result<(), LauncherError> {
        crate::atomic_io::write_atomic(
            &self.instance.join(".orbit-launcher/latest-launch.json"),
            &serde_json::to_vec_pretty(&self.record)
                .map_err(|error| LauncherError::Launch(error.to_string()))?,
        )
    }
    pub(crate) fn started(&mut self, pid: u32) -> Result<(), LauncherError> {
        self.record.pid = Some(pid);
        self.save()
    }
    pub(crate) fn line(
        &mut self,
        stream: LaunchOutputStream,
        line: &str,
    ) -> Result<(), LauncherError> {
        let (file, written) = &mut self.files[usize::from(stream == LaunchOutputStream::Stderr)];
        if *written >= MAX_STREAM_BYTES {
            return Ok(());
        }
        let mut remaining = (MAX_STREAM_BYTES - *written - 1).min(line.len());
        while !line.is_char_boundary(remaining) {
            remaining -= 1;
        }
        file.write_all(&line.as_bytes()[..remaining])?;
        file.write_all(b"\n")?;
        *written += remaining + 1;
        Ok(())
    }
    pub(crate) fn finish(&mut self, result: LaunchResult) -> Result<(), LauncherError> {
        for (file, _) in &self.files {
            file.sync_data()?;
        }
        if latest_launch(&self.instance)?
            .is_some_and(|current| current.session_id == self.record.session_id)
        {
            self.record.result = Some(result);
            self.save()?;
        }
        Ok(())
    }
}

/// Only our UUID-named, unlocked logs are eligible. Active sessions retain an
/// OS file lock, including when the GUI is gone; no process-name/PID guessing.
fn prune_logs(root: &Path) -> Result<(), LauncherError> {
    let mut sessions = std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(id) = name.strip_prefix("launch-").and_then(|name| {
            name.strip_suffix(".stdout.log")
                .or_else(|| name.strip_suffix(".stderr.log"))
        }) else {
            continue;
        };
        if Uuid::parse_str(id).is_err() {
            continue;
        }
        let metadata = std::fs::symlink_metadata(entry.path())?;
        if metadata.is_file() && !crate::instance_lifecycle::is_redirected(&metadata) {
            let modified = metadata.modified()?;
            sessions
                .entry(id.to_owned())
                .and_modify(|value: &mut std::time::SystemTime| *value = (*value).max(modified))
                .or_insert(modified);
        }
    }
    let mut sessions: Vec<_> = sessions
        .into_iter()
        .map(|(id, modified)| (modified, id))
        .collect();
    sessions.sort_by(|left, right| right.cmp(left));
    for (_, id) in sessions.into_iter().skip(19) {
        let lease = root.join(format!("launch-{id}.lock"));
        if std::fs::symlink_metadata(&lease)
            .is_ok_and(|metadata| crate::instance_lifecycle::is_redirected(&metadata))
        {
            return Err(LauncherError::Launch(
                "launch record lease must not be redirected".into(),
            ));
        }
        let file = match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lease)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => {
                for suffix in ["stdout.log", "stderr.log"] {
                    match std::fs::remove_file(root.join(format!("launch-{id}.{suffix}"))) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error.into()),
                    }
                }
                drop(file);
                std::fs::remove_file(lease)?;
            }
            Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_logs_remain_readable_and_a_newer_session_cannot_be_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let mut first = Recording::new(directory.path()).unwrap();
        first.started(42).unwrap();
        first
            .line(LaunchOutputStream::Stderr, &"中".repeat(MAX_STREAM_BYTES))
            .unwrap();
        first
            .line(LaunchOutputStream::Stderr, "extra output")
            .unwrap();
        assert!(
            std::fs::metadata(&first.record.stderr_log).unwrap().len() <= MAX_STREAM_BYTES as u64
        );
        assert!(std::fs::read_to_string(&first.record.stderr_log).is_ok());
        let second = Recording::new(directory.path()).unwrap();
        first
            .finish(LaunchResult {
                instance_id: Uuid::new_v4(),
                kind: crate::instance::InstanceKind::Client,
                pid: 42,
                exit_code: Some(0),
                success: true,
                elapsed_milliseconds: 5,
                diagnostics: None,
            })
            .unwrap();
        assert_eq!(
            latest_launch(directory.path()).unwrap().unwrap().session_id,
            second.record.session_id
        );
    }

    #[test]
    fn retention_removes_only_owned_unlocked_logs() {
        let directory = tempfile::tempdir().unwrap();
        let active = Recording::new(directory.path()).unwrap();
        let root = directory.path().join(".orbit-launcher");
        for path in [&active.record.stdout_log, &active.record.stderr_log] {
            std::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(std::time::UNIX_EPOCH)
                .unwrap();
        }
        for _ in 0..45 {
            std::fs::write(
                root.join(format!("launch-{}.stdout.log", Uuid::new_v4())),
                b"old log",
            )
            .unwrap();
        }
        std::fs::write(root.join("user-notes.txt"), b"keep").unwrap();
        prune_logs(&root).unwrap();
        assert!(active.record.stdout_log.is_file());
        assert!(active.record.stderr_log.is_file());
        assert!(root.join("user-notes.txt").is_file());
        assert!(std::fs::read_dir(&root).unwrap().count() <= 45);
    }
}
