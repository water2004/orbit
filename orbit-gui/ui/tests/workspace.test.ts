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
  selectInstance,
} from "../src/services/workspace";
import { run, loadPreferences } from "../src/services/cli";
import type { Instance, Detail } from "../src/types";
const instances = ["A", "B"].map((id) => ({
  id,
  name: id,
  directory: id,
  kind: "client",
  is_default: id === "A",
})) as Instance[];
beforeEach(() => {
  workspace.selected = null;
  workspace.instances = [];
  workspace.preferences = null;
  workspace.detail = null;
  workspace.inventoryError = "";
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
it("a direct instance route takes priority over the previously selected default", async () => {
  await initialize("B");
  expect(workspace.selected?.id).toBe("B");
  expect(workspace.detail?.id).toBe("B");
  expect(
    vi
      .mocked(run)
      .mock.calls.filter(([r]) => r.args.join(" ") === "instance show")
      .map(([r]) => r.instance),
  ).toEqual(["B"]);
});
it("refresh keeps the current context rather than switching to the default", async () => {
  workspace.selected = instances[1];
  await refresh();
  expect(workspace.selected?.id).toBe("B");
});

it("viewing vanilla does not invoke Orbit or report a missing mod workspace", async () => {
  workspace.inventoryError = "old inventory error";
  vi.mocked(run).mockResolvedValue({
    ...instances[0],
    desired: { minecraft: "26.2", loader: "vanilla" },
  });
  await selectInstance(instances[0]);
  expect(workspace.inventoryError).toBe("");
  expect(workspace.packages).toEqual([]);
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
    workspace.detail = { desired: { loader: "fabric" } } as Detail;
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
