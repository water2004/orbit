use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

use serde::Serialize;
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

use base64::Engine;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use crate::account::AccountLaunchIdentity;
use crate::artifact::hash_file_sha256;
use crate::config::GlobalConfig;
use crate::error::LauncherError;
use crate::instance::{
    InstanceKind, ManifestFile, RestartPolicy, ServerAuthenticationProvider, ServerConfig,
};
use crate::java::verify_locked_java_runtime;
use crate::layout::InstanceLocation;
use crate::lockfile::{LauncherLock, LockFile, LockedEntrypoint};
use crate::natives::prepare_native_directory;
use crate::runtime::RuntimePaths;

const LAUNCHER_NAME: &str = "orbit-launcher";
const REDACTED: &str = "<redacted>";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchPreparationEvent {
    ArtifactVerified { completed: usize, total: usize },
    JavaVerified { runtime_id: String },
    NativesPrepared { files: usize },
    PlanReady,
}

/// A fully expanded launch command. It deliberately has no `Debug`, `Clone`,
/// or serialization implementation because client arguments contain a live
/// access token.
pub struct LaunchPlan {
    instance_id: Uuid,
    kind: InstanceKind,
    java_executable: PathBuf,
    working_directory: PathBuf,
    arguments: Vec<String>,
    redacted_arguments: Vec<String>,
    sensitive_values: Vec<String>,
}

impl LaunchPlan {
    pub const fn instance_id(&self) -> Uuid {
        self.instance_id
    }

    pub const fn kind(&self) -> InstanceKind {
        self.kind
    }

    pub fn executable(&self) -> &Path {
        &self.java_executable
    }

    pub fn working_directory(&self) -> &Path {
        &self.working_directory
    }

    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.java_executable);
        command
            .args(&self.arguments)
            .current_dir(&self.working_directory);
        command
    }

    pub fn summary(&self) -> LaunchPlanSummary {
        LaunchPlanSummary {
            instance_id: self.instance_id,
            kind: self.kind,
            executable: self.java_executable.clone(),
            working_directory: self.working_directory.clone(),
            arguments: self.redacted_arguments.clone(),
        }
    }
}

