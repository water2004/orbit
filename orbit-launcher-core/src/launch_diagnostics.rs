//! Bounded, launch-scoped failure evidence. Old crash reports are never presented
//! as the cause of a new launch; report files themselves are not modified.
use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

const REPORT_BYTES: u64 = 128 * 1024;
const LOG_BYTES: u64 = 32 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticDocument {
    pub path: PathBuf,
    pub text: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchDiagnostics {
    pub reports: Vec<DiagnosticDocument>,
    pub log_tail: Option<DiagnosticDocument>,
    pub warnings: Vec<String>,
}

#[derive(PartialEq, Eq)]
struct Stamp {
    bytes: u64,
    modified: Option<SystemTime>,
}
pub(crate) struct DiagnosticSnapshot {
    directory: PathBuf,
    reports: BTreeMap<PathBuf, Stamp>,
    warnings: Vec<String>,
    log_stamp: Option<Stamp>,
    native: BTreeMap<PathBuf, Stamp>,
}
fn files(
    directory: &Path,
    extension: &str,
    prefix: &str,
    warnings: &mut Vec<String>,
) -> BTreeMap<PathBuf, Stamp> {
    let mut result = BTreeMap::new();
    if let Ok(metadata) = std::fs::symlink_metadata(directory)
        && crate::instance_lifecycle::is_redirected(&metadata)
    {
        warnings.push(format!(
            "refusing redirected report directory '{}'",
            directory.display()
        ));
        return result;
    }
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return result,
        Err(error) => {
            warnings.push(format!("cannot inspect '{}': {error}", directory.display()));
            return result;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                warnings.push(error.to_string());
                continue;
            }
        };
        let path = entry.path();
        if path.extension().is_none_or(|value| value != extension)
            || !entry.file_name().to_string_lossy().starts_with(prefix)
        {
            continue;
        }
        match std::fs::symlink_metadata(&path) {
            Ok(metadata)
                if metadata.is_file() && !crate::instance_lifecycle::is_redirected(&metadata) =>
            {
                result.insert(
                    path,
                    Stamp {
                        bytes: metadata.len(),
                        modified: metadata.modified().ok(),
                    },
                );
            }
            Ok(_) => {}
            Err(error) => warnings.push(format!("cannot inspect '{}': {error}", path.display())),
        }
    }
    result
}
impl DiagnosticSnapshot {
    pub(crate) fn capture(instance: &Path) -> Self {
        let directory = instance.join("crash-reports");
        let mut warnings = Vec::new();
        let reports = files(&directory, "txt", "", &mut warnings);
        let native = files(instance, "log", "hs_err_pid", &mut warnings);
        let log_stamp = stamp(&instance.join("logs/latest.log"), &mut warnings);
        Self {
            directory,
            reports,
            warnings,
            log_stamp,
            native,
        }
    }
    pub(crate) fn failure(
        self,
        instance: &Path,
        pid: u32,
        secrets: &[String],
    ) -> LaunchDiagnostics {
        let mut result = LaunchDiagnostics {
            warnings: self.warnings,
            ..Default::default()
        };
        let mut changed: Vec<_> = files(&self.directory, "txt", "", &mut result.warnings)
            .into_iter()
            .filter(|(path, stamp)| self.reports.get(path) != Some(stamp))
            .collect();
        changed.sort_by_key(|(_, stamp)| std::cmp::Reverse(stamp.modified));
        for (path, _) in changed.into_iter().take(2) {
            collect_document(&path, REPORT_BYTES, false, secrets, &mut result, false);
        }
        let native = instance.join(format!("hs_err_pid{pid}.log"));
        if stamp(&native, &mut result.warnings).as_ref() != self.native.get(&native) {
            collect_document(&native, REPORT_BYTES, false, secrets, &mut result, false);
        }
        let log_path = instance.join("logs/latest.log");
        let current_stamp = stamp(&log_path, &mut result.warnings);
        if current_stamp != self.log_stamp {
            collect_document(&log_path, LOG_BYTES, true, secrets, &mut result, true);
        }
        result
    }
}

