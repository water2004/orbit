use std::fs::{File, OpenOptions};
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use orbit_launcher_core::{LauncherError, SupervisorControl, SupervisorEvent};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, watch};
use uuid::Uuid;

const IPC_SCHEMA: u32 = 1;
const MAX_MESSAGE_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorPhase {
    Starting,
    Running,
    Backoff,
    Stopping,
    Stopped,
    Failed,
}

impl SupervisorPhase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Backoff => "backoff",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupervisorState {
    pub instance_id: Uuid,
    pub supervisor_pid: u32,
    pub child_pid: Option<u32>,
    pub phase: SupervisorPhase,
    pub generation: u32,
    pub restarts: u32,
    pub started_at_unix_seconds: u64,
    pub last_exit_code: Option<i32>,
    pub restart_limit_reached: bool,
}

impl SupervisorState {
    pub fn starting(instance_id: Uuid) -> Result<Self, LauncherError> {
        Ok(Self {
            instance_id,
            supervisor_pid: std::process::id(),
            child_pid: None,
            phase: SupervisorPhase::Starting,
            generation: 0,
            restarts: 0,
            started_at_unix_seconds: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| {
                    LauncherError::Launch(format!("system clock is invalid: {error}"))
                })?
                .as_secs(),
            last_exit_code: None,
            restart_limit_reached: false,
        })
    }

    pub fn apply(&mut self, event: &SupervisorEvent) {
        match event {
            SupervisorEvent::Spawned { pid, generation } => {
                self.phase = SupervisorPhase::Running;
                self.child_pid = Some(*pid);
                self.generation = *generation;
            }
            SupervisorEvent::StopRequested => self.phase = SupervisorPhase::Stopping,
            SupervisorEvent::Exited { exit_code, .. } => {
                self.child_pid = None;
                self.last_exit_code = *exit_code;
            }
            SupervisorEvent::Backoff {
                restart_attempt, ..
            } => {
                self.phase = SupervisorPhase::Backoff;
                self.restarts = *restart_attempt;
            }
            SupervisorEvent::Restarting { generation } => {
                self.phase = SupervisorPhase::Starting;
                self.generation = *generation;
            }
            SupervisorEvent::RestartLimitReached { .. } => {
                self.phase = SupervisorPhase::Failed;
                self.restart_limit_reached = true;
            }
            SupervisorEvent::Stopped if self.restart_limit_reached => {}
            SupervisorEvent::Stopped => self.phase = SupervisorPhase::Stopped,
            SupervisorEvent::Output { .. } | SupervisorEvent::CommandSent { .. } => {}
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum IpcRequest {
    Status,
    SendCommand { value: String },
    Stop,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IpcResponse {
    pub schema: u32,
    pub accepted: bool,
    pub state: SupervisorState,
    pub message: String,
}

pub struct SupervisorLock {
    _file: File,
}

impl SupervisorLock {
    pub fn acquire(instance_root: &Path) -> Result<Self, LauncherError> {
        let directory = instance_root.join(".orbit-launcher");
        std::fs::create_dir_all(&directory)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join("supervisor.lock"))?;
        file.try_lock_exclusive().map_err(|error| {
            LauncherError::Launch(format!(
                "another supervisor already owns this instance: {error}"
            ))
        })?;
        Ok(Self { _file: file })
    }
}

pub struct IpcServer {
    inner: platform::Listener,
}

impl IpcServer {
    pub async fn bind(data_dir: &Path, instance_id: Uuid) -> Result<Self, LauncherError> {
        Ok(Self {
            inner: platform::bind(data_dir, instance_id).await?,
        })
    }

    pub async fn serve(
        self,
        controls: mpsc::UnboundedSender<SupervisorControl>,
        state: Arc<RwLock<SupervisorState>>,
        shutdown: watch::Receiver<bool>,
    ) -> Result<(), LauncherError> {
        platform::serve(self.inner, controls, state, shutdown).await
    }
}

pub async fn request(
    data_dir: &Path,
    instance_id: Uuid,
    request: IpcRequest,
) -> Result<Option<IpcResponse>, LauncherError> {
    platform::request(data_dir, instance_id, request).await
}

async fn handle_stream<S>(
    stream: S,
    controls: &mpsc::UnboundedSender<SupervisorControl>,
    state: &Arc<RwLock<SupervisorState>>,
) -> Result<(), LauncherError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (read, mut write) = tokio::io::split(stream);
    let mut bytes = Vec::new();
    let mut reader = BufReader::new(read).take(MAX_MESSAGE_BYTES + 1);
    reader.read_until(b'\n', &mut bytes).await?;
    if bytes.len() as u64 > MAX_MESSAGE_BYTES {
        return Err(LauncherError::Launch(
            "supervisor IPC request exceeds 64 KiB".to_string(),
        ));
    }
    let request: IpcRequest = serde_json::from_slice(&bytes).map_err(|error| {
        LauncherError::Launch(format!("invalid supervisor IPC request: {error}"))
    })?;
    let current_phase = state
        .read()
        .map_err(|_| LauncherError::Launch("supervisor state lock was poisoned".to_string()))?
        .phase;
    let (accepted, message) = match request {
        IpcRequest::Status => (true, "status".to_string()),
        IpcRequest::SendCommand { value } if value == "stop" => {
            controls.send(SupervisorControl::Stop).map_err(|_| {
                LauncherError::Launch("supervisor control channel is closed".to_string())
            })?;
            (true, "stop requested".to_string())
        }
        IpcRequest::SendCommand { .. } if current_phase != SupervisorPhase::Running => (
            false,
            format!(
                "server command rejected while supervisor is {}",
                current_phase.as_str()
            ),
        ),
        IpcRequest::SendCommand { value } => {
            controls
                .send(SupervisorControl::Command(value))
                .map_err(|_| {
                    LauncherError::Launch("supervisor control channel is closed".to_string())
                })?;
            (true, "command queued".to_string())
        }
        IpcRequest::Stop => {
            controls.send(SupervisorControl::Stop).map_err(|_| {
                LauncherError::Launch("supervisor control channel is closed".to_string())
            })?;
            (true, "stop requested".to_string())
        }
    };
    let snapshot = state
        .read()
        .map_err(|_| LauncherError::Launch("supervisor state lock was poisoned".to_string()))?
        .clone();
    let response = IpcResponse {
        schema: IPC_SCHEMA,
        accepted,
        state: snapshot,
        message,
    };
    let mut serialized = serde_json::to_vec(&response).map_err(|error| {
        LauncherError::Launch(format!("failed to serialize supervisor response: {error}"))
    })?;
    serialized.push(b'\n');
    write.write_all(&serialized).await?;
    write.shutdown().await?;
    Ok(())
}

async fn request_stream<S>(stream: S, request: IpcRequest) -> Result<IpcResponse, LauncherError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (read, mut write) = tokio::io::split(stream);
    let mut serialized = serde_json::to_vec(&request).map_err(|error| {
        LauncherError::Launch(format!("failed to serialize supervisor request: {error}"))
    })?;
    serialized.push(b'\n');
    write.write_all(&serialized).await?;
    write.shutdown().await?;
    let mut bytes = Vec::new();
    let mut reader = BufReader::new(read).take(MAX_MESSAGE_BYTES + 1);
    reader.read_until(b'\n', &mut bytes).await?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_MESSAGE_BYTES {
        return Err(LauncherError::Launch(
            "supervisor returned an empty or oversized response".to_string(),
        ));
    }
    let response: IpcResponse = serde_json::from_slice(&bytes).map_err(|error| {
        LauncherError::Launch(format!("invalid supervisor IPC response: {error}"))
    })?;
    if response.schema != IPC_SCHEMA || response.state.instance_id.is_nil() {
        return Err(LauncherError::Launch(
            "supervisor IPC response has an unsupported schema".to_string(),
        ));
    }
    Ok(response)
}

