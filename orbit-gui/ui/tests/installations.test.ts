import { beforeEach, expect, it, vi } from "vitest";
const calls = vi.hoisted(() => ({ orbit: vi.fn(), launcher: vi.fn() }));
vi.mock("@tauri-apps/api/path", () => ({
  join: async (...parts: string[]) => parts.join("/"),
  tempDir: async () => "/tmp",
}));
vi.mock("../src/services/workspace", () => ({
  ...calls,
  refresh: vi.fn(),
  selectInstance: vi.fn(),
  workspace: {},
}));
import {
  installArguments,
  prepareMigration,
  applyMigration,
  updateLoader,
} from "../src/services/installations";
import { policyArguments } from "../src/services/commands";
import { workspace } from "../src/services/workspace";
const draft = {
  name: "Target",
  kind: "client" as const,
  minecraft: "26.2",
  loader: "fabric",
  loaderVersion: "0.20.0",
  serverDirectory: "",
};
const source = {
  id: "source",
  name: "Source",
  directory: "/minecraft/instances/Source",
  kind: "client",
  is_default: false,
};
beforeEach(() => {
  calls.orbit.mockReset();
  calls.launcher.mockReset();
});
it("never creates a target if source export fails", async () => {
  calls.orbit.mockRejectedValue(new Error("export cancelled"));
  await expect(prepareMigration(draft, source)).rejects.toThrow(
    "export cancelled",
  );
  expect(calls.launcher).not.toHaveBeenCalled();
});
it("composes both source projections before installing the fresh target", async () => {
  const order: string[] = [];
  calls.orbit.mockImplementation(async (args: string[]) => {
    order.push("orbit " + args[0]);
    return { summary: { removals: 0 }, changes: [], warnings: [] };
  });
  calls.launcher.mockImplementation(async (args: string[]) => {
    order.push("launcher " + args[0]);
    return args[0] === "install"
      ? { instance_id: "target" }
      : {
          id: "target",
          name: "Target",
          directory: "/target",
          desired: { minecraft: "26.2", loader: "fabric" },
        };
  });
  await prepareMigration(draft, source);
  expect(order).toEqual([
    "orbit export",
    "launcher export",
    "launcher install",
    "launcher instance",
  ]);
  expect(calls.launcher.mock.calls[1][0]).toContain("--from");
});
it("selects and confirms migration in a single export command, without a preceding check", async () => {
  calls.launcher
    .mockResolvedValueOnce({ instance_id: "target" })
    .mockResolvedValue({
      id: "target",
      name: "Target",
      directory: "/target",
    });
  const review = await prepareMigration(draft, source);
  calls.orbit.mockResolvedValue({ export: { applied: true } });
  expect(await applyMigration(review)).toBe(true);
  const migrations = calls.orbit.mock.calls.filter(
    ([args]) => args[0] === "migrate",
  );
  expect(migrations).toHaveLength(1);
  expect(migrations[0][0]).toEqual([
    "migrate",
    "export",
    "/target",
    "--source-pack",
    review.sourcePack,
    "--consume-source-pack",
  ]);
  expect(review.exported).toBe(true);
});
it("cancellation never registers or installs packages, and can retry against the prepared target", async () => {
  const review = {
    target: { ...source, desired: { minecraft: "26.2", loader: "fabric" } },
    sourcePack: "/source.orbitbundle",
    exported: false,
    registered: false,
  };
  calls.orbit.mockRejectedValueOnce(new Error("cancelled"));
  await expect(applyMigration(review)).rejects.toThrow("cancelled");
  expect(review.exported).toBe(false);
  expect(calls.orbit).toHaveBeenCalledTimes(1);
  calls.orbit.mockResolvedValue({ export: { applied: true } });
  expect(await applyMigration(review)).toBe(true);
  expect(calls.launcher).not.toHaveBeenCalled();
});
it("retrying a failed package install reuses the committed plan, not the consumed source snapshot", async () => {
  const review = {
    target: { ...source, desired: { minecraft: "26.2", loader: "fabric" } },
    sourcePack: "/source.orbitbundle",
    exported: false,
    registered: false,
  };
  calls.orbit
    .mockResolvedValueOnce({ export: { applied: true } })
    .mockResolvedValueOnce({})
    .mockRejectedValueOnce(new Error("download failed"));
  await expect(applyMigration(review)).rejects.toThrow("download failed");
  calls.orbit.mockResolvedValue({});
  expect(await applyMigration(review)).toBe(true);
  expect(calls.orbit.mock.calls.map(([args]) => args.slice(0, 2))).toEqual([
    ["migrate", "export"],
    ["instances", "register"],
    ["install"],
    ["install"],
  ]);
});
it("keeps client and server installation locations distinct", () => {
  expect(installArguments(draft)).not.toContain("--server-directory");
  expect(
    installArguments({ ...draft, kind: "server", serverDirectory: "/server" }),
  ).toContain("/server");
});
it("keeps a loader update on its source if the selection changes", async () => {
  calls.launcher.mockImplementation(async () => {
    workspace.selected = { ...source, id: "other" };
  });
  await updateLoader("0.20.0", source, true);
  expect(calls.launcher).toHaveBeenCalledTimes(2);
  for (const call of calls.launcher.mock.calls) expect(call[2]).toEqual(source);
  expect(calls.orbit.mock.calls[0][0]).toEqual(["sync"]);
  expect(calls.orbit.mock.calls[0][2]).toEqual(source);
});
it("passes inclusive and exclusive bounds without interpreting versions", () => {
  expect(
    policyArguments({
      kind: "range",
      lower: "v1",
      upper: "v2",
      include_lower: true,
      include_upper: false,
    }),
  ).toEqual([
    "range",
    "v1",
    "v2",
    "--lower-bound",
    "inclusive",
    "--upper-bound",
    "exclusive",
  ]);
});
