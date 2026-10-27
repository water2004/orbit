// Browser-only visual fixture. Does not invoke a CLI, open user files, or ship in dist.
import {
  mockIPC,
  mockWindows,
  mockConvertFileSrc,
} from "@tauri-apps/api/mocks";
mockWindows("main");
mockConvertFileSrc("windows");
// Fixture artwork is served by Vite, not the native asset protocol. Only this
// excluded test entry replaces the mock's conversion; production stays strict.
window.__TAURI_INTERNALS__.convertFileSrc = (path: string) => path;
import type { Channel } from "@tauri-apps/api/core";
import { activity, type Request } from "../src/services/cli";
import { discoverySession } from "../src/services/discovery";
const instances = [
  {
    id: "demo",
    name: "Creative · Fabric",
    directory: "X:/fixture/creative",
    kind: "client",
    game_version: "26.2",
    loader: "fabric",
    is_default: true,
  },
];
if (new URLSearchParams(location.search).has("instance-picker")) {
  instances.push(
    ...[
      ["26.1.2-Fabric", "26.1.2"],
      ["26.2-Fabric", "26.2"],
      ["26.3-fabric-migrate", "26.3"],
      ["一个名称很长的建筑与生存联机测试实例 · Fabric", "26.3"],
    ].map(([name, game_version], index) => ({
      ...instances[0],
      id: `picker-${index}`,
      name,
      game_version,
      is_default: false,
    })),
  );
}
const detail = {
  ...instances[0],
  desired: { minecraft: "26.2", loader: "fabric", loader_version: "0.20.0" },
  installed: { minecraft: "26.2", loader: "fabric", loader_version: "0.20.0" },
};
const packages = ["sodium", "iris", "fabric-api"].map((mod_id, i) => ({
  mod_id,
  version: ["0.9.1", "1.11.2", "0.152.0"][i],
  enabled: true,
  environment: "client",
  optional: false,
  remotes: [`modrinth:project-${i}`],
  dependencies: [],
  bundled_count: 0,
}));
const results = [
  "Sodium",
  "Sodium Extra",
  "Reese's Sodium Options",
  "Sodium Shadowy Path Blocks",
].map((name, i) => ({
  name,
  platform: "modrinth",
  project_id: i ? `new-${i}` : "project-0",
  description:
    "Improve your Minecraft experience with performance improvements and more customization options.",
  latest_version: "0.9.1",
  downloads: 12000000,
  categories: ["optimization"],
  compatible: true,
  icon_url: "/orbit.svg",
}));
Object.assign(discoverySession("demo"), {
  query: "sodium",
  results,
  searched: true,
});
const prefs = {
  orbit_binary: "fixture",
  launcher_binary: "fixture",
  language: "zh-CN",
  theme_mode: new URL(location.href).searchParams.get("theme") || "light",
  accent_theme: "indigo",
};
let id = 0;
mockIPC((cmd, args) => {
  if (cmd === "plugin:window|is_maximized") return false;
  if (cmd === "preferences") return prefs;
  if (cmd === "save_preferences") return;
  if (cmd === "run_cli") {
    const { request, events } = args as {
      request: Request;
      events: Channel<unknown>;
    };
    const key = request.args.slice(0, 2).join(" ");
    let data: unknown;
    if (key === "upgrade sodium") {
      const taskId = ++id;
      events.onmessage({
        type: "progress",
        message: "正在选择 sodium",
        completed: 4,
        total: 10,
      });
      setTimeout(
        () =>
          events.onmessage({
            type: "finished",
            ok: true,
            data: {
              changes: [],
              warnings: [],
              diagnostics: [
                {
                  package: "sodium",
                  candidate_version: "0.10",
                  facts: ["测试数据：另一个模组的依赖约束阻止该版本。"],
                },
              ],
            },
          }),
        1200,
      );
      return taskId;
    }
    if (key === "instance list") data = { instances };
    else if (key === "instance show") {
      const selected =
        instances.find((instance) => instance.id === request.instance) ||
        instances[0];
      data = {
        ...detail,
        ...selected,
        desired: { ...detail.desired, minecraft: selected.game_version },
        installed: { ...detail.installed, minecraft: selected.game_version },
      };
    } else if (key === "account list")
      data = {
        accounts: [
          {
            id: "offline",
            profile_name: "Player",
            provider: "offline",
            is_default: true,
            authentication_state: "offline",
          },
        ],
      };
    else if (request.args[0] === "list") data = { packages };
    else if (request.args[0] === "search") data = { results };
    else if (key === "versions minecraft")
      data = {
        versions: [
          { id: "26.2", version_type: "release" },
          { id: "26.1.2", version_type: "release" },
        ],
      };
    else if (key === "ownership sodium")
      data = {
        mod_id: "sodium",
        artifacts: [
          { path: "mods/sodium.jar", present: true, scope: "instance" },
        ],
        data: [
          {
            path: "config/sodium-options.json",
            scope: "instance",
            kind: "file",
            preserved: [],
          },
        ],
      };
    else if (key === "java list")
      data = {
        sources: ["system", "mojang"],
        runtimes: [
          {
            runtime_id: "fixture-system",
            provider: "system",
            executable: "X:/fixture/java/bin/java",
            major: 25,
            version: "25.0.2",
            verified: true,
            bytes: null,
          },
        ],
      };
    else if (key === "versions loader")
      data = {
        versions: [
          { version: "0.20.1", stable: true },
          { version: "0.20.0", stable: false },
          { version: "0.19.5", stable: true },
        ],
      };
    else if (key === "versions java") data = { major: 25 };
    else if (key === "config list")
      data =
        request.kind === "orbit"
          ? {
              entries: [
                {
                  key: "auth.modrinth-token",
                  value: "<redacted>",
                  sensitive: true,
                  allowed_values: [],
                },
                { key: "cache.dir", value: null, allowed_values: [] },
                { key: "repository.dir", value: null, allowed_values: [] },
                {
                  key: "ui.progress-bar",
                  value: "modern",
                  allowed_values: ["modern", "plain", "off"],
                },
                {
                  key: "ui.color",
                  value: "auto",
                  allowed_values: ["auto", "always", "never"],
                },
              ],
              resolved_paths: {
                cache_directory: "X:/fixture/cache/jars",
                repository_directory: "X:/fixture/cache/repository",
              },
            }
          : {
              settings: [
                { key: "java.selection", value: "auto", allowed_values: [] },
              ],
            };
    else if (key === "minecraft directory")
      data = { directory: "X:/fixture/minecraft" };
    else throw new Error(`Read-only fixture does not implement ${key}`);
    events.onmessage({ type: "finished", ok: true, data });
    return ++id;
  }
  throw new Error(`Read-only fixture does not implement ${cmd}`);
});
if (new URLSearchParams(location.search).has("activity")) {
  for (let index = 1; index <= 60; index++)
    activity.tasks.push({
      id: -index,
      label: `测试活动 ${index}`,
      state: "succeeded",
      message: "",
    });
  activity.open = true;
}
await import("../src/main");
