use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use command_group::CommandGroup;
use orbit_machine_protocol::{ErrorEnvelope, InteractionEnvelope, ProgressEnvelope};
use serde_json::Value;
use zeroize::Zeroizing;

use crate::wire;

pub type TaskId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliKind {
    Orbit,
    Launcher,
}

pub struct ProcessRequest {
    pub kind: CliKind,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub working_directory: Option<PathBuf>,
    pub label: String,
    /// Optional first stdin line. This is used only for protocols that
    /// explicitly require stdin (for example Yggdrasil passwords) and is
    /// never copied into errors or persistent GUI state.
    pub initial_stdin: Option<Zeroizing<String>>,
}

#[derive(Debug)]
pub enum BridgeEvent {
    Progress {
        task_id: TaskId,
        envelope: ProgressEnvelope<Value>,
    },
    MachineError {
        task_id: TaskId,
        envelope: ErrorEnvelope,
    },
    Interaction {
        task_id: TaskId,
        envelope: InteractionEnvelope<Value>,
    },
    ProtocolError {
        task_id: TaskId,
        message: String,
    },
    Finished {
        task_id: TaskId,
        status: Option<i32>,
        stdout: String,
        cancelled: bool,
    },
    SpawnFailed {
        task_id: TaskId,
        message: String,
    },
}

enum Control {
    Cancel,
    StdinLine(String),
}

pub struct ProcessBridge {
    next_task: TaskId,
    events_tx: Sender<BridgeEvent>,
    events_rx: Receiver<BridgeEvent>,
    controls: HashMap<TaskId, Sender<Control>>,
}

impl Default for ProcessBridge {
    fn default() -> Self {
        let (events_tx, events_rx) = mpsc::channel();
        Self {
            next_task: 1,
            events_tx,
            events_rx,
            controls: HashMap::new(),
        }
    }
}

impl ProcessBridge {
    pub fn spawn(&mut self, request: ProcessRequest) -> TaskId {
        let task_id = self.next_task;
        self.next_task += 1;
        let (control_tx, control_rx) = mpsc::channel();
        self.controls.insert(task_id, control_tx);
        let events = self.events_tx.clone();
        let recovery_events = events.clone();
        let worker = thread::Builder::new()
            .name(format!("orbit-command-{task_id}"))
            .spawn(move || {
                if let Err(payload) = catch_unwind(AssertUnwindSafe(|| {
                    run_process(task_id, request, control_rx, events);
                })) {
                    let message = format!(
                        "{}: {}",
                        tr!("CLI process worker failed"),
                        panic_message(payload.as_ref())
                    );
                    let _ = recovery_events.send(BridgeEvent::ProtocolError { task_id, message });
                    let _ = recovery_events.send(BridgeEvent::Finished {
                        task_id,
                        status: None,
                        stdout: String::new(),
                        cancelled: false,
                    });
                }
            });
        if let Err(error) = worker {
            let _ = self.events_tx.send(BridgeEvent::SpawnFailed {
                task_id,
                message: tr!(
                    "Failed to create CLI process worker: %{error}",
                    error = error
                ),
            });
        }
        task_id
    }

    pub fn cancel(&self, task_id: TaskId) {
        if let Some(control) = self.controls.get(&task_id) {
            let _ = control.send(Control::Cancel);
        }
    }

    pub fn send_line(&self, task_id: TaskId, line: String) {
        if let Some(control) = self.controls.get(&task_id) {
            let _ = control.send(Control::StdinLine(line));
        }
    }

