// Browser-only visual fixture. Does not invoke a CLI, open user files, or ship in dist.
import { mockIPC } from "@tauri-apps/api/mocks";
import type { Channel } from "@tauri-apps/api/core";
import type { Request } from "../src/services/cli";
import { discoverySession } from "../src/services/discovery";
const instances = [
  {
    id: "demo",
    name: "Creative · Fabric",
    directory: "X:/fixture/creative",
    kind: "client",
    is_default: true,
  },
];
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
}));
Object.assign(discoverySession("demo"), {
  query: "sodium",
  results,
  searched: true,
});
const prefs = {
  orbit_binary: "fixture",
  launcher_binary: "fixture",
  selected_instance: "demo",
  language: "zh-CN",
  theme_mode: new URL(location.href).searchParams.get("theme") || "light",
  accent_theme: "indigo",
};
let id = 0;
mockIPC((cmd, args) => {
  if (cmd === "preferences") return prefs;
  if (cmd === "save_preferences") return;
  if (cmd === "run_cli") {
    const { request, events } = args as {
      request: Request;
      events: Channel<unknown>;
    };
    const key = request.args.slice(0, 2).join(" ");
    let data: unknown;
    if (key === "instance list") data = { instances };
    else if (key === "instance show") data = detail;
    else if (key === "account list")
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
    else throw new Error(`Read-only fixture does not implement ${key}`);
    events.onmessage({ type: "finished", ok: true, data });
    return ++id;
  }
  throw new Error(`Read-only fixture does not implement ${cmd}`);
});
await import("../src/main");
