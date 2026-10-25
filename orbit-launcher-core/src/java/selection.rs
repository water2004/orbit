//! One Java selection policy for installers, clients and dedicated servers.
use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::environment::{JavaEnvironment, NativeJavaEnvironment};
use super::{
    JavaProgressEvent, JavaTarget, MojangJavaPlan, PreparedJavaRuntime, install_mojang_java,
    parse_java_major, plan_mojang_java, property,
};
use crate::error::LauncherError;
use crate::lockfile::{JavaProvider, LockedJavaRuntime};
use crate::mojang::MojangJavaRequirement;
use crate::runtime::RuntimePaths;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum JavaSelection {
    #[default]
    Auto,
    Managed,
    Executable(PathBuf),
}
impl FromStr for JavaSelection {
    type Err = LauncherError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "auto" => Ok(Self::Auto),
            "managed" => Ok(Self::Managed),
            _ if Path::new(value).is_absolute() => Ok(Self::Executable(value.into())),
            _ => Err(LauncherError::InvalidConfig(
                "Java selection must be auto, managed, or an absolute Java executable path".into(),
            )),
        }
    }
}
impl TryFrom<String> for JavaSelection {
    type Error = LauncherError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}
impl From<JavaSelection> for String {
    fn from(value: JavaSelection) -> Self {
        match value {
            JavaSelection::Auto => "auto".into(),
            JavaSelection::Managed => "managed".into(),
            JavaSelection::Executable(path) => path.to_string_lossy().into_owned(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SystemJava {
    pub executable: PathBuf,
    pub version: String,
    pub major: u32,
    pub platform: String,
}
impl SystemJava {
    pub fn locked(&self) -> LockedJavaRuntime {
        LockedJavaRuntime {
            runtime_id: format!(
                "system-{}",
                &hex::encode(Sha256::digest(
                    self.executable.as_os_str().as_encoded_bytes()
                ))[..24]
            ),
            provider: JavaProvider::System,
            version: self.version.clone(),
            major: self.major,
            platform: self.platform.clone(),
            executable: self.executable.to_string_lossy().into_owned(),
        }
    }
}

/// Probe a real executable, not its name, registry version or JAVA_HOME label.
/// Bounded execution/output avoids hanging discovery on a broken PATH entry.
pub fn inspect_system_java(executable: &Path) -> Result<SystemJava, LauncherError> {
    let executable = dunce::canonicalize(executable)?;
    if !executable.is_file() {
        return Err(LauncherError::InvalidConfig(
            "Java executable is not a file".into(),
        ));
    }
    let mut command = Command::new(&executable);
    command
        .args(["-XshowSettings:properties", "-version"])
        .env_remove("JAVA_TOOL_OPTIONS")
        .env_remove("JDK_JAVA_OPTIONS")
        .env_remove("_JAVA_OPTIONS")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| LauncherError::Launch("Java probe has no stdout".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| LauncherError::Launch("Java probe has no stderr".into()))?;
    let read = |stream: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            stream
                .take(64 * 1024)
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        })
    };
    let out = read(Box::new(stdout));
    let err = read(Box::new(stderr));
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(LauncherError::Launch(format!(
                "Java probe timed out: '{}'",
                executable.display()
            )));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let join = |thread: std::thread::JoinHandle<std::io::Result<Vec<u8>>>| {
        thread
            .join()
            .map_err(|_| LauncherError::Launch("Java probe reader failed".into()))?
            .map_err(LauncherError::Io)
    };
    let mut output = join(out)?;
    output.extend(join(err)?);
    if !status.success() {
        return Err(LauncherError::Launch(format!(
            "Java probe '{}' failed with {status}",
            executable.display()
        )));
    }
    // Only ASCII property values are consumed; the VM may localize the rest.
    let output = String::from_utf8_lossy(&output);
    let version = property(&output, "java.version")
        .ok_or_else(|| LauncherError::Launch("Java did not report java.version".into()))?;
    let arch = property(&output, "os.arch")
        .ok_or_else(|| LauncherError::Launch("Java did not report os.arch".into()))?;
    let host = crate::platform::HostPlatform::native()?;
    let expected: &[&str] = match host.architecture {
        crate::platform::Architecture::X86 => &["x86", "i386"],
        crate::platform::Architecture::X86_64 => &["amd64", "x86_64"],
        crate::platform::Architecture::Arm64 => &["aarch64", "arm64"],
    };
    let platform = format!(
        "{}-{}",
        host.os.mojang_name(),
        host.architecture.rule_name()
    );
    if !expected.contains(&arch) {
        return Err(LauncherError::UnsupportedRequirement(format!(
            "Java '{}' architecture '{arch}' does not match {}",
            executable.display(),
            platform
        )));
    }
    Ok(SystemJava {
        executable,
        version: version.into(),
        major: parse_java_major(version)?,
        platform,
    })
}

/// Environment discovery is isolated from compatibility/selection. This searches
/// explicit environment locations and bounded conventional installation roots,
/// never the filesystem recursively and never through a shell.
pub fn discover_system_java() -> Vec<SystemJava> {
    discover_with(&NativeJavaEnvironment)
}
fn discover_with(environment: &dyn JavaEnvironment) -> Vec<SystemJava> {
    let mut seen = BTreeSet::new();
    environment
        .candidates()
        .into_iter()
        .filter_map(|p| dunce::canonicalize(p).ok())
        .filter(|p| seen.insert(p.clone()))
        .filter_map(|p| inspect_system_java(&p).ok())
        .collect()
}

fn compatible(java: SystemJava, major: u32) -> Result<SystemJava, LauncherError> {
    if java.major != major {
        return Err(LauncherError::UnsupportedRequirement(format!(
            "Java '{}' is version {}, but this runtime requires Java {major}",
            java.executable.display(),
            java.major
        )));
    }
    Ok(java)
}
pub fn select_system_java(
    selection: &JavaSelection,
    major: u32,
) -> Result<Option<SystemJava>, LauncherError> {
    match selection {
        JavaSelection::Managed => Ok(None),
        JavaSelection::Executable(path) => compatible(inspect_system_java(path)?, major).map(Some),
        JavaSelection::Auto => Ok(discover_system_java()
            .into_iter()
            .filter(|java| java.major == major)
            .max_by_key(|java| version_components(&java.version))),
    }
}
fn version_components(version: &str) -> Vec<u64> {
    version
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn choices_are_explicit_and_relative_executables_are_rejected() {
        assert_eq!(
            "auto".parse::<JavaSelection>().unwrap(),
            JavaSelection::Auto
        );
        assert_eq!(
            "managed".parse::<JavaSelection>().unwrap(),
            JavaSelection::Managed
        );
        assert!("java".parse::<JavaSelection>().is_err());
        let absolute = std::env::current_exe().unwrap();
        assert_eq!(
            absolute.to_string_lossy().parse::<JavaSelection>().unwrap(),
            JavaSelection::Executable(absolute)
        );
    }
    #[test]
    fn compatibility_uses_the_official_major_not_newer_is_always_better() {
        let java = SystemJava {
            executable: std::env::current_exe().unwrap(),
            version: "25.0.2".into(),
            major: 25,
            platform: "windows-x64".into(),
        };
        assert!(compatible(java.clone(), 25).is_ok());
        assert!(compatible(java, 21).is_err());
        assert!(version_components("25.0.10+1") > version_components("25.0.2+8"));
    }
    #[tokio::test]
    async fn missing_explicit_java_is_an_error_not_a_download_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let selection = JavaSelection::Executable(dir.path().join("missing-java"));
        let requirement = MojangJavaRequirement {
            component: "java-runtime-epsilon".into(),
            major: 25,
        };
        assert!(
            plan_java(&reqwest::Client::new(), &requirement, &selection, |_| {})
                .await
                .is_err()
        );
    }
    #[test]
    fn a_system_lock_records_a_real_absolute_executable() {
        let java = SystemJava {
            executable: std::env::current_exe().unwrap(),
            version: "25.0.2".into(),
            major: 25,
            platform: "linux".into(),
        };
        let locked = java.locked();
        assert_eq!(locked.provider, JavaProvider::System);
        assert!(Path::new(&locked.executable).is_absolute());
    }

