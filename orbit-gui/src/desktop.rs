//! Tauri is a transport for the existing CLI process protocol, not a domain API.
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use orbit_machine_protocol::InteractionResponse;
use serde::Deserialize;
use serde_json::{Value, json};
use tauri::{Manager, State, ipc::Channel};
use tauri_plugin_opener::OpenerExt;
use zeroize::Zeroizing;

use crate::preferences::Preferences;
use crate::string_rule::StringRuleDraft;
use crate::{
    process::{BridgeEvent, CliKind, ProcessBridge, ProcessRequest, TaskId},
    wire,
};

#[tauri::command]
pub fn parse_string_rule(source: String) -> Result<StringRuleDraft, String> {
    StringRuleDraft::parse(&source).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn render_string_rule(draft: StringRuleDraft) -> Result<String, String> {
    draft
        .expression()
        .ok_or_else(|| "Incomplete string condition".into())
}

pub struct Desktop {
    bridge: ProcessBridge,
    channels: HashMap<TaskId, Channel<Value>>,
    preferences: Preferences,
    stopped: Arc<AtomicBool>,
    directories: std::collections::HashSet<PathBuf>,
}

/// OS integration is restricted to exact directories supplied by the CLI.
#[tauri::command]
pub fn open_directory(
    path: PathBuf,
    app: tauri::AppHandle,
    state: State<'_, Mutex<Desktop>>,
) -> Result<(), String> {
    if !state
        .lock()
        .map_err(|e| e.to_string())?
        .directories
        .contains(&path)
        || !path.is_dir()
    {
        return Err("This directory is unavailable or was not supplied by the CLI".into());
    }
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

pub fn shutdown(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<Mutex<Desktop>>()
        && let Ok(state) = state.lock()
    {
        state.stopped.store(true, Ordering::Relaxed);
        for id in state.channels.keys() {
            state.bridge.cancel(*id);
        }
    }
}

#[derive(Deserialize)]
pub struct CommandRequest {
    pub kind: CliKind,
    pub args: Vec<String>,
    pub directory: Option<PathBuf>,
    pub instance: Option<String>,
    pub secret: Option<String>,
}

#[tauri::command]
pub fn preferences(state: State<'_, Mutex<Desktop>>) -> Result<Preferences, String> {
    Ok(state.lock().map_err(|e| e.to_string())?.preferences.clone())
}

#[tauri::command]
pub fn save_preferences(
    mut value: Preferences,
    state: State<'_, Mutex<Desktop>>,
) -> Result<(), String> {
    let mut state = state.lock().map_err(|e| e.to_string())?;
    value.retain_installation_binding(&state.preferences);
    value.save()?;
    orbit_i18n::install(value.language);
    state.preferences = value;
    Ok(())
}

#[tauri::command]
pub fn run_cli(
    request: CommandRequest,
    events: Channel<Value>,
    state: State<'_, Mutex<Desktop>>,
) -> Result<TaskId, String> {
    let mut state = state.lock().map_err(|e| e.to_string())?;
    let mut args = vec![
        "--language".into(),
        state.preferences.language.argument().into(),
        "--output-format".into(),
        "json".into(),
        "--progress-format".into(),
        "ndjson".into(),
    ];
    let program = match request.kind {
        CliKind::Orbit => state.preferences.orbit_binary.clone(),
        CliKind::Launcher => {
            if let Some(instance) = request.instance {
                args.extend(["--instance".into(), instance]);
            }
            state.preferences.launcher_binary.clone()
        }
    };
    if !program.is_absolute() || !program.is_file() {
        return Err(format!(
            "CLI executable does not exist: {}",
            program.display()
        ));
    }
    args.extend(request.args);
    let id = state.bridge.spawn(ProcessRequest {
        kind: request.kind,
        program,
        args,
        working_directory: request.directory,
        initial_stdin: request.secret.map(Zeroizing::new),
    });
    state.channels.insert(id, events);
    Ok(id)
}

#[tauri::command]
pub fn cancel_cli(task: TaskId, state: State<'_, Mutex<Desktop>>) -> Result<(), String> {
    state.lock().map_err(|e| e.to_string())?.bridge.cancel(task);
    Ok(())
}

#[tauri::command]
pub fn answer_cli(
    task: TaskId,
    interaction: String,
    choice: Option<String>,
    state: State<'_, Mutex<Desktop>>,
) -> Result<(), String> {
    let response = match choice {
        Some(choice) => InteractionResponse::selected(interaction, choice),
        None => InteractionResponse::cancelled(interaction),
    };
    state.lock().map_err(|e| e.to_string())?.bridge.send_line(
        task,
        serde_json::to_string(&response).map_err(|e| e.to_string())?,
    );
    Ok(())
}

fn permit_resources(
    value: &Value,
    app: &tauri::AppHandle,
    directories: &mut std::collections::HashSet<PathBuf>,
) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if matches!(key.as_str(), "icon_path" | "avatar_path") {
                    if let Some(path) = value.as_str() {
                        let _ = app.asset_protocol_scope().allow_file(path);
                    }
                } else if key == "directory" {
                    if let Some(path) = value.as_str() {
                        directories.insert(PathBuf::from(path));
                    }
                } else {
                    permit_resources(value, app, directories);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                permit_resources(item, app, directories);
            }
        }
        _ => {}
    }
}

