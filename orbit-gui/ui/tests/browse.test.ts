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
    workspace: reactive({ instances: [], loading: false }),
    launcher: vi.fn(),
    orbit: vi.fn(),
    act: (fn: () => Promise<unknown>) => fn(),
  };
});
import Browse from "../src/pages/Browse.vue";
import { workspace, orbit, launcher } from "../src/services/workspace";
import InstancePicker from "../src/components/InstancePicker.vue";
import CatalogSettings from "../src/components/CatalogSettings.vue";
import { discoverySession } from "../src/services/discovery";
import type { Detail, Instance, SearchResult } from "../src/types";
const instance = {
  id: "browse-test",
  name: "Creative",
  directory: "test",
  kind: "client",
  is_default: true,
} as Instance;
const other = {
  ...instance,
  id: "other",
  name: "Server",
  directory: "server",
  kind: "server",
};
const project = {
  platform: "modrinth",
  project_id: "AANobbMI",
  name: "Sodium",
  description: "Rendering",
  downloads: 100,
  compatible: true,
} as SearchResult;
let details: Record<string, Detail>;
beforeEach(() => {
  vi.mocked(orbit).mockReset();
  vi.mocked(launcher).mockReset();
  workspace.instances = [instance, other];
  details = {
    [instance.id]: {
      ...instance,
      desired: { minecraft: "26.2", loader: "fabric" },
    } as Detail,
    [other.id]: {
      ...other,
      desired: { minecraft: "26.3", loader: "neoforge" },
    } as Detail,
  };
  vi.mocked(launcher).mockImplementation(
    async (_args, _label, target) => details[target!.id],
  );
  vi.mocked(orbit).mockImplementation(async (args) =>
    args[0] === "list" ? { packages: [] } : { results: [project] },
  );
  for (const target of [instance, other])
    Object.assign(discoverySession(target.id), {
      scope: "",
      query: "sodium",
      results: [project],
      searched: true,
      loading: false,
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
  await flushPromises();
  return { wrapper, router };
}
it("installs into the explicit target, guards double clicks and preserves the query", async () => {
  let finish!: () => void;
  vi.mocked(orbit).mockImplementation(async (args) =>
    args[0] === "add"
      ? new Promise<void>((resolve) => {
          finish = resolve;
        })
      : { packages: [] },
  );
  const { wrapper } = await page(`/browse?instance=${instance.id}`);
  await wrapper.find(".project-actions button").trigger("click");
  expect(orbit).toHaveBeenCalledWith(
    ["add", "mr:AANobbMI"],
    expect.any(String),
    instance,
    undefined,
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
it("sidebar discovery requires an explicit target even when a CLI default exists", async () => {
  const { wrapper, router } = await page("/browse");
  expect(wrapper.findComponent(InstancePicker).props("modelValue")).toBe("");
  expect(wrapper.find(".discovery-target").element.tagName).toBe("DIV");
  expect(wrapper.text()).toContain("Where should the mods be installed?");
  expect(router.currentRoute.value.query.instance).toBeUndefined();
  expect(orbit).not.toHaveBeenCalled();
  expect(launcher).not.toHaveBeenCalled();
  wrapper.unmount();
});
it("switches filtering and installation target with the visible instance picker", async () => {
  const { wrapper, router } = await page(`/browse?instance=${instance.id}`);
  wrapper.findComponent(InstancePicker).vm.$emit("update:modelValue", other.id);
  await flushPromises();
  expect(router.currentRoute.value.path).toBe("/browse");
  expect(launcher).toHaveBeenLastCalledWith(
    ["instance", "show"],
    expect.any(String),
    other,
  );
  expect(orbit).toHaveBeenLastCalledWith(
    ["search", "sodium", "--mc-version", "26.3", "--modloader", "neoforge"],
    expect.any(String),
    other,
  );
  await wrapper.find(".project-actions button").trigger("click");
  await flushPromises();
  expect(
    vi.mocked(orbit).mock.calls.find(([args]) => args[0] === "add")?.[2],
  ).toEqual(other);
  wrapper.unmount();
});
it("runtime inventory refresh invalidates compatibility and queries only the new runtime", async () => {
  const { wrapper } = await page(`/browse?instance=${instance.id}`);
  details[instance.id] = {
    ...details[instance.id],
    desired: { minecraft: "26.3", loader: "fabric" },
  } as Detail;
  workspace.instances = [{ ...instance }, other];
  await flushPromises();
  expect(discoverySession(instance.id).query).toBe("sodium");
  const searches = vi
    .mocked(orbit)
    .mock.calls.filter(([args]) => args[0] === "search");
  expect(searches).toHaveLength(1);
  expect(searches[0][0]).toContain("26.3");
  wrapper.unmount();
});
it("search always passes the exact instance runtime axes", async () => {
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
it("refreshes the query after CLI catalog settings are saved", async () => {
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
it("a missing instance never falls back to the default or first instance", async () => {
  const { wrapper, router } = await page("/browse?instance=removed");
  expect(wrapper.text()).toContain("This instance is not available");
  expect(router.currentRoute.value.query.instance).toBe("removed");
  expect(launcher).not.toHaveBeenCalled();
  expect(orbit).not.toHaveBeenCalled();
  wrapper.unmount();
});
it("a failed instance load cannot turn into an unfiltered global search", async () => {
  vi.mocked(launcher).mockRejectedValue(new Error("invalid runtime"));
  const { wrapper } = await page(`/browse?instance=${instance.id}`);
  await wrapper.find(".search-bar").trigger("submit");
  await flushPromises();
  expect(orbit).not.toHaveBeenCalled();
  expect(wrapper.find("[role=alert]").exists()).toBe(true);
  wrapper.unmount();
});
it("a search failure is not displayed as an empty successful result", async () => {
  const { wrapper } = await page(`/browse?instance=${instance.id}`);
  vi.mocked(orbit).mockRejectedValueOnce(
    new Error("invalid CurseForge JSON response"),
  );
  await wrapper.find(".search-bar").trigger("submit");
  await flushPromises();
  expect(wrapper.find("[role=alert]").text()).toContain(
    "invalid CurseForge JSON response",
  );
  expect(wrapper.text()).not.toContain("No matching mods");
  expect(wrapper.find(".results-column > small").exists()).toBe(false);
  expect(wrapper.findAll(".project-card")).toHaveLength(0);
  wrapper.unmount();
});
it("vanilla cannot invoke Orbit discovery or source settings", async () => {
  details[instance.id].desired.loader = "vanilla";
  const { wrapper } = await page(`/browse?instance=${instance.id}`);
  await wrapper.find(".search-bar").trigger("submit");
  await flushPromises();
  expect(orbit).not.toHaveBeenCalled();
  expect(wrapper.findComponent(CatalogSettings).exists()).toBe(false);
  expect(
    wrapper.find('.search-bar button[type="submit"]').attributes("disabled"),
  ).toBeDefined();
  wrapper.unmount();
});
it("an install already started remains bound to its original target after switching", async () => {
  let finish!: () => void;
  vi.mocked(orbit).mockImplementation(async (args) =>
    args[0] === "add"
      ? new Promise<void>((resolve) => {
          finish = resolve;
        })
      : args[0] === "list"
        ? { packages: [] }
        : { results: [project] },
  );
  const { wrapper } = await page(`/browse?instance=${instance.id}`);
  await wrapper.find(".project-actions button").trigger("click");
  wrapper.findComponent(InstancePicker).vm.$emit("update:modelValue", other.id);
  await flushPromises();
  finish();
  await flushPromises();
  const adds = vi
    .mocked(orbit)
    .mock.calls.filter(([args]) => args[0] === "add");
  expect(adds).toHaveLength(1);
  expect(adds[0][2]).toEqual(instance);
  expect(wrapper.findComponent(InstancePicker).props("modelValue")).toBe(
    other.id,
  );
  wrapper.unmount();
});
