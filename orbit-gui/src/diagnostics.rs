use std::fmt::Write as _;

pub(crate) fn install_panic_reporter() {
    std::panic::set_hook(Box::new(|panic| {
        let mut report = String::new();
        let _ = writeln!(report, "Orbit GUI panicked: {panic}");
        let _ = writeln!(report, "{}", std::backtrace::Backtrace::force_capture());
        if let Some(path) = crash_report_path() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(path, report);
        }
    }));
}

fn crash_report_path() -> Option<std::path::PathBuf> {
    directories::ProjectDirs::from("dev", "Orbit", "Orbit GUI")
        .map(|dirs| dirs.data_local_dir().join("crash-report.txt"))
}