pub fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let preferences = Preferences::load().map_err(std::io::Error::other)?;
    orbit_i18n::install(preferences.language);
    let stopped = Arc::new(AtomicBool::new(false));
    app.manage(Mutex::new(Desktop {
        bridge: ProcessBridge::default(),
        channels: HashMap::new(),
        preferences,
        stopped: stopped.clone(),
        directories: Default::default(),
    }));
    let handle = app.handle().clone();
    std::thread::Builder::new()
        .name("orbit-cli-events".into())
        .spawn(move || dispatch_events(handle, stopped))?;
    Ok(())
}

fn dispatch_events(app: tauri::AppHandle, stopped: Arc<AtomicBool>) {
    while !stopped.load(Ordering::Relaxed) {
        let state = app.state::<Mutex<Desktop>>();
        let Ok(mut state) = state.lock() else {
            break;
        };
        for event in state.bridge.drain() {
            let (id, finished, payload) = event_payload(event, &app, &mut state.directories);
            if let Some(channel) = state.channels.get(&id)
                && channel.send(payload).is_err()
                && !finished
            {
                state.bridge.cancel(id);
            }
            if finished {
                state.channels.remove(&id);
            }
        }
        drop(state);
        std::thread::sleep(Duration::from_millis(40));
    }
}

fn event_payload(
    event: BridgeEvent,
    app: &tauri::AppHandle,
    directories: &mut std::collections::HashSet<PathBuf>,
) -> (TaskId, bool, Value) {
    match event {
        BridgeEvent::Progress { task_id, envelope } => {
            let (completed, total) = wire::progress_numbers(&envelope.data);
            let message = wire::progress_label(&envelope.data);
            (
                task_id,
                false,
                json!({
                    "type": "progress", "message": message, "completed": completed,
                    "total": total, "envelope": envelope,
                }),
            )
        }
        BridgeEvent::Interaction { task_id, envelope } => (
            task_id,
            false,
            json!({"type": "interaction", "envelope": envelope}),
        ),
        BridgeEvent::MachineError { task_id, envelope } => (
            task_id,
            false,
            json!({"type": "error", "message": envelope.message, "code": envelope.code}),
        ),
        BridgeEvent::ProtocolError { task_id, message } => {
            (task_id, false, json!({"type": "error", "message": message}))
        }
        BridgeEvent::SpawnFailed { task_id, message } => (
            task_id,
            true,
            json!({"type": "finished", "ok": false, "message": message}),
        ),
        BridgeEvent::Finished {
            task_id,
            status,
            stdout,
            cancelled,
        } => {
            let payload = if cancelled {
                json!({"type": "finished", "ok": false, "cancelled": true})
            } else if status == Some(0) {
                match success_payload(&stdout, app, directories) {
                    Ok(data) => json!({"type": "finished", "ok": true, "data": data}),
                    Err(error) => {
                        json!({"type": "finished", "ok": false, "message": error.to_string()})
                    }
                }
            } else {
                json!({"type": "finished", "ok": false, "status": status})
            };
            (task_id, true, payload)
        }
    }
}

fn success_payload(
    stdout: &str,
    app: &tauri::AppHandle,
    directories: &mut std::collections::HashSet<PathBuf>,
) -> anyhow::Result<Value> {
    let envelope = wire::success_document(stdout)?;
    permit_resources(&envelope.result, app, directories);
    if envelope.command == "audit" {
        Ok(serde_json::to_value(wire::audit_summary(
            &envelope.result,
        )?)?)
    } else {
        Ok(envelope.result)
    }
}