    pub fn drain(&mut self) -> Vec<BridgeEvent> {
        let events: Vec<_> = self.events_rx.try_iter().collect();
        for event in &events {
            if let BridgeEvent::Finished { task_id, .. }
            | BridgeEvent::SpawnFailed { task_id, .. } = event
            {
                self.controls.remove(task_id);
            }
        }
        events
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown panic")
}

fn run_process(
    task_id: TaskId,
    request: ProcessRequest,
    controls: Receiver<Control>,
    events: Sender<BridgeEvent>,
) {
    let mut command = Command::new(&request.program);
    command
        .args(&request.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(directory) = &request.working_directory {
        command.current_dir(directory);
    }

    let mut child = match spawn_process_group(&mut command) {
        Ok(child) => child,
        Err(error) => {
            let cli = match request.kind {
                CliKind::Orbit => "Orbit",
                CliKind::Launcher => "Orbit Launcher",
            };
            let _ = events.send(BridgeEvent::SpawnFailed {
                task_id,
                message: tr!(
                    "Failed to start %{cli} at %{path}: %{error}",
                    cli = cli,
                    path = request.program.display(),
                    error = error
                ),
            });
            return;
        }
    };
    let mut stdin = child.inner().stdin.take();
    if let Some(secret) = request.initial_stdin
        && let Some(handle) = stdin.as_mut()
    {
        let _ = writeln!(handle, "{}", secret.as_str());
        let _ = handle.flush();
    }
    let stdout = child.inner().stdout.take();
    let stderr = child.inner().stderr.take();

    let stdout_reader = thread::spawn(move || {
        stdout.map_or_else(
            || Ok(String::new()),
            |stdout| read_utf8_stream(stdout, "stdout"),
        )
    });

    let stderr_events = events.clone();
    let (stderr_failure_tx, stderr_failure_rx) = mpsc::channel();
    let stderr_reader = thread::spawn(move || {
        let Some(stderr) = stderr else {
            return;
        };
        let mut reader = BufReader::new(stderr);
        let mut bytes = Vec::new();
        loop {
            bytes.clear();
            let read = match reader.read_until(b'\n', &mut bytes) {
                Ok(read) => read,
                Err(error) => {
                    let _ = stderr_events.send(BridgeEvent::ProtocolError {
                        task_id,
                        message: tr!("Failed to read CLI stderr: %{error}", error = error),
                    });
                    let _ = stderr_failure_tx.send(());
                    break;
                }
            };
            if read == 0 {
                break;
            }
            let line = match std::str::from_utf8(&bytes) {
                Ok(line) => line.trim_end_matches(['\r', '\n']),
                Err(error) => {
                    let _ = stderr_events.send(BridgeEvent::ProtocolError {
                        task_id,
                        message: tr!(
                            "CLI %{stream} is not valid UTF-8 at byte %{offset}",
                            stream = "stderr",
                            offset = error.valid_up_to()
                        ),
                    });
                    let _ = stderr_failure_tx.send(());
                    continue;
                }
            };
            if let Some(interaction) = wire::interaction_line(line) {
                match interaction {
                    Ok(envelope) => {
                        let _ = stderr_events.send(BridgeEvent::Interaction { task_id, envelope });
                    }
                    Err(error) => {
                        let _ = stderr_events.send(BridgeEvent::ProtocolError {
                            task_id,
                            message: tr!(
                                "Invalid CLI interaction message: %{error}",
                                error = error
                            ),
                        });
                        let _ = stderr_failure_tx.send(());
                    }
                }
            } else if let Some(progress) = wire::progress_line(line) {
                match progress {
                    Ok(envelope) => {
                        let _ = stderr_events.send(BridgeEvent::Progress { task_id, envelope });
                    }
                    Err(error) => {
                        let _ = stderr_events.send(BridgeEvent::ProtocolError {
                            task_id,
                            message: tr!("Invalid CLI progress message: %{error}", error = error),
                        });
                        let _ = stderr_failure_tx.send(());
                    }
                }
            } else if let Some(machine_error) = wire::error_line(line) {
                match machine_error {
                    Ok(envelope) => {
                        let _ = stderr_events.send(BridgeEvent::MachineError { task_id, envelope });
                    }
                    Err(error) => {
                        let _ = stderr_events.send(BridgeEvent::ProtocolError {
                            task_id,
                            message: tr!("Invalid CLI error message: %{error}", error = error),
                        });
                        let _ = stderr_failure_tx.send(());
                    }
                }
            }
        }
    });

    let mut cancelled = false;
    let mut protocol_failed = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) => {}
            Err(error) => {
                let _ = events.send(BridgeEvent::ProtocolError {
                    task_id,
                    message: tr!("Failed to query child process: %{error}", error = error),
                });
                break None;
            }
        }

        if !protocol_failed && stderr_failure_rx.try_recv().is_ok() {
            protocol_failed = true;
            let _ = child.kill();
        }

        match controls.recv_timeout(Duration::from_millis(40)) {
            Ok(Control::Cancel) => {
                cancelled = true;
                let _ = child.kill();
            }
            Ok(Control::StdinLine(line)) => {
                if let Some(handle) = stdin.as_mut() {
                    let _ = writeln!(handle, "{line}");
                    let _ = handle.flush();
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {}
        }
    };

    drop(stdin);
    let stdout = match stdout_reader.join() {
        Ok(Ok(stdout)) => stdout,
        Ok(Err(message)) => {
            let _ = events.send(BridgeEvent::ProtocolError { task_id, message });
            String::new()
        }
        Err(_) => {
            let _ = events.send(BridgeEvent::ProtocolError {
                task_id,
                message: tr!("CLI stdout reader thread panicked").into_owned(),
            });
            String::new()
        }
    };
    let _ = stderr_reader.join();
    let _ = events.send(BridgeEvent::Finished {
        task_id,
        status,
        stdout,
        cancelled,
    });
}

fn spawn_process_group(command: &mut Command) -> std::io::Result<command_group::GroupChild> {
    let mut group = command.group();
    #[cfg(target_os = "windows")]
    group.creation_flags(CREATE_NO_WINDOW);
    group.spawn()
}

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn read_utf8_stream(mut reader: impl Read, stream: &str) -> Result<String, String> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).map_err(|error| {
        tr!(
            "Failed to read CLI %{stream}: %{error}",
            stream = stream,
            error = error
        )
    })?;
    String::from_utf8(bytes).map_err(|error| {
        tr!(
            "CLI %{stream} is not valid UTF-8 at byte %{offset}",
            stream = stream,
            offset = error.utf8_error().valid_up_to()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_streams_require_utf8_instead_of_silently_replacing_bytes() {
        assert_eq!(
            read_utf8_stream("中文\n".as_bytes(), "stdout").unwrap(),
            "中文\n"
        );
        let error = read_utf8_stream([0xff, b'\n'].as_slice(), "stdout").unwrap_err();
        assert_eq!(error, "CLI stdout is not valid UTF-8 at byte 0");
    }
}
