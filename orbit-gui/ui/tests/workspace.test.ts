import { beforeEach, expect, it, vi } from "vitest";
vi.mock("../src/services/cli", () => ({
  run: vi.fn(),
  loadPreferences: vi.fn(),
  savePreferences: vi.fn(),
  Cancelled: class extends Error {},
}));
import { initialize, refresh, workspace } from "../src/services/workspace";
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
  workspace.selected = null;
  workspace.instances = [];
  workspace.preferences = null;
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
