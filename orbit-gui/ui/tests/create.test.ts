import { beforeEach, expect, it, vi } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import { createRouter, createMemoryHistory } from "vue-router";
import { shallowRef } from "vue";
import {
  instanceContextKey,
  type InstanceContext,
} from "../src/services/instance-context";
vi.mock("@tauri-apps/api/path", () => ({ join: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("../src/components/DownloadSourcePicker.vue", () => ({
  default: {
    name: "DownloadSourcePicker",
    emits: ["changed", "pending"],
    template: "<div />",
  },
}));
vi.mock("../src/services/installations", () => ({
  createInstallation: vi.fn(),
  prepareMigration: vi.fn(),
  applyMigration: vi.fn(),
  updateLoader: vi.fn(),
}));
vi.mock("../src/services/workspace", () => ({
  launcher: vi.fn(),
  refresh: vi.fn(),
  act: (fn: () => unknown) => fn(),
}));
import Create from "../src/pages/Create.vue";
import DropdownSelect from "../src/components/upstream/DropdownSelect.vue";
import { launcher } from "../src/services/workspace";
const catalog = {
  versions: [
    { id: "26.2", version_type: "release" },
    { id: "26.3-snapshot", version_type: "snapshot" },
  ],
};
beforeEach(() => {
  vi.mocked(launcher).mockReset();
});
async function page(start = true, path = "/create", context?: InstanceContext) {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/create", component: Create },
      { path: "/instance/:id/create", component: Create },
    ],
  });
  await router.push(path);
  await router.isReady();
  const wrapper = mount(Create, {
    global: {
      plugins: [router],
      provide: context
        ? { [instanceContextKey as symbol]: shallowRef(context) }
        : {},
    },
  });
  if (start) await wrapper.find(".setup-options button").trigger("click");
  await flushPromises();
  return wrapper;
}
it("migration without a source route context cannot fall back to any instance", async () => {
  const wrapper = await page(false, "/create?migrate=1");
  expect(wrapper.text()).toContain("Open a source instance from the library");
  expect(launcher).not.toHaveBeenCalled();
  expect(
    wrapper.findAll(".wizard-footer button").at(-1)!.attributes("disabled"),
  ).toBeDefined();
  wrapper.unmount();
});
it("loader updates show the exact source supplied by the instance route", async () => {
  vi.mocked(launcher).mockImplementation(async (args) =>
    args[1] === "loader"
      ? { versions: [{ version: "0.20.1", stable: true }] }
      : { major: 25 },
  );
  const context = {
    state: {
      instance: { id: "source", name: "Creative Source" },
      detail: { desired: { minecraft: "26.2", loader: "fabric" } },
      inventoryError: "",
    },
  } as unknown as InstanceContext;
  const wrapper = await page(
    false,
    "/instance/source/create?loader=1",
    context,
  );
  expect(wrapper.find(".source-context").text()).toContain("Creative Source");
  expect(
    vi.mocked(launcher).mock.calls.every((call) => call[0].includes("26.2")),
  ).toBe(true);
  wrapper.unmount();
});
it("uses one control for Minecraft and Loader, and blocks continuing until metadata has loaded", async () => {
  let loaders!: (result: unknown) => void, java!: (result: unknown) => void;
  vi.mocked(launcher).mockImplementation(async (args) =>
    args[1] === "minecraft"
      ? catalog
      : new Promise((resolve) => {
          if (args[1] === "loader") loaders = resolve;
          else java = resolve;
        }),
  );
  const wrapper = await page();
  wrapper.findComponent(DropdownSelect).vm.$emit("update:modelValue", "26.2");
  await flushPromises();
  await wrapper.findAll(".wizard-footer button").at(-1)!.trigger("click");
  expect(wrapper.findComponent(DropdownSelect).props("loading")).toBe(true);
  expect(
    wrapper.findAll(".wizard-footer button").at(-1)!.attributes("disabled"),
  ).toBeDefined();
  loaders({ versions: [{ version: "0.20.1", stable: true }] });
  java({ major: 25 });
  await flushPromises();
  expect(wrapper.findComponent(DropdownSelect).props("modelValue")).toBe(
    "0.20.1",
  );
  expect(wrapper.findComponent(DropdownSelect).props("loading")).toBe(false);
  expect(
    wrapper.findAll(".wizard-footer button").at(-1)!.attributes("disabled"),
  ).toBeUndefined();
  expect(wrapper.find("select").exists()).toBe(false);
  wrapper.unmount();
});
it("does not expose source or query components before the user starts the wizard", async () => {
  vi.mocked(launcher).mockResolvedValue(catalog);
  const wrapper = await page(false);
  expect(wrapper.findComponent({ name: "DownloadSourcePicker" }).exists()).toBe(
    false,
  );
  expect(launcher).not.toHaveBeenCalled();
  await wrapper.find(".setup-options button").trigger("click");
  await flushPromises();
  wrapper.findComponent({ name: "DownloadSourcePicker" }).vm.$emit("changed");
  await flushPromises();
  expect(vi.mocked(launcher).mock.calls.map((call) => call[0])).toEqual([
    ["versions", "minecraft"],
    ["versions", "minecraft"],
  ]);
  wrapper.unmount();
});
it("source changes refresh components, preserve explicit loader selection and block installation until ready", async () => {
  const initial = {
    versions: [
      { version: "0.20.1", stable: true },
      { version: "0.19.5", stable: false },
    ],
  };
  vi.mocked(launcher).mockImplementation(async (args) =>
    args[1] === "minecraft"
      ? catalog
      : args[1] === "loader"
        ? initial
        : { major: 25 },
  );
  const wrapper = await page();
  wrapper.findComponent(DropdownSelect).vm.$emit("update:modelValue", "26.2");
  await flushPromises();
  await wrapper.findAll(".wizard-footer button").at(-1)!.trigger("click");
  await flushPromises();
  wrapper.findComponent(DropdownSelect).vm.$emit("update:modelValue", "0.19.5");
  await flushPromises();
  let finish!: (result: unknown) => void;
  vi.mocked(launcher).mockImplementation(async (args) =>
    args[1] === "loader"
      ? new Promise((resolve) => {
          finish = resolve;
        })
      : { major: 25 },
  );
  wrapper.findComponent({ name: "DownloadSourcePicker" }).vm.$emit("changed");
  await flushPromises();
  expect(
    wrapper.findAll(".wizard-footer button").at(-1)!.attributes("disabled"),
  ).toBeDefined();
  finish(initial);
  await flushPromises();
  expect(wrapper.findComponent(DropdownSelect).props("modelValue")).toBe(
    "0.19.5",
  );
  expect(
    wrapper.findAll(".wizard-footer button").at(-1)!.attributes("disabled"),
  ).toBeUndefined();
  expect(
    vi.mocked(launcher).mock.calls.every((call) => !call[0].includes("")),
  ).toBe(true);
  wrapper.unmount();
});
it("ignores an older Minecraft catalog response after switching download source", async () => {
  let finishOld!: (result: unknown) => void;
  vi.mocked(launcher).mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finishOld = resolve;
      }),
  );
  const wrapper = await page();
  vi.mocked(launcher).mockResolvedValue(catalog);
  wrapper.findComponent({ name: "DownloadSourcePicker" }).vm.$emit("changed");
  await flushPromises();
  wrapper.findComponent(DropdownSelect).vm.$emit("update:modelValue", "26.2");
  finishOld({ versions: [{ id: "1.20.1", version_type: "release" }] });
  await flushPromises();
  expect(wrapper.findComponent(DropdownSelect).props("modelValue")).toBe(
    "26.2",
  );
  expect(wrapper.findComponent(DropdownSelect).props("options")).toEqual(
    expect.arrayContaining([expect.objectContaining({ value: "26.2" })]),
  );
  wrapper.unmount();
});
it("blocks continuation after a source refresh failure and retries even with an older cached catalog", async () => {
  vi.mocked(launcher).mockResolvedValue(catalog);
  const wrapper = await page();
  wrapper.findComponent(DropdownSelect).vm.$emit("update:modelValue", "26.2");
  await flushPromises();
  wrapper
    .findComponent({ name: "DownloadSourcePicker" })
    .vm.$emit("pending", true);
  await flushPromises();
  expect(
    wrapper.findAll(".wizard-footer button").at(-1)!.attributes("disabled"),
  ).toBeDefined();
  wrapper
    .findComponent({ name: "DownloadSourcePicker" })
    .vm.$emit("pending", false);
  vi.mocked(launcher).mockRejectedValueOnce(new Error("mirror unavailable"));
  wrapper.findComponent({ name: "DownloadSourcePicker" }).vm.$emit("changed");
  await flushPromises();
  expect(
    wrapper.findAll(".wizard-footer button").at(-1)!.attributes("disabled"),
  ).toBeDefined();
  const retry = wrapper
    .findAll("button")
    .find((button) => button.text() === "Retry")!;
  expect(retry).toBeDefined();
  await retry.trigger("click");
  await flushPromises();
  expect(launcher).toHaveBeenCalledTimes(3);
  expect(
    wrapper.findAll(".wizard-footer button").at(-1)!.attributes("disabled"),
  ).toBeUndefined();
  wrapper.unmount();
});
it("changing the release filter cannot leave a hidden snapshot selected", async () => {
  vi.mocked(launcher).mockResolvedValue(catalog);
  const wrapper = await page();
  await wrapper.findAll(".tabs button")[1].trigger("click");
  wrapper
    .findComponent(DropdownSelect)
    .vm.$emit("update:modelValue", "26.3-snapshot");
  await wrapper.findAll(".tabs button")[0].trigger("click");
  expect(wrapper.findComponent(DropdownSelect).props("modelValue")).toBe("");
  expect(
    wrapper.findAll(".wizard-footer button").at(-1)!.attributes("disabled"),
  ).toBeDefined();
  wrapper.unmount();
});
