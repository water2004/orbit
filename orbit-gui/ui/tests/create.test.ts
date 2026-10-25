import { beforeEach, expect, it, vi } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import { createRouter, createMemoryHistory } from "vue-router";
vi.mock("@tauri-apps/api/path", () => ({ join: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("../src/components/DownloadSourcePicker.vue", () => ({
  default: { template: "<div />" },
}));
vi.mock("../src/services/installations", () => ({
  createInstallation: vi.fn(),
  prepareMigration: vi.fn(),
  applyMigration: vi.fn(),
  updateLoader: vi.fn(),
}));
vi.mock("../src/services/workspace", () => ({
  workspace: { selected: null },
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
async function page() {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: "/create", component: Create }],
  });
  await router.push("/create");
  await router.isReady();
  const wrapper = mount(Create, { global: { plugins: [router] } });
  await wrapper.find(".setup-options button").trigger("click");
  await flushPromises();
  return wrapper;
}
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
