use std::fmt::Write as _;
use std::io::Write as _;
use std::sync::Mutex;

pub(crate) fn install_panic_reporter() {
    let report_started = Mutex::new(false);
    std::panic::set_hook(Box::new(move |panic| {
        let mut report = String::new();
        let _ = writeln!(report, "Orbit GUI panicked: {panic}");
        let _ = writeln!(report, "{}", std::backtrace::Backtrace::force_capture());
        if let Some(path) = crash_report_path() {
            let Ok(mut started) = report_started.lock() else {
                return;
            };
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            // A panic crossing a native window callback triggers another panic
            // ("cannot unwind"). Preserve the original cause instead of
            // replacing it with that secondary failure.
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(!*started)
                .append(*started)
                .open(path)
            {
                *started = true;
                let _ = file.write_all(report.as_bytes());
            }
        }
    }));
}

fn crash_report_path() -> Option<std::path::PathBuf> {
    directories::ProjectDirs::from("dev", "Orbit", "Orbit GUI")
        .map(|dirs| dirs.data_local_dir().join("crash-report.txt"))
}
