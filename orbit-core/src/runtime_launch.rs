//! Joint Orbit + Orbit Launcher process orchestration.
//!
//! Orbit owns runtime observation. Orbit Launcher remains an independent
//! runtime launcher and receives the Java agent only through the child process
//! environment.

use std::path::{Path, PathBuf};
use std::process::Command;

use base64::Engine as _;

use crate::error::{OrbitError, RuntimeComponent, RuntimeDataError};
use crate::runtime_data::{merge_observation_sessions, observation_session_path};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeLaunchTarget {
    Client,
    Server,
}

#[derive(Debug, Clone)]
pub struct RuntimeLaunchRequest {
    pub instance_dir: PathBuf,
    pub launcher_program: PathBuf,
    pub runtime_agent: PathBuf,
    pub launcher_instance: Option<String>,
    pub target: RuntimeLaunchTarget,
    pub language: String,
    pub output_format: String,
    pub progress_format: String,
    pub non_interactive: bool,
    pub dry_run: bool,
}

pub fn launch_with_runtime_observation(request: &RuntimeLaunchRequest) -> Result<(), OrbitError> {
    let instance_dir = request.instance_dir.canonicalize()?;
    if !instance_dir.join("orbit.toml").is_file() {
        return Err(OrbitError::ManifestNotFound);
    }
    if !instance_dir.join("orbit.lock").is_file() {
        return Err(OrbitError::LockfileNotFound);
    }
    let launcher_program = absolute_file(&request.launcher_program, RuntimeComponent::Launcher)?;
    let runtime_agent = absolute_file(&request.runtime_agent, RuntimeComponent::Agent)?;
    if request.target == RuntimeLaunchTarget::Server && request.dry_run {
        return Err(OrbitError::RuntimeData(RuntimeDataError::ServerDryRun));
    }

    let java_tool_options = if request.dry_run {
        None
    } else {
        merge_observation_sessions(&instance_dir)?;
        let session = observation_session_path(&instance_dir)?;
        let agent_option = java_agent_option(&runtime_agent, &instance_dir, &session)?;
        Some(append_java_tool_option(
            std::env::var_os("JAVA_TOOL_OPTIONS").as_deref(),
            &agent_option,
        )?)
    };

    let mut command = Command::new(launcher_program);
    command
        .current_dir(&instance_dir)
        .arg("--language")
        .arg(&request.language)
        .arg("--output-format")
        .arg(&request.output_format)
        .arg("--progress-format")
        .arg(&request.progress_format);
    if let Some(java_tool_options) = java_tool_options {
        command.env("JAVA_TOOL_OPTIONS", java_tool_options);
    }
    if request.non_interactive {
        command.arg("--non-interactive");
    }
    if let Some(instance) = &request.launcher_instance {
        command.arg("--instance").arg(instance);
    }
    match request.target {
        RuntimeLaunchTarget::Client => {
            command.arg("launch");
            if request.dry_run {
                command.arg("--dry-run");
            }
        }
        RuntimeLaunchTarget::Server => {
            command.args(["server", "start"]);
        }
    }

    let status = command.status()?;
    // Client launch normally blocks until Java exits. Server start normally
    // detaches; its snapshot is merged by the next Orbit launch/purge command.
    if !request.dry_run {
        merge_observation_sessions(&instance_dir)?;
    }
    if status.success() {
        Ok(())
    } else {
        Err(OrbitError::ForwardedProcessExit(status.code().unwrap_or(1)))
    }
}

fn absolute_file(path: &Path, component: RuntimeComponent) -> Result<PathBuf, OrbitError> {
    if !path.is_absolute() {
        return Err(OrbitError::RuntimeData(
            RuntimeDataError::ComponentPathNotAbsolute {
                component,
                path: path.display().to_string(),
            },
        ));
    }
    if path.is_file() {
        return Ok(path.to_path_buf());
    }
    Err(OrbitError::RuntimeData(
        RuntimeDataError::ComponentNotFound {
            component,
            path: path.display().to_string(),
        },
    ))
}

fn java_agent_option(agent: &Path, instance: &Path, session: &Path) -> Result<String, OrbitError> {
    let agent = agent.to_string_lossy();
    if agent.contains('"') {
        return Err(OrbitError::RuntimeData(
            RuntimeDataError::AgentPathContainsQuote,
        ));
    }
    let encode = |path: &Path| {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(path.to_string_lossy().as_bytes())
    };
    Ok(format!(
        "-javaagent:\"{agent}\"=root={};session={}",
        encode(instance),
        encode(session)
    ))
}

fn append_java_tool_option(
    existing: Option<&std::ffi::OsStr>,
    agent_option: &str,
) -> Result<std::ffi::OsString, OrbitError> {
    let mut value = existing.map(std::ffi::OsString::from).unwrap_or_default();
    if value
        .to_string_lossy()
        .contains("dev.orbit.agent.OrbitRuntimeAgent")
        || value.to_string_lossy().contains("orbit-runtime-agent")
    {
        return Err(OrbitError::RuntimeData(
            RuntimeDataError::AgentAlreadyPresent,
        ));
    }
    if !value.is_empty() {
        value.push(" ");
    }
    value.push(agent_option);
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_existing_java_tool_options() {
        let combined = append_java_tool_option(
            Some(std::ffi::OsStr::new("-Xmx2G")),
            "-javaagent:agent.jar=example",
        )
        .unwrap();
        assert_eq!(combined, "-Xmx2G -javaagent:agent.jar=example");
    }

    #[test]
    fn quotes_agent_paths_and_encodes_runtime_paths() {
        let option = java_agent_option(
            Path::new("C:/Program Files/Orbit/orbit-runtime-agent.jar"),
            Path::new("C:/Games/Example"),
            Path::new("C:/Games/Example/.orbit/session.events"),
        )
        .unwrap();
        assert!(option.starts_with("-javaagent:\"C:/Program Files/Orbit/"));
        assert!(!option.contains("C:/Games/Example"));
    }

    #[test]
    fn rejects_relative_components_before_switching_to_the_instance_directory() {
        assert!(absolute_file(Path::new("component"), RuntimeComponent::Agent).is_err());
    }
}
