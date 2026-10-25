import { beforeEach, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { createMemoryHistory, createRouter } from "vue-router";
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (p: string) => p,
}));
vi.mock("../src/services/workspace", async () => {
  const { reactive } = await import("vue");
  return {
    workspace: reactive({
      instances: [],
      packages: [],
      detail: null,
      selected: null,
    }),
    selectInstance: vi.fn(),
    orbit: vi.fn(),
    mutate: vi.fn(),
    act: (fn: () => Promise<unknown>) => fn(),
  };
});
import Browse from "../src/pages/Browse.vue";
import {
  workspace,
  mutate,
  orbit,
  selectInstance,
} from "../src/services/workspace";
import DropdownSelect from "../src/components/upstream/DropdownSelect.vue";
import CatalogSettings from "../src/components/CatalogSettings.vue";
import { discoverySession } from "../src/services/discovery";
import type { Detail, Instance, SearchResult } from "../src/types";
const instance = {
  id: "browse-test",
  name: "Creative",
  directory: "test",
  kind: "client",
} as Instance;
const project = {
  platform: "modrinth",
  project_id: "AANobbMI",
  name: "Sodium",
  description: "Rendering",
  downloads: 100,
  compatible: true,
} as SearchResult;
beforeEach(() => {
  vi.mocked(mutate).mockReset();
  vi.mocked(orbit).mockReset();
  workspace.instances = [instance];
  workspace.selected = instance;
  workspace.packages = [];
  workspace.detail = {
    desired: { minecraft: "26.2", loader: "fabric" },
  } as Detail;
  for (const id of [instance.id])
    Object.assign(discoverySession(id), {
      scope: "",
      query: "sodium",
      results: [project],
      searched: true,
      installing: null,
      error: "",
      recommended: false,
    });
  HTMLDialogElement.prototype.showModal = function () {
    this.open = true;
  };
});
async function page(path: string) {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/browse", component: Browse },
      { path: "/create", component: { template: "<div />" } },
    ],
  });
  await router.push(path);
  await router.isReady();
  const wrapper = mount(Browse, {
    global: {
      plugins: [router],
      stubs: { Teleport: true, CatalogSettings: true },
    },
  });
  return { wrapper, router };
}
it("installs in place without another form, guards double clicks and preserves the query", async () => {
  let finish!: () => void;
  vi.mocked(mutate).mockImplementation(
    () =>
      new Promise<void>((resolve) => {
        finish = resolve;
      }),
  );
  const { wrapper } = await page(`/browse?instance=${instance.id}`);
  await wrapper.find(".project-actions button").trigger("click");
  expect(mutate).toHaveBeenCalledWith(
    ["add", "mr:AANobbMI"],
    expect.any(String),
    instance,
  );
  expect(
    wrapper.find(".project-actions button").attributes("disabled"),
  ).toBeDefined();
  expect(wrapper.find("dialog").exists()).toBe(false);
  finish();
  await flushPromises();
  expect(discoverySession(instance.id).query).toBe("sodium");
  wrapper.unmount();
});
it("the sidebar and instance entry share one page with an inline target selector", async () => {
  const { wrapper, router } = await page("/browse");
  await flushPromises();
  expect(wrapper.find(".target-selector").exists()).toBe(true);
  expect(wrapper.find(".back-link").exists()).toBe(false);
  expect(mutate).not.toHaveBeenCalled();
  expect(router.currentRoute.value.query.instance).toBe(instance.id);
  expect(mutate).not.toHaveBeenCalled();
  wrapper.unmount();
});
it("switches runtime filtering and installation target without another navigation flow", async () => {
  const other = { ...instance, id: "other", name: "Server", kind: "server" };
  workspace.instances.push(other);
  vi.mocked(selectInstance).mockImplementation(async (target) => {
    workspace.selected = target;
    workspace.detail = {
      desired: { minecraft: "26.3", loader: "neoforge" },
    } as Detail;
    workspace.packages = [];
  });
  vi.mocked(orbit).mockResolvedValue({ results: [project] });
  const { wrapper, router } = await page(`/browse?instance=${instance.id}`);
  wrapper.findComponent(DropdownSelect).vm.$emit("update:modelValue", other.id);
  await flushPromises();
  expect(router.currentRoute.value.path).toBe("/browse");
  expect(selectInstance).toHaveBeenCalledWith(other);
  expect(orbit).toHaveBeenLastCalledWith(
    ["search", "sodium", "--mc-version", "26.3", "--modloader", "neoforge"],
    expect.any(String),
    other,
  );
  await wrapper.find(".project-actions button").trigger("click");
  expect(mutate).toHaveBeenLastCalledWith(
    expect.arrayContaining(["add", "mr:AANobbMI"]),
    expect.any(String),
    other,
  );
  wrapper.unmount();
});
it("invalidates displayed compatibility after the instance runtime changes", async () => {
  const { wrapper } = await page(`/browse?instance=${instance.id}`);
  expect(wrapper.findAll(".project-card")).toHaveLength(1);
  workspace.detail = {
    desired: { minecraft: "26.3", loader: "fabric" },
  } as Detail;
  await flushPromises();
  expect(wrapper.findAll(".project-card")).toHaveLength(0);
  expect(discoverySession(instance.id).query).toBe("sodium");
  wrapper.unmount();
});
it("search always passes the exact instance runtime axes", async () => {
  vi.mocked(orbit).mockResolvedValue({ results: [project] });
  const { wrapper } = await page(`/browse?instance=${instance.id}`);
  await wrapper.find(".search-bar").trigger("submit");
  await flushPromises();
  expect(orbit).toHaveBeenCalledWith(
    ["search", "sodium", "--mc-version", "26.2", "--modloader", "fabric"],
    expect.any(String),
    instance,
  );
  wrapper.unmount();
});
it("refreshes the current query after CLI source settings are saved", async () => {
  vi.mocked(orbit).mockResolvedValue({ results: [project] });
  const { wrapper } = await page(`/browse?instance=${instance.id}`);
  const settings = wrapper.findComponent(CatalogSettings);
  settings.vm.$emit("busy", instance.id, true);
  await flushPromises();
  expect(
    wrapper.find('.search-bar button[type="submit"]').attributes("disabled"),
  ).toBeDefined();
  settings.vm.$emit("changed");
  settings.vm.$emit("busy", instance.id, false);
  await flushPromises();
  expect(orbit).toHaveBeenCalledWith(
    ["search", "sodium", "--mc-version", "26.2", "--modloader", "fabric"],
    expect.any(String),
    instance,
  );
  wrapper.unmount();
});
it("a failed instance load cannot turn into an unfiltered global search", async () => {
  workspace.detail = null;
  const { wrapper } = await page(`/browse?instance=${instance.id}`);
  await wrapper.find(".search-bar").trigger("submit");
  await flushPromises();
  expect(orbit).not.toHaveBeenCalled();
  expect(wrapper.find("[role=alert]").exists()).toBe(true);
  wrapper.unmount();
});
