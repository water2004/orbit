import { reactive } from "vue";
import {
  run,
  loadPreferences,
  savePreferences,
  Cancelled,
  type Preferences,
  type Task,
} from "./cli";
import type { Account, Detail, Instance, Package } from "../types";
import { language, t } from "../i18n";

export const workspace = reactive({
  preferences: null as Preferences | null,
  instances: [] as Instance[],
  accounts: [] as Account[],
  selected: null as Instance | null,
  detail: null as Detail | null,
  packages: [] as Package[],
  inventoryError: "",
  accountsError: "",
  error: "",
  loading: false,
});
let selectionRevision = 0;
export async function initialize(instanceId?: string) {
  try {
    workspace.preferences = await loadPreferences();
    language.value = workspace.preferences.language;
    await refresh(instanceId);
  } catch (error) {
    workspace.error = String(error);
  }
}
export async function refresh(instanceId = workspace.selected?.id) {
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
    if (accounts.status === "fulfilled") {
      workspace.accounts = accounts.value.accounts;
      workspace.accountsError = "";
    } else workspace.accountsError = String(accounts.reason);
    if (instances.status === "rejected") throw instances.reason;
    workspace.instances = instances.value.instances;
    const chosen =
      workspace.instances.find((x) => x.id === instanceId) ??
      workspace.instances.find(
        (x) => x.id === workspace.preferences?.selected_instance,
      ) ??
      workspace.instances.find((x) => x.is_default) ??
      workspace.instances[0];
    if (chosen) await selectInstance(chosen);
    else {
      workspace.selected = null;
      workspace.detail = null;
      workspace.packages = [];
    }
  } catch (error) {
    workspace.error = String(error);
  } finally {
    workspace.loading = false;
  }
}
export async function selectInstance(instance: Instance) {
  const revision = ++selectionRevision;
  workspace.selected = instance;
  workspace.detail = null;
  workspace.packages = [];
  workspace.inventoryError = "";
  if (workspace.preferences) {
    workspace.preferences.selected_instance = instance.id;
    await savePreferences(workspace.preferences);
  }
  const result = await Promise.allSettled([
    launcher<Detail>(
      ["instance", "show"],
      t("Loading installation", "正在加载实例"),
      instance,
    ),
    orbit<{ packages: Package[] }>(
      ["list"],
      t("Loading mods", "正在加载模组"),
      instance,
    ),
  ]);
  if (revision !== selectionRevision) return;
  if (result[0].status === "fulfilled") workspace.detail = result[0].value;
  else workspace.error = String(result[0].reason);
  if (result[1].status === "fulfilled")
    workspace.packages = result[1].value.packages;
  else workspace.inventoryError = String(result[1].reason);
}
export function orbit<T>(
  args: string[],
  label: string,
  instance = workspace.selected,
  observe?: (task: Task) => void,
) {
  return run<T>(
    { kind: "orbit", args, label, directory: instance?.directory },
    observe,
  );
}
export function launcher<T>(
  args: string[],
  label: string,
  instance = workspace.selected,
  observe?: (task: Task) => void,
) {
  return run<T>(
    { kind: "launcher", args, label, instance: instance?.id },
    observe,
  );
}
export async function mutate(
  args: string[],
  label: string,
  instance = workspace.selected,
  observe?: (task: Task) => void,
) {
  if (!instance) throw new Error(t("Select an installation", "请先选择实例"));
  await orbit(args, label, instance, observe);
  if (workspace.selected?.id === instance.id) await selectInstance(instance);
}
export async function launch(instance: Instance) {
  const binary = workspace.preferences?.launcher_binary;
  if (!binary) throw new Error("Orbit Launcher executable is not configured");
  await orbit(
    [
      "launch",
      "--launcher",
      binary,
      "--launcher-instance",
      instance.id,
      ...(instance.kind === "server" ? ["--server"] : []),
    ],
    t("Launching game", "正在启动游戏"),
    instance,
  );
}
export async function act(operation: () => Promise<unknown>) {
  try {
    await operation();
  } catch (error) {
    if (!(error instanceof Cancelled)) workspace.error = String(error);
  }
}
