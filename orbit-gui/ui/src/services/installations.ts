import { join, tempDir } from "@tauri-apps/api/path";
import {
  launcher,
  orbit,
  refresh,
  selectInstance,
  workspace,
} from "./workspace";
import { importArguments } from "./commands";
import { t } from "../i18n";
import type { Detail, Instance } from "../types";

export interface InstallDraft {
  name: string;
  kind: "client" | "server";
  minecraft: string;
  loader: string;
  loaderVersion: string;
  serverDirectory: string;
}
export interface Pack {
  source: string;
  name: string;
  format: string;
  version: string;
  targets: string[];
  minecraft: string;
  loader: string;
  loader_version?: string;
  orbit_content: boolean;
  launcher_state: boolean;
  optional_files: { path: string; targets: string[] }[];
}
export interface Migration {
  target: Detail;
  sourcePack: string;
  exported: boolean;
  registered: boolean;
}

export function installArguments(draft: InstallDraft) {
  return [
    "install",
    "--new",
    draft.name,
    "--kind",
    draft.kind,
    "--minecraft",
    draft.minecraft,
    "--loader",
    draft.loader,
    ...(draft.loader === "vanilla"
      ? []
      : ["--loader-version", draft.loaderVersion]),
    ...(draft.kind === "server"
      ? ["--server-directory", draft.serverDirectory]
      : []),
  ];
}
export async function initializeOrbit(detail: Detail) {
  await orbit(
    [
      "init",
      detail.name,
      "--mc-version",
      detail.desired.minecraft,
      "--modloader",
      detail.desired.loader,
      ...(detail.desired.loader_version
        ? ["--modloader-version", detail.desired.loader_version]
        : []),
    ],
    t("Initializing mod workspace", "正在初始化模组工作区"),
    detail,
  );
}
export async function createInstallation(
  draft: InstallDraft,
  pack?: Pack,
  optional: string[] = [],
) {
  const installed = await launcher<{ instance_id: string }>(
    [...installArguments(draft), ...(pack ? ["--from", pack.source] : [])],
    t("Creating installation", "正在创建实例"),
    null,
  );
  const detail = await launcher<Detail>(
    ["instance", "show"],
    t("Loading new installation", "正在加载新实例"),
    { id: installed.instance_id } as Instance,
  );
  if (pack?.orbit_content) {
    await initializeOrbit(detail);
    await orbit(
      importArguments(pack.source, optional),
      t("Importing mod content", "正在导入模组内容"),
      detail,
    );
  }
  await refresh();
  await selectInstance(detail);
  return detail;
}
export async function prepareMigration(
  draft: InstallDraft,
  source: Instance,
): Promise<Migration> {
  // Freeze the source before creating any target. These are the same exporters
  // used by ordinary user exports; the GUI does not inspect or modify archives.
  const sourcePack = await join(
    await tempDir(),
    `orbit-migration-${crypto.randomUUID()}.orbitbundle`,
  );
  await orbit(
    ["export", sourcePack, "--format", "orbit", "--content", "mods-and-data"],
    t("Exporting migration source", "正在导出迁移源"),
    source,
  );
  await launcher(
    ["export", sourcePack, "--base", sourcePack],
    t("Exporting worlds and game state", "正在导出世界与游戏状态"),
    source,
  );
  const installed = await launcher<{ instance_id: string }>(
    [...installArguments(draft), "--from", sourcePack],
    t("Creating migration target", "正在创建迁移目标"),
    null,
  );
  const target = await launcher<Detail>(
    ["instance", "show"],
    t("Loading migration target", "正在加载迁移目标"),
    { id: installed.instance_id } as Instance,
  );
  return { target, sourcePack, exported: false, registered: false };
}
export async function applyMigration(review: Migration) {
  // The same CLI process selects the solution and confirms its exact plan.
  // Retrying materialization must not repeat a committed export or re-solve.
  if (!review.exported) {
    const result = await orbit<{ export?: { applied: boolean } }>(
      [
        "migrate",
        "export",
        review.target.directory,
        "--source-pack",
        review.sourcePack,
        "--consume-source-pack",
      ],
      t("Applying mod migration", "正在应用模组迁移"),
      review.target,
    );
    if (!result.export?.applied) return false;
    review.exported = true;
  }
  if (!review.registered) {
    await orbit(
      ["instances", "register", review.target.name, review.target.directory],
      t("Registering mod workspace", "正在注册模组工作区"),
      null,
    );
    review.registered = true;
  }
  await orbit(
    ["install"],
    t("Installing migrated mods", "正在安装迁移后的模组"),
    review.target,
  );
  await refresh();
  await selectInstance(review.target);
  return true;
}
export async function updateLoader(
  version: string,
  instance: Instance,
  syncOrbit: boolean,
) {
  await launcher(
    ["instance", "configure", "--loader-version", version],
    t("Configuring loader update", "正在配置加载器更新"),
    instance,
  );
  await launcher(
    ["install"],
    t("Installing loader update", "正在安装加载器更新"),
    instance,
  );
  if (syncOrbit)
    await orbit(
      ["sync"],
      t("Synchronizing loader metadata", "正在同步加载器元数据"),
      instance,
    );
  if (workspace.selected?.id === instance.id) await selectInstance(instance);
}
