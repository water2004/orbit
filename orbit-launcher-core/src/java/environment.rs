//! Platform locations only. Selection never reads environment variables itself.
use std::collections::BTreeSet;
use std::path::PathBuf;

pub(super) trait JavaEnvironment {
    fn candidates(&self) -> Vec<PathBuf>;
}
pub(super) struct NativeJavaEnvironment;
impl JavaEnvironment for NativeJavaEnvironment {
    fn candidates(&self) -> Vec<PathBuf> {
        let executable = if cfg!(windows) { "java.exe" } else { "java" };
        let mut candidates = BTreeSet::new();
        for key in ["JAVA_HOME", "JDK_HOME"] {
            if let Some(home) = std::env::var_os(key) {
                candidates.insert(PathBuf::from(home).join("bin").join(executable));
            }
        }
        if let Some(path) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path).filter(|p| p.is_absolute()) {
                candidates.insert(dir.join(executable));
            }
        }
        for root in installation_roots() {
            if let Ok(entries) = std::fs::read_dir(root) {
                for entry in entries.flatten() {
                    candidates.insert(entry.path().join("bin").join(executable));
                    #[cfg(target_os = "macos")]
                    candidates.insert(entry.path().join("Contents/Home/bin/java"));
                }
            }
        }
        candidates.into_iter().collect()
    }
}

#[cfg(windows)]
fn installation_roots() -> Vec<PathBuf> {
    ["ProgramFiles", "ProgramFiles(x86)"]
        .into_iter()
        .filter_map(std::env::var_os)
        .flat_map(|root| {
            [
                "Java",
                "Eclipse Adoptium",
                "Microsoft",
                "Amazon Corretto",
                "Zulu",
                "BellSoft",
            ]
            .map(|vendor| PathBuf::from(&root).join(vendor))
        })
        .collect()
}
#[cfg(target_os = "linux")]
fn installation_roots() -> Vec<PathBuf> {
    ["/usr/lib/jvm", "/usr/java", "/opt/java"]
        .into_iter()
        .map(PathBuf::from)
        .collect()
}
#[cfg(target_os = "macos")]
fn installation_roots() -> Vec<PathBuf> {
    vec![PathBuf::from("/Library/Java/JavaVirtualMachines")]
}
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn installation_roots() -> Vec<PathBuf> {
    Vec::new()
}
