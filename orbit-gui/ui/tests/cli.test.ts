import { beforeEach, describe, expect, it, vi } from "vitest";
const ipc = vi.hoisted(() => ({
  invoke: vi.fn(),
  channels: [] as { onmessage: (event: unknown) => void }[],
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: ipc.invoke,
  convertFileSrc: (p: string) => p,
  Channel: class {
    onmessage = (_event: unknown) => {};
    constructor() {
      ipc.channels.push(this);
    }
  },
}));
import { activity, answer, cancel, Cancelled, run } from "../src/services/cli";

beforeEach(() => {
  activity.tasks.splice(0);
  ipc.channels.splice(0);
  ipc.invoke.mockReset();
  ipc.invoke.mockResolvedValue(12);
});
describe("CLI channel lifecycle", () => {
  it("waits for the real process ID when cancelled during spawn", async () => {
    let spawned!: (id: number) => void;
    ipc.invoke.mockImplementationOnce(
      () =>
        new Promise<number>((resolve) => {
          spawned = resolve;
        }),
    );
    const result = run({ kind: "orbit", args: ["upgrade"], label: "Upgrade" });
    const cancellation = cancel(activity.tasks[0]);
    expect(ipc.invoke).toHaveBeenCalledTimes(1);
    spawned(42);
    await cancellation;
    expect(ipc.invoke).toHaveBeenLastCalledWith("cancel_cli", { task: 42 });
    ipc.channels[0].onmessage({ type: "finished", ok: false, cancelled: true });
    await expect(result).rejects.toBeInstanceOf(Cancelled);
  });
  it("submits a choice only once while waiting for the IPC response", async () => {
    const result = run({ kind: "orbit", args: ["fix"], label: "Fix" });
    await Promise.resolve();
    ipc.channels[0].onmessage({
      type: "interaction",
      envelope: {
        interaction_id: "factor",
        interaction: "resolution",
        choices: [],
        prompt: "Choose",
        allow_cancel: true,
      },
    });
    await Promise.all([
      answer(activity.tasks[0], "one"),
      answer(activity.tasks[0], "two"),
    ]);
    expect(
      ipc.invoke.mock.calls.filter((call) => call[0] === "answer_cli"),
    ).toHaveLength(1);
    ipc.channels[0].onmessage({ type: "finished", ok: true, data: {} });
    await result;
  });
  it("treats a CLI cancellation as cancellation instead of a generic failure", async () => {
    const result = run({ kind: "orbit", args: ["fix"], label: "Fix" });
    ipc.channels[0].onmessage({
      type: "error",
      code: "cancelled",
      message: "Cancelled by user",
    });
    ipc.channels[0].onmessage({ type: "finished", ok: false, status: 1 });
    await expect(result).rejects.toBeInstanceOf(Cancelled);
    expect(activity.tasks[0].state).toBe("cancelled");
  });
  it("receives an immediate result before the spawn IPC resolves", async () => {
    ipc.invoke.mockImplementation(async () => {
      ipc.channels[0].onmessage({
        type: "finished",
        ok: true,
        data: { packages: [] },
      });
      return 12;
    });
    await expect(
      run({ kind: "orbit", args: ["list"], label: "list" }),
    ).resolves.toEqual({ packages: [] });
    expect(activity.tasks[0].state).toBe("succeeded");
  });
  it("never turns a protocol error into success even if exit is zero", async () => {
    const result = run({ kind: "orbit", args: ["list"], label: "list" });
    ipc.channels[0].onmessage({ type: "error", message: "invalid UTF-8" });
    ipc.channels[0].onmessage({ type: "finished", ok: true, data: {} });
    await expect(result).rejects.toThrow("invalid UTF-8");
  });
  it("does not claim upgrade completion when its final report is missing", async () => {
    const result = run({
      kind: "orbit",
      args: ["upgrade", "sodium"],
      label: "Upgrading",
    });
    ipc.channels[0].onmessage({ type: "finished", ok: true, data: {} });
    await expect(result).rejects.toThrow(/invalid upgrade result|升级结果无效/);
    expect(activity.tasks[0].state).toBe("failed");
  });
  it("ends a no-op upgrade with the returned reason instead of its last solver progress", async () => {
    const result = run({
      kind: "orbit",
      args: ["upgrade", "sodium"],
      label: "Upgrading mod",
    });
    ipc.channels[0].onmessage({
      type: "progress",
      message: "Choosing sodium",
      completed: 3,
      total: 10,
    });
    ipc.channels[0].onmessage({
      type: "finished",
      ok: true,
      data: {
        changes: [],
        warnings: [],
        diagnostics: [
          {
            package: "sodium",
            candidate_version: "0.10",
            facts: ["dependency requires another version"],
          },
        ],
      },
    });
    await result;
    const task = activity.tasks[0];
    expect(task.state).toBe("succeeded");
    expect(task.message).toBe("");
    expect(task.total).toBeUndefined();
    expect(task.outcome?.title).toMatch(/No feasible|没有可行/);
    expect(task.outcome?.detailsExpanded).toBe(true);
    expect(task.outcome?.notices[0].lines).toEqual([
      "dependency requires another version",
    ]);
    ipc.channels[0].onmessage({
      type: "progress",
      message: "Late queued progress",
    });
    expect(task.message).toBe("");
    expect(task.state).toBe("succeeded");
  });
  it("summarizes an applied upgrade from the CLI transaction without another version check", async () => {
    const result = run({
      kind: "orbit",
      args: ["upgrade", "sodium"],
      label: "Upgrading mod",
    });
    ipc.channels[0].onmessage({
      type: "finished",
      ok: true,
      data: {
        changes: [{ package: "sodium", kind: "upgrade" }],
        diagnostics: [],
        warnings: [],
      },
    });
    await result;
    expect(activity.tasks[0].outcome?.title).toMatch(
      /Update completed|升级完成/,
    );
    expect(
      ipc.invoke.mock.calls.filter((call) => call[0] === "run_cli"),
    ).toHaveLength(1);
  });
  it("does not erase the next factor question when the previous answer resolves", async () => {
    const result = run({ kind: "orbit", args: ["upgrade"], label: "upgrade" });
    await Promise.resolve();
    const interaction = (id: string) => ({
      type: "interaction",
      envelope: {
        interaction_id: id,
        interaction: "resolution",
        prompt: id,
        choices: [],
        allow_cancel: true,
      },
    });
    ipc.channels[0].onmessage(interaction("factor-1"));
    ipc.invoke.mockImplementation(async () => {
      ipc.channels[0].onmessage(interaction("factor-2"));
    });
    await answer(activity.tasks[0], "choice-1");
    expect(activity.tasks[0].interaction?.interaction_id).toBe("factor-2");
    expect(activity.tasks[0].state).toBe("waiting");
    ipc.channels[0].onmessage({ type: "finished", ok: false, cancelled: true });
    await expect(result).rejects.toThrow();
  });
});
