import { beforeEach, expect, it, vi } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import { reactive } from "vue";
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (p: string) => p,
}));
vi.mock("../src/services/workspace", async () => {
  const { reactive } = await import("vue");
  return {
    workspace: reactive({ selected: { id: "instance" }, packages: [] }),
    mutate: vi.fn(),
    orbit: vi.fn(),
    act: (f: () => unknown) => f(),
  };
});
import PackageManager from "../src/components/PackageManager.vue";
import { mutate, orbit } from "../src/services/workspace";
import type { Package } from "../src/types";
import type { Task } from "../src/services/cli";
const pkg = {
  mod_id: "sodium",
  version: "0.9.1",
  enabled: true,
  remotes: [],
  dependencies: [],
  bundled_count: 0,
} as Package;
beforeEach(() => {
  vi.mocked(mutate).mockReset();
  vi.mocked(orbit).mockReset();
  HTMLDialogElement.prototype.showModal = function () {
    this.open = true;
  };
});
it("opens without fetching a version catalog and files only request ownership", async () => {
  const wrapper = mount(PackageManager, {
    props: { pkg },
    global: { stubs: { Teleport: true } },
  });
  expect(orbit).not.toHaveBeenCalled();
  vi.mocked(orbit).mockResolvedValue({
    mod_id: "sodium",
    artifacts: [],
    data: [],
  });
  await wrapper.findAll(".management-actions button")[2].trigger("click");
  await flushPromises();
  expect(orbit).toHaveBeenCalledTimes(1);
  expect(vi.mocked(orbit).mock.calls[0][0]).toEqual(["ownership", "sodium"]);
  await wrapper.find("footer button").trigger("click");
  expect(wrapper.emitted("close")).toHaveLength(1);
  wrapper.unmount();
});
it("passes a single upgrade target and renders its live progress inside the modal", async () => {
  let finish!: () => void;
  const task = reactive<Task>({
    id: 1,
    label: "Upgrade Sodium",
    state: "running",
    message: "Resolving",
    completed: 3,
    total: 8,
  });
  vi.mocked(mutate).mockImplementation((_args, _label, _instance, observe) => {
    observe?.(task);
    return new Promise<void>((resolve) => {
      finish = resolve;
    });
  });
  const wrapper = mount(PackageManager, {
    props: { pkg },
    global: { stubs: { Teleport: true } },
  });
  await wrapper.find(".management-actions button").trigger("click");
  expect(mutate).toHaveBeenCalledWith(
    ["upgrade", "sodium"],
    expect.any(String),
    { id: "instance" },
    expect.any(Function),
  );
  expect(
    wrapper.find("dialog .task-progress progress").attributes("value"),
  ).toBe("3");
  task.completed = 6;
  task.message = "Applying";
  await flushPromises();
  expect(wrapper.find("dialog .task-progress").text()).toContain("Applying");
  expect(
    wrapper.find("dialog .task-progress progress").attributes("value"),
  ).toBe("6");
  expect(
    wrapper.find(".package-controls").attributes("disabled"),
  ).toBeDefined();
  task.state = "succeeded";
  finish();
  await flushPromises();
  wrapper.unmount();
});
it("shows ownership loading and results without waiting for game startup", async () => {
  const task = reactive<Task>({
    id: 2,
    label: "Files",
    state: "running",
    message: "Reading ledger",
  });
  let finish!: (data: unknown) => void;
  vi.mocked(orbit).mockImplementation((_args, _label, _instance, observe) => {
    observe?.(task);
    return new Promise((resolve) => {
      finish = resolve;
    });
  });
  const wrapper = mount(PackageManager, {
    props: { pkg },
    global: { stubs: { Teleport: true } },
  });
  await wrapper.findAll(".management-actions button")[2].trigger("click");
  expect(wrapper.find("dialog .task-progress").text()).toContain(
    "Reading ledger",
  );
  task.state = "succeeded";
  finish({
    artifacts: [],
    data: [{ path: "config/sodium.json", kind: "file", preserved: [] }],
  });
  await flushPromises();
  expect(wrapper.text()).toContain("config/sodium.json");
  wrapper.unmount();
});
