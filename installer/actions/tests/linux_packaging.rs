#![cfg(target_os = "linux")]
use std::{os::unix::fs::PermissionsExt, path::Path, process::Command};

fn executable(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}
#[test]
fn deb_hooks_register_one_interface_and_never_clean_user_files_or_flip_on_upgrade() {
    let temp = tempfile::tempdir().unwrap();
    let tools = temp.path().join("tools");
    std::fs::create_dir(&tools).unwrap();
    executable(&tools.join("dpkg-query"), "#!/bin/sh\nprintf '0.8.1-1'\n");
    executable(
        &tools.join("update-alternatives"),
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >>\"$TEST_LOG\"\n",
    );
    let log = temp.path().join("calls");
    let home = temp.path().join("home");
    std::fs::create_dir(&home).unwrap();
    std::fs::write(home.join("world"), "never inspect or delete").unwrap();
    let scripts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../linux/maintainer-scripts");
    for package in ["orbit", "orbit-launcher"] {
        for (hook, action) in [
            ("postinst", "configure"),
            ("prerm", "upgrade"),
            ("postinst", "abort-upgrade"),
            ("prerm", "remove"),
            ("postinst", "abort-remove"),
        ] {
            let output = Command::new("/bin/sh")
                .arg(scripts.join(hook))
                .arg(action)
                .env("PATH", &tools)
                .env("TEST_LOG", &log)
                .env("HOME", &home)
                .env("DPKG_MAINTSCRIPT_PACKAGE", package)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    assert_eq!(
        std::fs::read_to_string(log).unwrap(),
        concat!(
            "--install /usr/bin/orbit-maintenance orbit-maintenance /usr/lib/orbit/orbit-maintenance 8002\n",
            "--remove orbit-maintenance /usr/lib/orbit/orbit-maintenance\n",
            "--install /usr/bin/orbit-maintenance orbit-maintenance /usr/lib/orbit/orbit-maintenance 8002\n",
            "--install /usr/bin/orbit-maintenance orbit-maintenance /usr/lib/orbit-launcher/orbit-maintenance 8002\n",
            "--remove orbit-maintenance /usr/lib/orbit-launcher/orbit-maintenance\n",
            "--install /usr/bin/orbit-maintenance orbit-maintenance /usr/lib/orbit-launcher/orbit-maintenance 8002\n"
        )
    );
    assert_eq!(
        std::fs::read_to_string(home.join("world")).unwrap(),
        "never inspect or delete"
    );
}
#[test]
fn native_cleanup_cli_resolves_xdg_without_writing_on_preview_or_cancel() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    for part in ["config", "data", "cache"] {
        std::fs::create_dir_all(home.join(part)).unwrap();
    }
    for level in ["cache", "settings", "all"] {
        let output = Command::new(env!("CARGO_BIN_EXE_orbit-maintenance"))
            .args(["--level", level, "--dry-run", "--output-format", "json"])
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("XDG_DATA_HOME", home.join("data"))
            .env("XDG_CACHE_HOME", home.join("cache"))
            .env_remove("SUDO_USER")
            .env_remove("PKEXEC_UID")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        for part in ["config", "data", "cache"] {
            assert_eq!(
                Path::new(plan["roots"][part].as_str().unwrap()),
                home.join(part)
            );
            assert_eq!(std::fs::read_dir(home.join(part)).unwrap().count(), 0);
        }
    }
}
