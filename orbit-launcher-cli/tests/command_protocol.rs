use std::process::{Command, Output};

fn run(root: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_orbit-launcher"))
        .current_dir(root)
        .args([
            "--language",
            "en",
            "--output-format",
            "json",
            "--progress-format",
            "none",
        ])
        .arg("--config-dir")
        .arg(root.join("config"))
        .arg("--data-dir")
        .arg(root.join("data"))
        .arg("--cache-dir")
        .arg(root.join("cache"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn clear_is_not_mislabeled_as_select_and_failure_keeps_the_same_identity() {
    let temp = tempfile::tempdir().unwrap();
    let cleared = run(temp.path(), &["account", "default", "clear"]);
    assert!(
        cleared.status.success(),
        "{}",
        String::from_utf8_lossy(&cleared.stderr)
    );
    let output: serde_json::Value = serde_json::from_slice(&cleared.stdout).unwrap();
    assert_eq!(output["command"], "account.default.clear");
    assert_eq!(output["result"]["selection"]["state"], "not-configured");
    let failed = run(
        temp.path(),
        &["--instance", "missing", "instance", "account", "show"],
    );
    assert!(!failed.status.success());
    assert!(failed.stdout.is_empty());
    let error: serde_json::Value = serde_json::from_slice(&failed.stderr).unwrap();
    assert_eq!(error["command"], "instance.account.show");
}
