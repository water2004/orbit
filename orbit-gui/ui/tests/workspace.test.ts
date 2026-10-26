import { beforeEach, expect, it, vi } from "vitest";
vi.mock("../src/services/cli", () => ({
  run: vi.fn(),
  loadPreferences: vi.fn(),
  savePreferences: vi.fn(),
  Cancelled: class extends Error {},
}));
import {
  initialize,
  refresh,
  workspace,
  launch,
  orbit,
  launcher,
} from "../src/services/workspace";
import { createInstanceContext } from "../src/services/instance-context";
import { run, loadPreferences } from "../src/services/cli";
import type { Instance } from "../src/types";
const instances = ["A", "B"].map((id) => ({
  id,
  name: id,
  directory: id,
  kind: "client",
  is_default: id === "A",
})) as Instance[];
beforeEach(() => {
  workspace.instances = [];
  workspace.preferences = null;
  workspace.error = "";
  vi.mocked(loadPreferences).mockResolvedValue({
    selected_instance: "A",
    language: "en",
  } as never);
  vi.mocked(run).mockReset();
  vi.mocked(run).mockImplementation(async (request) => {
    if (request.args.join(" ") === "instance list") return { instances };
    if (request.args[0] === "account") return { accounts: [] };
    if (request.args[0] === "list") return { packages: [] };
    return {
      id: request.instance,
      kind: "client",
      desired: { minecraft: "26.2", loader: "fabric" },
    };
  });
});
it("startup loads global inventory without opening a persisted, default or first instance", async () => {
  await initialize();
  expect(workspace.instances).toEqual(instances);
  expect(vi.mocked(run).mock.calls.map(([r]) => r.args)).toEqual([
    ["instance", "list"],
    ["account", "list"],
  ]);
  expect("selected" in workspace).toBe(false);
});
it("refresh reloads only the inventory", async () => {
  await refresh();
  expect(vi.mocked(run).mock.calls.map(([r]) => r.args)).toEqual([
    ["instance", "list"],
    ["account", "list"],
  ]);
});
it("an older inventory request cannot replace the refreshed account/instance summaries", async () => {
  let finish!: (data: unknown) => void;
  vi.mocked(run).mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const pending = refresh();
  await refresh();
  finish({ instances: [{ ...instances[0], name: "Stale" }] });
  await pending;
  expect(workspace.instances).toEqual(instances);
});

it("viewing vanilla does not invoke Orbit or report a missing mod workspace", async () => {
  vi.mocked(run).mockResolvedValue({
    ...instances[0],
    desired: { minecraft: "26.2", loader: "vanilla" },
  });
  const context = createInstanceContext(instances[0]);
  await context.refresh();
  expect(context.state.inventoryError).toBe("");
  expect(context.state.packages).toEqual([]);
  expect(
    vi.mocked(run).mock.calls.every(([request]) => request.kind === "launcher"),
  ).toBe(true);
});

it.each(["client", "server"])(
  "launches a vanilla %s through Launcher without Orbit",
  async (kind) => {
    workspace.preferences = {
      launcher_binary: "launcher",
      orbit_binary: "",
    } as never;
    const target = { ...instances[1], loader: "fabric", kind };
    vi.mocked(run)
      .mockResolvedValueOnce({ ...target, desired: { loader: "vanilla" } })
      .mockResolvedValueOnce({});
    await launch(target);
    expect(
      vi
        .mocked(run)
        .mock.calls.map(([request]) => [
          request.kind,
          request.args,
          request.instance,
        ]),
    ).toEqual([
      ["launcher", ["instance", "show"], target.id],
      [
        "launcher",
        kind === "server" ? ["server", "start"] : ["launch"],
        target.id,
      ],
    ]);
  },
);

it("instance views isolate late responses and mutations keep their captured target", async () => {
  let finish!: (data: unknown) => void;
  vi.mocked(run).mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const a = createInstanceContext(instances[0]);
  const b = createInstanceContext(instances[1]);
  const pending = a.refresh();
  a.dispose();
  await b.refresh();
  finish({ ...instances[0], desired: { loader: "fabric" } });
  await pending;
  expect(a.state.detail).toBeNull();
  expect(b.state.detail?.id).toBe("B");
  await a.mutate(["upgrade", "sodium"], "Upgrade A");
  expect(vi.mocked(run).mock.lastCall?.[0]).toMatchObject({
    directory: "A",
    args: ["upgrade", "sodium"],
  });
});

it("rejects mismatched CLI instance metadata instead of retargeting the view", async () => {
  const context = createInstanceContext(instances[0]);
  vi.mocked(run).mockResolvedValueOnce({
    id: "B",
    desired: { loader: "fabric" },
  });
  await expect(context.refresh()).rejects.toThrow("different instance");
  expect(context.state.detail).toBeNull();
  expect(run).toHaveBeenCalledTimes(1);
});

it("global commands have no implicit target after opening an instance", async () => {
  await createInstanceContext(instances[0]).refresh();
  await launcher(["account", "list"], "Accounts", null);
  await orbit(["config", "list"], "Configuration", null);
  expect(
    vi
      .mocked(run)
      .mock.calls.slice(-2)
      .map(([request]) => [request.instance, request.directory]),
  ).toEqual([
    [undefined, undefined],
    [undefined, undefined],
  ]);
});

it.each(["client", "server"])(
  "preserves Orbit observation for a modded %s",
  async (kind) => {
    workspace.preferences = {
      launcher_binary: "launcher",
      orbit_binary: "orbit",
    } as never;
    const target = { ...instances[1], kind };
    vi.mocked(run)
      .mockResolvedValueOnce({ ...target, desired: { loader: "fabric" } })
      .mockResolvedValueOnce({});
    await launch(target);
    expect(vi.mocked(run).mock.calls[1][0]).toMatchObject({
      lifetime: "game",
      kind: "orbit",
      directory: target.directory,
      args: [
        "launch",
        "--launcher",
        "launcher",
        "--launcher-instance",
        target.id,
        ...(kind === "server" ? ["--server"] : []),
      ],
    });
  },
);

it("does not bypass a failed observed launch with a direct Launcher retry", async () => {
  workspace.preferences = {
    launcher_binary: "launcher",
    orbit_binary: "orbit",
  } as never;
  vi.mocked(run)
    .mockResolvedValueOnce({ ...instances[0], desired: { loader: "fabric" } })
    .mockRejectedValueOnce(new Error("observation failed"));
  await expect(launch(instances[0])).rejects.toThrow("observation failed");
  expect(run).toHaveBeenCalledTimes(2);
});

it("fails before launching if the exact target metadata cannot be read", async () => {
  workspace.preferences = { launcher_binary: "launcher" } as never;
  vi.mocked(run).mockRejectedValue(new Error("invalid instance"));
  await expect(launch(instances[0])).rejects.toThrow("invalid instance");
  expect(run).toHaveBeenCalledTimes(1);
});
