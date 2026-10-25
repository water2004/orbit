import { beforeEach, expect, it, vi } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
const calls = vi.hoisted(() => ({
  launcher: vi.fn(),
  refresh: vi.fn(),
  selectInstance: vi.fn(),
  open: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: calls.open }));
vi.mock("../src/services/task-scope", async () => {
  const { ref } = await import("vue");
  return { useCommandScope: () => ({ launcher: calls.launcher, task: ref() }) };
});
vi.mock("../src/services/workspace", () => ({
  refresh: calls.refresh,
  selectInstance: calls.selectInstance,
  act: (f: () => unknown) => f(),
}));
import ImportInstance from "../src/components/ImportInstance.vue";
import DropdownSelect from "../src/components/upstream/DropdownSelect.vue";
beforeEach(() => {
  vi.clearAllMocks();
  HTMLDialogElement.prototype.showModal = function () {
    this.open = true;
  };
});
const source = (id: string) => ({
  id,
  name: id,
  format: "pcl",
  kind: "client",
  game_directory: `X:/source/versions/${id}`,
  runtime: { minecraft: "26.2", loader: "fabric", loader_version: "0.20.1" },
  warnings: [],
  directory_evidence: "PCL version isolation",
});
it("requires a source choice and reviews the same CLI import before executing it", async () => {
  calls.open.mockResolvedValue("X:/source");
  calls.launcher
    .mockResolvedValueOnce({ instances: [source("One"), source("Two")] })
    .mockResolvedValueOnce({
      game_directory: "X:/source/versions/Two",
      destination: "X:/orbit/instances/Two",
      files: 3,
      bytes: 1024,
      excluded: ["PCL"],
      resuming: false,
    })
    .mockResolvedValueOnce({ instance: { id: "imported" } })
    .mockResolvedValueOnce({ id: "imported", name: "Two" });
  const w = mount(ImportInstance, { global: { stubs: { Teleport: true } } });
  await w.find(".row button").trigger("click");
  await flushPromises();
  expect(w.findComponent(DropdownSelect).props("modelValue")).toBe("");
  expect(
    w.findAll("footer button").at(-1)!.attributes("disabled"),
  ).toBeDefined();
  w.findComponent(DropdownSelect).vm.$emit("update:modelValue", "Two");
  await flushPromises();
  await w.findAll("footer button").at(-1)!.trigger("click");
  await flushPromises();
  const args = [
    "instance",
    "import",
    "--directory",
    "X:/source",
    "--source",
    "Two",
    "--name",
    "Two",
    "--game-directory",
    "X:/source/versions/Two",
  ];
  expect(calls.launcher.mock.calls[1][0]).toEqual([...args, "--dry-run"]);
  expect(calls.launcher).toHaveBeenCalledTimes(2);
  await w.findAll("footer button").at(-1)!.trigger("click");
  await flushPromises();
  expect(calls.launcher.mock.calls[2][0]).toEqual(args);
  expect(calls.selectInstance).toHaveBeenCalledWith({
    id: "imported",
    name: "Two",
  });
  expect(w.emitted("imported")).toEqual([["imported"]]);
  w.unmount();
});
it("does not guess a missing game directory or keep a preview after name edits", async () => {
  calls.open.mockResolvedValue("X:/source");
  calls.launcher
    .mockResolvedValueOnce({
      instances: [{ ...source("One"), game_directory: null }],
    })
    .mockResolvedValueOnce({
      game_directory: "X:/chosen",
      destination: "X:/orbit/instances/One",
      files: 0,
      bytes: 0,
      excluded: [],
      resuming: false,
    });
  const w = mount(ImportInstance, { global: { stubs: { Teleport: true } } });
  await w.find(".row button").trigger("click");
  await flushPromises();
  expect(
    w.findAll("footer button").at(-1)!.attributes("disabled"),
  ).toBeDefined();
  calls.open.mockResolvedValueOnce("X:/chosen");
  await w.findAll(".row button").at(-1)!.trigger("click");
  await flushPromises();
  await w.findAll("footer button").at(-1)!.trigger("click");
  await flushPromises();
  expect(w.text()).toContain("X:/orbit/instances/One");
  await w.find("input").setValue("Another");
  expect(w.text()).not.toContain("X:/orbit/instances/One");
  expect(calls.launcher).toHaveBeenCalledTimes(2);
  w.unmount();
});