#[cfg(unix)]
fn socket_directory(data_dir: &Path) -> PathBuf {
    data_dir.join("supervisors")
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tokio::net::{UnixListener, UnixStream};

    pub struct Listener {
        listener: UnixListener,
        path: PathBuf,
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    fn path(data_dir: &Path, instance_id: Uuid) -> PathBuf {
        socket_directory(data_dir).join(format!("{instance_id}.sock"))
    }

    pub async fn bind(data_dir: &Path, instance_id: Uuid) -> Result<Listener, LauncherError> {
        let directory = socket_directory(data_dir);
        std::fs::create_dir_all(&directory)?;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
        let path = path(data_dir, instance_id);
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        Ok(Listener { listener, path })
    }

    pub async fn serve(
        listener: Listener,
        controls: mpsc::UnboundedSender<SupervisorControl>,
        state: Arc<RwLock<SupervisorState>>,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<(), LauncherError> {
        loop {
            tokio::select! {
                accepted = listener.listener.accept() => {
                    let (stream, _) = accepted?;
                    handle_stream(stream, &controls, &state).await?;
                }
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        return Ok(());
                    }
                }
            }
        }
    }

    pub async fn request(
        data_dir: &Path,
        instance_id: Uuid,
        request: IpcRequest,
    ) -> Result<Option<IpcResponse>, LauncherError> {
        match UnixStream::connect(path(data_dir, instance_id)).await {
            Ok(stream) => request_stream(stream, request).await.map(Some),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) =>
            {
                Ok(None)
            }
            Err(error) => Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_lock_has_single_owner() {
        let directory = tempfile::tempdir().unwrap();
        let first = SupervisorLock::acquire(directory.path()).unwrap();
        assert!(SupervisorLock::acquire(directory.path()).is_err());
        drop(first);
        SupervisorLock::acquire(directory.path()).unwrap();
    }

    #[tokio::test]
    async fn local_ipc_reports_supervisor_state() {
        let directory = tempfile::tempdir().unwrap();
        let instance_id = Uuid::new_v4();
        let server = IpcServer::bind(directory.path(), instance_id)
            .await
            .unwrap();
        let state = Arc::new(RwLock::new(SupervisorState::starting(instance_id).unwrap()));
        let (controls, _receiver) = mpsc::unbounded_channel();
        let (shutdown, shutdown_receiver) = watch::channel(false);
        let task = tokio::spawn(server.serve(controls, Arc::clone(&state), shutdown_receiver));

        let response = request(directory.path(), instance_id, IpcRequest::Status)
            .await
            .unwrap()
            .unwrap();
        assert!(response.accepted);
        assert_eq!(response.state.instance_id, instance_id);
        assert_eq!(response.state.supervisor_pid, std::process::id());

        shutdown.send(true).unwrap();
        task.await.unwrap().unwrap();
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};

    pub struct Listener {
        server: NamedPipeServer,
        name: String,
    }

    fn name(instance_id: Uuid) -> String {
        format!(r"\\.\pipe\orbit-launcher-{instance_id}")
    }

    fn create(name: &str, first: bool) -> Result<NamedPipeServer, LauncherError> {
        let mut options = ServerOptions::new();
        options.reject_remote_clients(true);
        if first {
            options.first_pipe_instance(true);
        }
        options.create(name).map_err(LauncherError::from)
    }

    pub async fn bind(_data_dir: &Path, instance_id: Uuid) -> Result<Listener, LauncherError> {
        let name = name(instance_id);
        Ok(Listener {
            server: create(&name, true)?,
            name,
        })
    }

    pub async fn serve(
        mut listener: Listener,
        controls: mpsc::UnboundedSender<SupervisorControl>,
        state: Arc<RwLock<SupervisorState>>,
        mut shutdown: watch::Receiver<bool>,
    ) -> Result<(), LauncherError> {
        loop {
            tokio::select! {
                connected = listener.server.connect() => {
                    connected?;
                    let next = create(&listener.name, false)?;
                    let connected = std::mem::replace(&mut listener.server, next);
                    handle_stream(connected, &controls, &state).await?;
                }
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        return Ok(());
                    }
                }
            }
        }
    }

    pub async fn request(
        _data_dir: &Path,
        instance_id: Uuid,
        request: IpcRequest,
    ) -> Result<Option<IpcResponse>, LauncherError> {
        match ClientOptions::new().open(name(instance_id)) {
            Ok(stream) => request_stream(stream, request).await.map(Some),
            Err(error) if matches!(error.raw_os_error(), Some(2 | 231 | 233)) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
}
