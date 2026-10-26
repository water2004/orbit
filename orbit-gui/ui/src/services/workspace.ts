import { reactive } from "vue";
import {
  run,
  loadPreferences,
  Cancelled,
  type Preferences,
  type Task,
} from "./cli";
import type { Account, Detail, Instance } from "../types";
import { language, t } from "../i18n";

export const workspace = reactive({
  preferences: null as Preferences | null,
  instances: [] as Instance[],
  accounts: [] as Account[],
  accountsError: "",
  error: "",
  loading: false,
  initialized: false,
});
let inventoryRevision = 0;
export async function initialize() {
  try {
    workspace.preferences = await loadPreferences();
    language.value = workspace.preferences.language;
    await refresh();
  } catch (error) {
    workspace.error = String(error);
    workspace.initialized = true;
  }
}
export async function refresh() {
  const revision = ++inventoryRevision;
  workspace.loading = true;
  try {
    const [instances, accounts] = await Promise.allSettled([
      run<{ instances: Instance[] }>({
        kind: "launcher",
        args: ["instance", "list"],
        label: t("Loading instances", "正在加载实例"),
      }),
      run<{ accounts: Account[] }>({
        kind: "launcher",
        args: ["account", "list"],
        label: t("Loading accounts", "正在加载账户"),
      }),
    ]);
    if (revision !== inventoryRevision) return;
    if (accounts.status === "fulfilled") {
      workspace.accounts = accounts.value.accounts;
      workspace.accountsError = "";
    } else workspace.accountsError = String(accounts.reason);
    if (instances.status === "rejected") throw instances.reason;
    workspace.instances = instances.value.instances;
  } catch (error) {
    if (revision === inventoryRevision) workspace.error = String(error);
  } finally {
    if (revision === inventoryRevision) {
      workspace.loading = false;
      workspace.initialized = true;
    }
  }
}
export function orbit<T>(
  args: string[],
  label: string,
  instance: Instance | null,
  observe?: (task: Task) => void,
) {
  return run<T>(
    {
      kind: "orbit",
      args,
      label: instance?.name ? `${label} · ${instance.name}` : label,
      directory: instance?.directory,
    },
    observe,
  );
}
export function launcher<T>(
  args: string[],
  label: string,
  instance: Instance | null,
  observe?: (task: Task) => void,
) {
  return run<T>(
    {
      kind: "launcher",
      args,
      label: instance?.name ? `${label} · ${instance.name}` : label,
      instance: instance?.id,
    },
    observe,
  );
}
export async function launch(instance: Instance) {
  const binary = workspace.preferences?.launcher_binary;
  if (!binary) throw new Error("Orbit Launcher executable is not configured");
  // Read the explicit launch target: a library action need not open its page,
  // and page metadata may be stale.
  const detail = await launcher<Detail>(
    ["instance", "show"],
    t("Loading launch target", "正在读取启动实例"),
    instance,
  );
  if (detail.desired.loader === "vanilla") {
    await run({
      kind: "launcher",
      lifetime: "game",
      instance: instance.id,
      args: detail.kind === "server" ? ["server", "start"] : ["launch"],
      label:
        detail.kind === "server"
          ? t("Starting server", "正在启动服务端")
          : t("Launching game", "正在启动游戏"),
    });
    return;
  }
  await run({
    kind: "orbit",
    lifetime: "game",
    directory: instance.directory,
    args: [
      "launch",
      "--launcher",
      binary,
      "--launcher-instance",
      instance.id,
      ...(detail.kind === "server" ? ["--server"] : []),
    ],
    label:
      detail.kind === "server"
        ? t("Starting server", "正在启动服务端")
        : t("Launching game", "正在启动游戏"),
  });
}
export async function act(operation: () => Promise<unknown>) {
  try {
    await operation();
  } catch (error) {
    if (!(error instanceof Cancelled)) workspace.error = String(error);
  }
}