impl LaunchDiagnostics {
    pub(crate) fn fallback_log(&mut self, path: &Path, secrets: &[String]) {
        if self.log_tail.is_none()
            && stamp(path, &mut self.warnings).is_some_and(|stamp| stamp.bytes > 0)
        {
            collect_document(path, LOG_BYTES, true, secrets, self, true);
        }
    }
}
fn stamp(path: &Path, warnings: &mut Vec<String>) -> Option<Stamp> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => Some(Stamp {
            bytes: metadata.len(),
            modified: metadata.modified().ok(),
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            warnings.push(format!("cannot inspect '{}': {error}", path.display()));
            None
        }
    }
}
fn collect_document(
    path: &Path,
    limit: u64,
    tail: bool,
    secrets: &[String],
    result: &mut LaunchDiagnostics,
    log: bool,
) {
    match read_document(path, limit, tail, secrets) {
        Ok(Some(document)) if log => result.log_tail = Some(document),
        Ok(Some(document)) => result.reports.push(document),
        Ok(None) => {}
        Err(error) => result
            .warnings
            .push(format!("cannot read '{}': {error}", path.display())),
    }
}
fn read_document(
    path: &Path,
    limit: u64,
    tail: bool,
    secrets: &[String],
) -> std::io::Result<Option<DiagnosticDocument>> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("diagnostic path has no parent"))?;
    if !metadata.is_file()
        || crate::instance_lifecycle::is_redirected(&metadata)
        || crate::instance_lifecycle::is_redirected(&std::fs::symlink_metadata(parent)?)
    {
        return Err(std::io::Error::other(
            "diagnostic path is not a regular file",
        ));
    }
    let mut file = std::fs::File::open(path)?;
    if tail && metadata.len() > limit {
        file.seek(SeekFrom::End(-(limit as i64)))?;
    }
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes)?;
    // Game files are not the UTF-8 machine protocol. Invalid file bytes are
    // displayed with an explicit notice, while the original remains available.
    let invalid_encoding = std::str::from_utf8(&bytes).is_err();
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    for secret in secrets.iter().filter(|secret| !secret.is_empty()) {
        text = text.replace(secret, "<redacted>");
    }
    if invalid_encoding {
        text.insert_str(0, "[Some non-UTF-8 bytes could not be displayed; open the original file for exact bytes.]\n");
    }
    let truncated = metadata.len() > limit || text.len() as u64 > limit;
    if text.len() as u64 > limit {
        let mut end = limit as usize;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    Ok(Some(DiagnosticDocument {
        path: path.into(),
        text,
        truncated,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_current_reports_are_collected_and_tokens_are_redacted() {
        let temp = tempfile::tempdir().unwrap();
        let reports = temp.path().join("crash-reports");
        std::fs::create_dir(&reports).unwrap();
        std::fs::write(reports.join("old.txt"), "unrelated failure").unwrap();
        let before = DiagnosticSnapshot::capture(temp.path());
        std::fs::write(reports.join("new.txt"), "crashed with secret-token").unwrap();
        let result = before.failure(temp.path(), 42, &["secret-token".into()]);
        assert_eq!(result.reports.len(), 1);
        assert_eq!(result.reports[0].text, "crashed with <redacted>");
        assert_eq!(
            std::fs::read_to_string(reports.join("new.txt")).unwrap(),
            "crashed with secret-token"
        );
    }
    #[test]
    fn native_reports_and_log_tails_are_bounded() {
        let temp = tempfile::tempdir().unwrap();
        let before = DiagnosticSnapshot::capture(temp.path());
        std::fs::write(
            temp.path().join("hs_err_pid42.log"),
            vec![b'a'; REPORT_BYTES as usize + 5],
        )
        .unwrap();
        std::fs::create_dir(temp.path().join("logs")).unwrap();
        std::fs::write(
            temp.path().join("logs/latest.log"),
            format!("{}END", "x".repeat(LOG_BYTES as usize)),
        )
        .unwrap();
        let result = before.failure(temp.path(), 42, &[]);
        assert!(result.reports[0].truncated);
        assert!(result.log_tail.as_ref().unwrap().text.ends_with("END"));
        assert!(result.log_tail.unwrap().truncated);
    }
    #[test]
    fn old_native_report_and_unchanged_game_log_are_not_reused() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("logs")).unwrap();
        std::fs::write(temp.path().join("logs/latest.log"), "old log").unwrap();
        std::fs::write(temp.path().join("hs_err_pid42.log"), "old native report").unwrap();
        let result = DiagnosticSnapshot::capture(temp.path()).failure(temp.path(), 42, &[]);
        assert!(result.reports.is_empty());
        assert!(result.log_tail.is_none());
    }
}
