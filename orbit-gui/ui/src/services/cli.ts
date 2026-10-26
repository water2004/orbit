import { Channel, convertFileSrc, invoke } from "@tauri-apps/api/core";
import { reactive } from "vue";
import { taskOutcome, type TaskOutcome } from "./task-outcome";
import {
  launchDiagnostics,
  type LaunchDiagnostics,
} from "./launch-diagnostics";

export type Kind = "orbit" | "launcher";
export interface Preferences {
  close_after_launch: boolean;
  orbit_binary: string;
  launcher_binary: string;
  selected_instance: string | null;
  language: "system" | "en" | "zh-CN";
  theme_mode: "system" | "light" | "dark";
  accent_theme: "indigo" | "emerald" | "amber";
}
export interface Request {
  lifetime?: "window" | "game";
  kind: Kind;
  args: string[];
  label: string;
  directory?: string;
  instance?: string;
  secret?: string;
}
export interface Choice {
  id: string;
  label: string;
  description?: string;
  data: Record<string, unknown>;
}
export interface Interaction {
  interaction_id: string;
  interaction: string;
  prompt: string;
  choices: Choice[];
  default_choice?: string;
  allow_cancel: boolean;
}
export interface Task {
  id: number;
  label: string;
  state: "running" | "waiting" | "succeeded" | "failed" | "cancelled";
  message: string;
  completed?: number;
  total?: number;
  interaction?: Interaction;
  answering?: boolean;
  outcome?: TaskOutcome;
  diagnostics?: LaunchDiagnostics;
  downloadSource?: { name: string; documentation: string };
  observationNotice?: string;
}
type Event =
  | {
      type: "progress";
      message: string;
      completed?: number;
      total?: number;
      envelope?: {
        data?: {
          event?: string;
          attribution?: { name: string; documentation: string };
          reason?: string;
        };
      };
    }
  | { type: "interaction"; envelope: Interaction }
  | { type: "error"; message: string; code?: string }
  | {
      type: "finished";
      ok: boolean;
      data?: unknown;
      message?: string;
      cancelled?: boolean;
      status?: number;
    };

export const activity = reactive({
  tasks: [] as Task[],
  open: false,
  diagnostics: null as LaunchDiagnostics | null,
});
let provisionalId = -1;
const ready = new WeakMap<Task, Promise<number>>();
export class Cancelled extends Error {}

// A Channel is supplied before spawning: even a CLI which exits immediately
// cannot finish before its consumer exists. No stdout parsing happens in Vue.
export function run<T>(
  request: Request,
  observe?: (task: Task) => void,
): Promise<T> {
  const task = reactive<Task>({
    id: provisionalId--,
    label: request.label,
    state: "running",
    message: "",
  });
  activity.tasks.push(task);
  observe?.(task);
  // Keep active requests and a bounded recent history.
  if (activity.tasks.length > 100) {
    const old = activity.tasks.findIndex((t) =>
      ["succeeded", "failed", "cancelled"].includes(t.state),
    );
    if (old >= 0) activity.tasks.splice(old, 1);
  }
  return new Promise<T>((resolve, reject) => {
    const channel = new Channel<Event>();
    let protocolError: string | undefined;
    let protocolCancelled = false;
    let finished = false;
    channel.onmessage = (event) => {
      if (finished) return;
      switch (event.type) {
        case "progress":
          if (event.envelope?.data?.event === "download_source")
            task.downloadSource = event.envelope.data.attribution;
          if (event.envelope?.data?.event === "runtime_observation_unavailable")
            task.observationNotice = event.envelope.data.reason;
          task.message = event.message;
          task.completed = event.completed;
          task.total = event.total;
          break;
        case "interaction":
          task.interaction = event.envelope;
          task.state = "waiting";
          break;
        case "error":
          protocolCancelled = event.code === "cancelled";
          protocolError = event.message;
          task.message = event.message;
          break;
        case "finished":
          finished = true;
          task.interaction = undefined;
          if (event.ok && !protocolError) {
            try {
              task.outcome = taskOutcome(request, event.data);
            } catch (error) {
              task.state = "failed";
              task.message = String(error);
              reject(error);
              break;
            }
            task.state = "succeeded";
            task.message = "";
            task.completed = undefined;
            task.total = undefined;
            resolve(event.data as T);
          } else {
            try {
              task.diagnostics = launchDiagnostics(event.data);
              if (task.diagnostics) activity.diagnostics = task.diagnostics;
            } catch (error) {
              protocolError = String(error);
            }
            task.state =
              event.cancelled || protocolCancelled ? "cancelled" : "failed";
            task.message =
              event.message ||
              protocolError ||
              (event.cancelled || protocolCancelled
                ? "Cancelled"
                : `CLI exited (${event.status ?? "?"})`);
            reject(
              event.cancelled || protocolCancelled
                ? new Cancelled(task.message)
                : new Error(task.message),
            );
          }
      }
    };
    const started = invoke<number>("run_cli", { request, events: channel });
    ready.set(task, started);
    started
      .then((id) => {
        task.id = id;
      })
      .catch((error) => {
        task.state = "failed";
        task.message = String(error);
        reject(new Error(task.message));
      });
  });
}

export async function cancel(task: Task) {
  const id = (await ready.get(task)) ?? task.id;
  if (["running", "waiting"].includes(task.state))
    await invoke("cancel_cli", { task: id });
}
export async function answer(task: Task, choice: string | null) {
  if (!task.interaction || task.answering) return;
  task.answering = true;
  const interaction = task.interaction.interaction_id;
  try {
    const id = (await ready.get(task)) ?? task.id;
    if (task.interaction?.interaction_id !== interaction) return;
    await invoke("answer_cli", { task: id, interaction, choice });
    // A subsequent question may already be delivered when the IPC call resolves.
    if (task.interaction?.interaction_id === interaction) {
      task.interaction = undefined;
      task.state = "running";
    }
  } finally {
    task.answering = false;
  }
}
export function imageUrl(path?: string) {
  return path
    ? /^https:\/\//.test(path)
      ? path
      : convertFileSrc(path)
    : undefined;
}
export const loadPreferences = () => invoke<Preferences>("preferences");
export const savePreferences = (value: Preferences) =>
  invoke("save_preferences", { value });