impl Drop for LaunchPlan {
    fn drop(&mut self) {
        for argument in &mut self.arguments {
            argument.zeroize();
        }
        self.sensitive_values.zeroize();
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LaunchPlanSummary {
    pub instance_id: Uuid,
    pub kind: InstanceKind,
    pub executable: PathBuf,
    pub working_directory: PathBuf,
    pub arguments: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchOutputStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchProcessEvent {
    Spawned {
        pid: u32,
    },
    Output {
        stream: LaunchOutputStream,
        line: String,
    },
    Exited {
        exit_code: Option<i32>,
        success: bool,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct LaunchResult {
    pub instance_id: Uuid,
    pub kind: InstanceKind,
    pub pid: u32,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub elapsed_milliseconds: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupervisorControl {
    Command(String),
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupervisorEvent {
    Spawned {
        pid: u32,
        generation: u32,
    },
    Output {
        stream: LaunchOutputStream,
        line: String,
    },
    CommandSent {
        command: String,
    },
    StopRequested,
    Exited {
        exit_code: Option<i32>,
        success: bool,
        expected: bool,
        uptime_milliseconds: u128,
    },
    Backoff {
        delay_seconds: u64,
        restart_attempt: u32,
    },
    Restarting {
        generation: u32,
    },
    RestartLimitReached {
        attempts: u32,
        window_seconds: u64,
    },
    Stopped,
}

#[derive(Debug, Clone, Serialize)]
pub struct SupervisorResult {
    pub instance_id: Uuid,
    pub generations: u32,
    pub restarts: u32,
    pub final_exit_code: Option<i32>,
    pub final_success: bool,
    pub stopped_by_request: bool,
    pub restart_limit_reached: bool,
}

pub fn prepare_launch<F>(
    location: &InstanceLocation,
    runtime_paths: &RuntimePaths,
    config: &GlobalConfig,
    identity: Option<AccountLaunchIdentity>,
    mut progress: F,
) -> Result<LaunchPlan, LauncherError>
where
    F: FnMut(LaunchPreparationEvent),
{
    location
        .validate()
        .map_err(LauncherError::InvalidRegistry)?;
    let instance_root = dunce::canonicalize(location.instance_directory())?;
    let artifact_root = dunce::canonicalize(location.artifact_directory())?;
    let manifest = ManifestFile::open(&instance_root)?.inner;
    let lock = LockFile::open(&instance_root)?.inner;
    validate_manifest_lock(&manifest, &lock)?;
    if manifest.kind != location.kind() {
        return Err(LauncherError::InstanceRegistryMismatch(
            "registered instance layout kind disagrees with orbit-launcher.toml".to_string(),
        ));
    }
    verify_instance_files(&artifact_root, &instance_root, &lock, &mut progress)?;

    let locked_java = lock.java.as_ref().ok_or_else(|| {
        LauncherError::InvalidLock("installed instance does not lock a Java runtime".to_string())
    })?;
    let java_executable = verify_locked_java_runtime(runtime_paths, locked_java)?;
    progress(LaunchPreparationEvent::JavaVerified {
        runtime_id: locked_java.runtime_id.clone(),
    });

    if manifest.kind == InstanceKind::Client {
        let files = prepare_native_directory(
            &artifact_root,
            &instance_root.join("natives"),
            &lock.artifacts,
        )?;
        progress(LaunchPreparationEvent::NativesPrepared { files });
    }

    let placeholders = build_placeholders(
        &artifact_root,
        &instance_root,
        &manifest,
        &lock,
        identity.as_ref(),
    )?;
    let authentication_arguments =
        authentication_arguments(&artifact_root, &manifest, &lock, config, identity.as_ref())?;
    let classpath_explicit = lock
        .arguments
        .jvm
        .iter()
        .any(|argument| argument.contains("${classpath}"));
    let arguments = assemble_arguments(
        &artifact_root,
        &manifest,
        &lock,
        classpath_explicit,
        &placeholders.actual,
        &authentication_arguments,
    )?;
    let redacted_arguments = assemble_arguments(
        &artifact_root,
        &manifest,
        &lock,
        classpath_explicit,
        &placeholders.redacted,
        &authentication_arguments,
    )?;
    let sensitive_values = identity
        .as_ref()
        .filter(|identity| identity.access_token.len() > 1)
        .map(|identity| vec![identity.access_token.clone()])
        .unwrap_or_default();

    progress(LaunchPreparationEvent::PlanReady);
    Ok(LaunchPlan {
        instance_id: manifest.id,
        kind: manifest.kind,
        java_executable,
        working_directory: instance_root,
        arguments,
        redacted_arguments,
        sensitive_values,
    })
}

pub async fn run_launch<F>(
    mut plan: LaunchPlan,
    mut event_handler: F,
) -> Result<LaunchResult, LauncherError>
where
    F: FnMut(LaunchProcessEvent),
{
    let instance_id = plan.instance_id;
    let kind = plan.kind;
    let started = Instant::now();
    let sensitive_values = Zeroizing::new(std::mem::take(&mut plan.sensitive_values));
    let mut command = tokio::process::Command::from(plan.command());
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| {
        LauncherError::Launch(format!(
            "failed to start Java executable '{}': {error}",
            plan.executable().display()
        ))
    })?;
    let pid = child.id().ok_or_else(|| {
        LauncherError::Launch("spawned Java process has no process ID".to_string())
    })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| LauncherError::Launch("failed to capture Java stdout".to_string()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| LauncherError::Launch("failed to capture Java stderr".to_string()))?;
    event_handler(LaunchProcessEvent::Spawned { pid });

    let (sender, mut receiver) = mpsc::unbounded_channel();
    let stdout_task = tokio::spawn(pump_output(
        stdout,
        LaunchOutputStream::Stdout,
        sender.clone(),
    ));
    let stderr_task = tokio::spawn(pump_output(
        stderr,
        LaunchOutputStream::Stderr,
        sender.clone(),
    ));
    drop(sender);
    let wait_task = tokio::spawn(async move { child.wait().await });

    let mut output_error = None;
    while let Some(event) = receiver.recv().await {
        match event {
            Ok(LaunchProcessEvent::Output { stream, mut line }) => {
                for value in sensitive_values.iter() {
                    if line.contains(value) {
                        line = line.replace(value, REDACTED);
                    }
                }
                event_handler(LaunchProcessEvent::Output { stream, line });
            }
            Ok(event) => event_handler(event),
            Err(error) if output_error.is_none() => output_error = Some(error),
            Err(_) => {}
        }
    }
    stdout_task
        .await
        .map_err(|error| LauncherError::Launch(format!("stdout reader task failed: {error}")))?;
    stderr_task
        .await
        .map_err(|error| LauncherError::Launch(format!("stderr reader task failed: {error}")))?;
    let status = wait_task
        .await
        .map_err(|error| LauncherError::Launch(format!("process wait task failed: {error}")))??;
    if let Some(error) = output_error {
        return Err(error);
    }
    let exit_code = status.code();
    let success = status.success();
    event_handler(LaunchProcessEvent::Exited { exit_code, success });
    Ok(LaunchResult {
        instance_id,
        kind,
        pid,
        exit_code,
        success,
        elapsed_milliseconds: started.elapsed().as_millis(),
    })
}

pub async fn supervise_server<F>(
    plan: LaunchPlan,
    config: &ServerConfig,
    controls: &mut mpsc::UnboundedReceiver<SupervisorControl>,
    mut event_handler: F,
) -> Result<SupervisorResult, LauncherError>
where
    F: FnMut(SupervisorEvent),
{
    if plan.kind != InstanceKind::Server {
        return Err(LauncherError::Launch(
            "server supervisor requires a server launch plan".to_string(),
        ));
    }
    validate_supervisor_config(config)?;
    let mut generation = 0_u32;
    let mut restarts = 0_u32;
    let mut failures = VecDeque::new();
    let mut restart_limit_reached = false;
    let mut final_exit_code;
    let mut final_success;
    let mut stopped_by_request = false;

    'supervisor: loop {
        generation = generation.checked_add(1).ok_or_else(|| {
            LauncherError::Launch("server generation counter overflowed".to_string())
        })?;
        if generation > 1 {
            event_handler(SupervisorEvent::Restarting { generation });
        }
        let mut running = spawn_supervised_child(&plan).await?;
        event_handler(SupervisorEvent::Spawned {
            pid: running.pid,
            generation,
        });
        let started = Instant::now();
        let mut expected = false;
        let mut output_open = true;
        let status = loop {
            tokio::select! {
                status = running.child.wait() => break status?,
                output = running.output.recv(), if output_open => {
                    if let Some(output) = output {
                        match output {
                            Ok(LaunchProcessEvent::Output { stream, line }) => {
                                event_handler(SupervisorEvent::Output { stream, line });
                            }
                            Ok(_) => unreachable!("output pumps only emit output events"),
                            Err(error) => return Err(error),
                        }
                    } else {
                        output_open = false;
                    }
                }
                control = controls.recv() => {
                    match control.unwrap_or(SupervisorControl::Stop) {
                        SupervisorControl::Command(command) => {
                            validate_server_command(&command)?;
                            let stdin = running.stdin.as_mut().ok_or_else(|| {
                                LauncherError::Launch("server stdin is unavailable".to_string())
                            })?;
                            stdin.write_all(command.as_bytes()).await?;
                            stdin.write_all(b"\n").await?;
                            stdin.flush().await?;
                            event_handler(SupervisorEvent::CommandSent { command });
                        }
                        SupervisorControl::Stop => {
                            expected = true;
                            event_handler(SupervisorEvent::StopRequested);
                            break stop_supervised_child(
                                &mut running,
                                Duration::from_secs(config.graceful_stop_timeout_seconds),
                                Duration::from_secs(config.kill_timeout_seconds),
                            ).await?;
                        }
                    }
                }
            }
        };
        for (stream, line) in running.finish_output().await? {
            event_handler(SupervisorEvent::Output { stream, line });
        }
        let uptime = started.elapsed();
        event_handler(SupervisorEvent::Exited {
            exit_code: status.code(),
            success: status.success(),
            expected,
            uptime_milliseconds: uptime.as_millis(),
        });
        final_exit_code = status.code();
        final_success = status.success();
        if expected || config.restart == RestartPolicy::Never {
            stopped_by_request = expected;
            break 'supervisor;
        }

        if uptime >= Duration::from_secs(config.restart_window_seconds) {
            failures.clear();
        }
        let now = Instant::now();
        let window = Duration::from_secs(config.restart_window_seconds);
        while failures
            .front()
            .is_some_and(|failure| now.duration_since(*failure) > window)
        {
            failures.pop_front();
        }
        if failures.len() >= config.restart_limit as usize {
            restart_limit_reached = true;
            event_handler(SupervisorEvent::RestartLimitReached {
                attempts: config.restart_limit,
                window_seconds: config.restart_window_seconds,
            });
            break 'supervisor;
        }
        failures.push_back(now);
        restarts = restarts.checked_add(1).ok_or_else(|| {
            LauncherError::Launch("server restart counter overflowed".to_string())
        })?;
        let exponent = failures.len().saturating_sub(1).min(62) as u32;
        let delay_seconds = 1_u64
            .checked_shl(exponent)
            .unwrap_or(u64::MAX)
            .min(config.restart_backoff_max_seconds);
        event_handler(SupervisorEvent::Backoff {
            delay_seconds,
            restart_attempt: restarts,
        });
        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(delay_seconds)) => {}
            control = controls.recv() => {
                match control.unwrap_or(SupervisorControl::Stop) {
                    SupervisorControl::Stop => {
                        stopped_by_request = true;
                        event_handler(SupervisorEvent::StopRequested);
                        break 'supervisor;
                    }
                    SupervisorControl::Command(_) => {
                        return Err(LauncherError::Launch(
                            "cannot send a server command while restart backoff is active".to_string(),
                        ));
                    }
                }
            }
        }
    }
    event_handler(SupervisorEvent::Stopped);
    Ok(SupervisorResult {
        instance_id: plan.instance_id,
        generations: generation,
        restarts,
        final_exit_code,
        final_success,
        stopped_by_request,
        restart_limit_reached,
    })
}

struct SupervisedChild {
    child: tokio::process::Child,
    stdin: Option<tokio::process::ChildStdin>,
    output: mpsc::UnboundedReceiver<Result<LaunchProcessEvent, LauncherError>>,
    stdout_task: tokio::task::JoinHandle<()>,
    stderr_task: tokio::task::JoinHandle<()>,
    pid: u32,
}

impl SupervisedChild {
    async fn finish_output(mut self) -> Result<Vec<(LaunchOutputStream, String)>, LauncherError> {
        self.stdout_task.await.map_err(|error| {
            LauncherError::Launch(format!("stdout reader task failed: {error}"))
        })?;
        self.stderr_task.await.map_err(|error| {
            LauncherError::Launch(format!("stderr reader task failed: {error}"))
        })?;
        let mut remaining = Vec::new();
        while let Some(output) = self.output.recv().await {
            match output? {
                LaunchProcessEvent::Output { stream, line } => remaining.push((stream, line)),
                _ => unreachable!("output pumps only emit output events"),
            }
        }
        Ok(remaining)
    }
}

async fn spawn_supervised_child(plan: &LaunchPlan) -> Result<SupervisedChild, LauncherError> {
    let mut command = tokio::process::Command::from(plan.command());
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|error| {
        LauncherError::Launch(format!(
            "failed to start Java executable '{}': {error}",
            plan.executable().display()
        ))
    })?;
    let pid = child.id().ok_or_else(|| {
        LauncherError::Launch("spawned Java process has no process ID".to_string())
    })?;
    let stdin = child.stdin.take();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| LauncherError::Launch("failed to capture Java stdout".to_string()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| LauncherError::Launch("failed to capture Java stderr".to_string()))?;
    let (sender, output) = mpsc::unbounded_channel();
    let stdout_task = tokio::spawn(pump_output(
        stdout,
        LaunchOutputStream::Stdout,
        sender.clone(),
    ));
    let stderr_task = tokio::spawn(pump_output(stderr, LaunchOutputStream::Stderr, sender));
    Ok(SupervisedChild {
        child,
        stdin,
        output,
        stdout_task,
        stderr_task,
        pid,
    })
}

async fn stop_supervised_child(
    running: &mut SupervisedChild,
    graceful_timeout: Duration,
    kill_timeout: Duration,
) -> Result<std::process::ExitStatus, LauncherError> {
    if let Some(stdin) = running.stdin.as_mut() {
        let _ = stdin.write_all(b"stop\n").await;
        let _ = stdin.flush().await;
    }
    match tokio::time::timeout(graceful_timeout, running.child.wait()).await {
        Ok(status) => status.map_err(LauncherError::from),
        Err(_) => {
            running.child.start_kill()?;
            tokio::time::timeout(kill_timeout, running.child.wait())
                .await
                .map_err(|_| {
                    LauncherError::Launch(
                        "server did not exit after graceful stop and forced termination"
                            .to_string(),
                    )
                })?
                .map_err(LauncherError::from)
        }
    }
}

fn validate_server_command(command: &str) -> Result<(), LauncherError> {
    if command.is_empty()
        || command.len() > 32 * 1024
        || command.trim() != command
        || command.chars().any(char::is_control)
    {
        return Err(LauncherError::Launch(
            "server command must be a non-empty single line of at most 32 KiB".to_string(),
        ));
    }
    Ok(())
}

fn validate_supervisor_config(config: &ServerConfig) -> Result<(), LauncherError> {
    if config.restart_limit == 0
        || config.restart_window_seconds == 0
        || config.restart_backoff_max_seconds == 0
        || config.graceful_stop_timeout_seconds == 0
        || config.kill_timeout_seconds == 0
    {
        return Err(LauncherError::InvalidManifest(
            "server supervisor limits and timeouts must be greater than zero".to_string(),
        ));
    }
    Ok(())
}

async fn pump_output<R>(
    reader: R,
    stream: LaunchOutputStream,
    sender: mpsc::UnboundedSender<Result<LaunchProcessEvent, LauncherError>>,
) where
    R: AsyncRead + Unpin,
{
    let mut lines = BufReader::new(reader).split(b'\n');
    loop {
        match lines.next_segment().await {
            Ok(Some(mut bytes)) => {
                if bytes.last() == Some(&b'\r') {
                    bytes.pop();
                }
                let line = String::from_utf8_lossy(&bytes).into_owned();
                if sender
                    .send(Ok(LaunchProcessEvent::Output { stream, line }))
                    .is_err()
                {
                    break;
                }
            }
            Ok(None) => break,
            Err(error) => {
                let _ = sender.send(Err(LauncherError::Launch(format!(
                    "failed to read Java {stream:?}: {error}"
                ))));
                break;
            }
        }
    }
}

fn validate_manifest_lock(
    manifest: &crate::instance::InstanceManifest,
    lock: &LauncherLock,
) -> Result<(), LauncherError> {
    if manifest.id != lock.instance_id || manifest.kind != lock.kind {
        return Err(LauncherError::InvalidLock(
            "orbit-launcher.toml and orbit-launcher.lock identify different instances".to_string(),
        ));
    }
    Ok(())
}

fn verify_instance_files<F>(
    artifact_root: &Path,
    instance_root: &Path,
    lock: &LauncherLock,
    progress: &mut F,
) -> Result<(), LauncherError>
where
    F: FnMut(LaunchPreparationEvent),
{
    let total = lock.artifacts.len();
    for (index, artifact) in lock.artifacts.iter().enumerate() {
        let path = artifact_root.join(path_from_portable(&artifact.path));
        let metadata = std::fs::metadata(&path).map_err(|error| {
            LauncherError::ArtifactIntegrity(format!(
                "installed artifact '{}' is unavailable: {error}",
                artifact.logical_name
            ))
        })?;
        if !metadata.is_file()
            || metadata.len() != artifact.size
            || hash_file_sha256(&path)? != artifact.sha256
        {
            return Err(LauncherError::ArtifactIntegrity(format!(
                "installed artifact '{}' failed launch-time verification",
                artifact.logical_name
            )));
        }
        progress(LaunchPreparationEvent::ArtifactVerified {
            completed: index + 1,
            total,
        });
    }
    for generated in &lock.generated_files {
        if !artifact_root.join(path_from_portable(generated)).is_file() {
            return Err(LauncherError::ArtifactIntegrity(format!(
                "generated runtime file '{generated}' is missing"
            )));
        }
    }
    if lock.kind == InstanceKind::Server
        && std::fs::read_to_string(instance_root.join("eula.txt"))?.trim() != "eula=true"
    {
        return Err(LauncherError::EulaRequired(
            "eula.txt no longer records acceptance; install again or accept the EULA".to_string(),
        ));
    }
    Ok(())
}

struct PlaceholderSets {
    actual: BTreeMap<&'static str, String>,
    redacted: BTreeMap<&'static str, String>,
}

fn build_placeholders(
    artifact_root: &Path,
    instance_root: &Path,
    manifest: &crate::instance::InstanceManifest,
    lock: &LauncherLock,
    identity: Option<&AccountLaunchIdentity>,
) -> Result<PlaceholderSets, LauncherError> {
    match (manifest.kind, identity) {
        (InstanceKind::Client, None) => {
            return Err(LauncherError::InteractionRequired(
                "a client account must be resolved before preparing launch".to_string(),
            ));
        }
        (InstanceKind::Server, Some(_)) => {
            return Err(LauncherError::Launch(
                "server launch cannot receive a client account".to_string(),
            ));
        }
        _ => {}
    }
    let classpath = match &lock.entrypoint {
        LockedEntrypoint::Classpath { classpath, .. } => classpath
            .iter()
            .map(|path| artifact_root.join(path_from_portable(path)))
            .collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    let classpath = std::env::join_paths(&classpath)
        .map_err(|error| LauncherError::Launch(format!("invalid classpath: {error}")))?
        .to_string_lossy()
        .into_owned();
    let asset_index = lock.minecraft.asset_index.clone().unwrap_or_default();
    let mut values = BTreeMap::from([
        (
            "${natives_directory}",
            instance_root.join("natives").display().to_string(),
        ),
        ("${launcher_name}", LAUNCHER_NAME.to_string()),
        ("${launcher_version}", env!("CARGO_PKG_VERSION").to_string()),
        ("${classpath}", classpath),
        ("${classpath_separator}", classpath_separator().to_string()),
        (
            "${library_directory}",
            artifact_root.join("libraries").display().to_string(),
        ),
        (
            "${libraries_directory}",
            artifact_root.join("libraries").display().to_string(),
        ),
        ("${game_directory}", instance_root.display().to_string()),
        (
            "${assets_root}",
            artifact_root.join("assets").display().to_string(),
        ),
        ("${assets_index_name}", asset_index),
        ("${version_name}", lock.minecraft.version.clone()),
        ("${version_type}", lock.minecraft.version_type.clone()),
        ("${auth_player_name}", String::new()),
        ("${auth_session}", String::new()),
        ("${auth_access_token}", String::new()),
        ("${auth_uuid}", String::new()),
        ("${user_type}", String::new()),
        ("${user_properties}", "{}".to_string()),
        ("${clientid}", String::new()),
        ("${auth_xuid}", String::new()),
    ]);
    let mut redacted = values.clone();
    if let Some(identity) = identity {
        values.insert("${auth_player_name}", identity.profile_name.clone());
        values.insert("${auth_session}", identity.access_token.clone());
        values.insert("${auth_access_token}", identity.access_token.clone());
        values.insert("${auth_uuid}", identity.profile_id.simple().to_string());
        values.insert("${user_type}", identity.user_type.clone());
        values.insert("${user_properties}", identity.user_properties.clone());
        redacted.insert("${auth_player_name}", identity.profile_name.clone());
        redacted.insert("${auth_session}", REDACTED.to_string());
        redacted.insert("${auth_access_token}", REDACTED.to_string());
        redacted.insert("${auth_uuid}", identity.profile_id.simple().to_string());
        redacted.insert("${user_type}", identity.user_type.clone());
        redacted.insert("${user_properties}", identity.user_properties.clone());
    }
    Ok(PlaceholderSets {
        actual: values,
        redacted,
    })
}

fn assemble_arguments(
    root: &Path,
    manifest: &crate::instance::InstanceManifest,
    lock: &LauncherLock,
    classpath_explicit: bool,
    placeholders: &BTreeMap<&'static str, String>,
    authentication_arguments: &[String],
) -> Result<Vec<String>, LauncherError> {
    let mut arguments = vec![
        format!("-Xms{}M", manifest.launch.min_memory_mib),
        format!("-Xmx{}M", manifest.launch.max_memory_mib),
    ];
    arguments.extend(authentication_arguments.iter().cloned());
    arguments.extend(manifest.launch.jvm_args.iter().cloned());
    arguments.extend(expand_arguments(&lock.arguments.jvm, placeholders)?);
    match &lock.entrypoint {
        LockedEntrypoint::Jar { path } => {
            arguments.push("-jar".to_string());
            arguments.push(root.join(path_from_portable(path)).display().to_string());
        }
        LockedEntrypoint::ArgumentFile { path } => {
            arguments.push(format!(
                "@{}",
                root.join(path_from_portable(path)).display()
            ));
        }
        LockedEntrypoint::Classpath {
            main_class,
            classpath: _,
        } => {
            if !classpath_explicit {
                arguments.push("-cp".to_string());
                arguments.push(placeholders["${classpath}"].clone());
            }
            arguments.push(main_class.clone());
        }
    }
    arguments.extend(expand_arguments(&lock.arguments.game, placeholders)?);
    arguments.extend(manifest.launch.game_args.iter().cloned());
    Ok(arguments)
}

fn authentication_arguments(
    root: &Path,
    manifest: &crate::instance::InstanceManifest,
    lock: &LauncherLock,
    config: &GlobalConfig,
    identity: Option<&AccountLaunchIdentity>,
) -> Result<Vec<String>, LauncherError> {
    let client_yggdrasil = identity.filter(|identity| identity.yggdrasil_provider.is_some());
    let server_yggdrasil = manifest.server.as_ref().filter(|server| {
        server.authentication.provider == ServerAuthenticationProvider::ExternalYggdrasil
    });
    if client_yggdrasil.is_none() && server_yggdrasil.is_none() {
        return Ok(Vec::new());
    }
    let injector = lock.authlib_injector.as_ref().ok_or_else(|| {
        LauncherError::InvalidLock(
            "External Yggdrasil requires a locked authlib-injector artifact; run install"
                .to_string(),
        )
    })?;
    let injector_path = root.join(path_from_portable(&injector.path));
    if let Some(identity) = client_yggdrasil {
        let api_root = identity.yggdrasil_api_root.as_deref().ok_or_else(|| {
            LauncherError::Authentication(
                "External Yggdrasil identity has no resolved API root".to_string(),
            )
        })?;
        let metadata = identity
            .yggdrasil_prefetched_metadata
            .as_deref()
            .ok_or_else(|| {
                LauncherError::Authentication(
                    "External Yggdrasil identity has no prefetched API metadata".to_string(),
                )
            })?;
        return Ok(vec![
            format!("-javaagent:{}={api_root}", injector_path.display()),
            "-Dauthlibinjector.side=client".to_string(),
            format!(
                "-Dauthlibinjector.yggdrasil.prefetched={}",
                base64::engine::general_purpose::STANDARD.encode(metadata.as_bytes())
            ),
        ]);
    }
    let provider_id = server_yggdrasil
        .and_then(|server| server.authentication.yggdrasil_provider.as_deref())
        .ok_or_else(|| {
            LauncherError::InvalidManifest(
                "External Yggdrasil server has no provider ID".to_string(),
            )
        })?;
    let provider = config
        .yggdrasil
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .ok_or_else(|| {
            LauncherError::InvalidConfig(format!(
                "External Yggdrasil provider '{provider_id}' is not configured"
            ))
        })?;
    let api_root = normalized_api_root(&provider.api_root)?;
    Ok(vec![format!(
        "-javaagent:{}={api_root}",
        injector_path.display()
    )])
}

fn normalized_api_root(value: &str) -> Result<String, LauncherError> {
    let mut url = url::Url::parse(value).map_err(|error| {
        LauncherError::InvalidConfig(format!("Yggdrasil API root is invalid: {error}"))
    })?;
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url.to_string())
}

fn expand_arguments(
    arguments: &[String],
    placeholders: &BTreeMap<&'static str, String>,
) -> Result<Vec<String>, LauncherError> {
    arguments
        .iter()
        .map(|argument| {
            let mut expanded = argument.clone();
            for (placeholder, value) in placeholders {
                expanded = expanded.replace(placeholder, value);
            }
            if let Some(start) = expanded.find("${") {
                let end = expanded[start..]
                    .find('}')
                    .map_or(expanded.len(), |offset| start + offset + 1);
                return Err(LauncherError::UnsupportedRequirement(format!(
                    "launch argument contains unsupported placeholder '{}'",
                    &expanded[start..end]
                )));
            }
            Ok(expanded)
        })
        .collect()
}

fn path_from_portable(value: &str) -> PathBuf {
    value.split('/').collect()
}

#[cfg(windows)]
const fn classpath_separator() -> char {
    ';'
}

#[cfg(not(windows))]
const fn classpath_separator() -> char {
    ':'
}

pub fn display_command<I, S>(executable: &Path, arguments: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    std::iter::once(executable.as_os_str().to_string_lossy().into_owned())
        .chain(
            arguments
                .into_iter()
                .map(|value| value.as_ref().to_string_lossy().into_owned()),
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn successful_test_process_plan(working_directory: &Path) -> LaunchPlan {
        LaunchPlan {
            instance_id: Uuid::new_v4(),
            kind: InstanceKind::Server,
            java_executable: std::env::current_exe().unwrap(),
            working_directory: working_directory.to_path_buf(),
            arguments: vec![
                "--exact".to_string(),
                "__orbit_supervisor_child__".to_string(),
            ],
            redacted_arguments: vec![
                "--exact".to_string(),
                "__orbit_supervisor_child__".to_string(),
            ],
            sensitive_values: Vec::new(),
        }
    }

    #[test]
    fn expands_multiple_placeholders_without_shell_parsing() {
        let values = BTreeMap::from([
            ("${a}", "hello".to_string()),
            ("${b}", "path with spaces".to_string()),
        ]);
        assert_eq!(
            expand_arguments(&["${a}:${b}".to_string()], &values).unwrap(),
            ["hello:path with spaces"]
        );
    }

    #[test]
    fn rejects_unknown_placeholders_instead_of_guessing() {
        let error = expand_arguments(&["${unknown}".to_string()], &BTreeMap::new()).unwrap_err();
        assert_eq!(error.code(), "unsupported_requirement");
    }

    #[test]
    fn redacted_mapping_does_not_replace_unrelated_characters() {
        let values = BTreeMap::from([("${token}", REDACTED.to_string())]);
        let arguments = expand_arguments(
            &["--token=${token}".to_string(), "1.20.1".to_string()],
            &values,
        )
        .unwrap();
        assert_eq!(arguments, ["--token=<redacted>", "1.20.1"]);
    }

    #[tokio::test]
    async fn supervisor_restarts_natural_zero_exit_until_limit() {
        let directory = tempfile::tempdir().unwrap();
        let plan = successful_test_process_plan(directory.path());
        let instance_id = plan.instance_id;
        let config = ServerConfig {
            restart: RestartPolicy::OnUnexpectedExit,
            restart_limit: 1,
            restart_window_seconds: 60,
            restart_backoff_max_seconds: 1,
            ..ServerConfig::default()
        };
        let (_sender, mut controls) = mpsc::unbounded_channel();
        let mut events = Vec::new();
        let result = supervise_server(plan, &config, &mut controls, |event| events.push(event))
            .await
            .unwrap();

        assert_eq!(result.instance_id, instance_id);
        assert_eq!(result.generations, 2);
        assert_eq!(result.restarts, 1);
        assert!(result.final_success);
        assert!(!result.stopped_by_request);
        assert!(result.restart_limit_reached);
        assert!(events.iter().any(|event| matches!(
            event,
            SupervisorEvent::Exited {
                success: true,
                expected: false,
                ..
            }
        )));
    }
}