    #[tokio::test]
    #[ignore = "requires an installed host Java runtime"]
    async fn host_java_planning_and_materialization_use_no_downloads() {
        let java = discover_system_java()
            .into_iter()
            .next()
            .expect("host has Java");
        let requirement = MojangJavaRequirement {
            component: "not-a-remote-runtime".into(),
            major: java.major,
        };
        let selection = JavaSelection::Executable(java.executable.clone());
        let client = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all("http://127.0.0.1:1").unwrap())
            .build()
            .unwrap();
        let plan = plan_java(&client, &requirement, &selection, |_| {})
            .await
            .unwrap();
        assert!(matches!(plan, JavaInstallPlan::System(_)));
        let dir = tempfile::tempdir().unwrap();
        let paths = RuntimePaths::resolve(&crate::runtime::RuntimePathOptions {
            config_dir: Some(dir.path().join("config")),
            data_dir: Some(dir.path().join("data")),
            cache_dir: Some(dir.path().join("cache")),
        })
        .unwrap();
        let prepared = materialize_java(&paths, &client, plan, 1, |_| {})
            .await
            .unwrap();
        assert_eq!(prepared.executable, java.executable);
        assert_eq!(prepared.downloaded_artifacts, 0);
        assert!(!paths.data_dir().join("runtimes").exists());
        assert_eq!(
            resolve_launch_java(&paths, &prepared.locked, &selection)
                .unwrap()
                .runtime_id,
            prepared.locked.runtime_id
        );
    }
}

