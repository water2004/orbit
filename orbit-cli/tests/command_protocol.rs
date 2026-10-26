//! Exercise the process boundary used by both shell users and GUI callers.
use std::process::{Command, Output};

fn run(root: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_orbit"))
        .current_dir(root)
        .args([
            "--language",
            "en",
            "--output-format",
            "json",
            "--progress-format",
            "none",
        ])
        .arg("--config")
        .arg(root.join("config.toml"))
        .arg("--cache-dir")
        .arg(root.join("cache"))
        .arg("--repository-dir")
        .arg(root.join("repository"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn nested_success_and_failure_use_the_parsed_command_identity() {
    let temp = tempfile::tempdir().unwrap();
    let success = run(temp.path(), &["config", "list"]);
    assert!(
        success.status.success(),
        "{}",
        String::from_utf8_lossy(&success.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&success.stdout).unwrap();
    assert_eq!(result["command"], "config.list");
    let failed = run(temp.path(), &["package", "sodium", "files"]);
    assert!(!failed.status.success());
    assert!(failed.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&failed.stderr).unwrap();
    assert_eq!(error["command"], "package.files");
    assert_eq!(error["code"], "manifest_not_found");
}

#[test]
fn unsupported_global_dry_run_cannot_change_the_registry() {
    let temp = tempfile::tempdir().unwrap();
    let result = run(
        temp.path(),
        &["instance", "unregister", "missing", "--dry-run"],
    );
    assert!(!result.status.success());
    let error: serde_json::Value = serde_json::from_slice(&result.stderr).unwrap();
    assert_eq!(error["command"], "instance.unregister");
    assert!(error["message"].as_str().unwrap().contains("--dry-run"));
    assert!(!temp.path().join("instances.toml").exists());
}