#[derive(Debug, Clone)]
pub enum JavaInstallPlan {
    Managed(MojangJavaPlan),
    System(SystemJava),
}
pub async fn plan_java<F: FnMut(JavaProgressEvent)>(
    client: &reqwest::Client,
    requirement: &MojangJavaRequirement,
    selection: &JavaSelection,
    progress: F,
) -> Result<JavaInstallPlan, LauncherError> {
    if let Some(java) = select_system_java(selection, requirement.major)? {
        return Ok(JavaInstallPlan::System(java));
    }
    plan_mojang_java(client, requirement, JavaTarget::native()?, progress)
        .await
        .map(JavaInstallPlan::Managed)
}
pub async fn materialize_java<F: FnMut(JavaProgressEvent) + Send>(
    paths: &RuntimePaths,
    client: &reqwest::Client,
    plan: JavaInstallPlan,
    concurrency: usize,
    mut progress: F,
) -> Result<PreparedJavaRuntime, LauncherError> {
    match plan {
        JavaInstallPlan::Managed(plan) => {
            install_mojang_java(paths, client, plan, concurrency, progress).await
        }
        JavaInstallPlan::System(java) => {
            let java = compatible(inspect_system_java(&java.executable)?, java.major)?;
            let locked = java.locked();
            progress(JavaProgressEvent::RuntimeVerified {
                runtime_id: locked.runtime_id.clone(),
            });
            Ok(PreparedJavaRuntime {
                locked,
                executable: java.executable,
                downloaded_artifacts: 0,
                cached_artifacts: 0,
            })
        }
    }
}

pub struct SelectedJava {
    pub executable: PathBuf,
    pub runtime_id: String,
}
pub fn resolve_launch_java(
    paths: &RuntimePaths,
    locked: &LockedJavaRuntime,
    selection: &JavaSelection,
) -> Result<SelectedJava, LauncherError> {
    if let Some(java) = select_system_java(selection, locked.major)? {
        return Ok(SelectedJava {
            runtime_id: java.locked().runtime_id,
            executable: java.executable,
        });
    }
    if locked.provider != JavaProvider::Mojang {
        return Err(LauncherError::Launch("No compatible system Java is available; run 'orbit-launcher install' to provision the selected Java runtime".into()));
    }
    Ok(SelectedJava {
        executable: super::verify_locked_java_runtime(paths, locked)?,
        runtime_id: locked.runtime_id.clone(),
    })
}
