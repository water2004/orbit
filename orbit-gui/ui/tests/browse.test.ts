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
    workspace: reactive({ instances: [], packages: [], detail: null }),
    orbit: vi.fn(),
    mutate: vi.fn(),
    act: (fn: () => Promise<unknown>) => fn(),
  };
});
import Browse from "../src/pages/Browse.vue";
import { workspace, mutate } from "../src/services/workspace";
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
  workspace.instances = [instance];
  workspace.packages = [];
  workspace.detail = {
    desired: { minecraft: "26.2", loader: "fabric" },
  } as Detail;
  for (const id of [undefined, instance.id])
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
      { path: "/instance/:id/:tab", component: Browse },
      { path: "/create", component: { template: "<div />" } },
    ],
  });
  await router.push(path);
  await router.isReady();
  const wrapper = mount(Browse, {
    global: { plugins: [router], stubs: { Teleport: true } },
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
  const { wrapper } = await page(`/instance/${instance.id}/browse`);
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
it("global discovery asks for a target and then starts the same add path", async () => {
  vi.mocked(mutate).mockResolvedValue(undefined);
  const { wrapper, router } = await page("/browse");
  await wrapper.find(".project-actions button").trigger("click");
  expect(mutate).not.toHaveBeenCalled();
  await wrapper.find(".target-list button").trigger("click");
  await flushPromises();
  expect(router.currentRoute.value.params.id).toBe(instance.id);
  expect(mutate).toHaveBeenCalledOnce();
  expect(vi.mocked(mutate).mock.calls[0][0]).toEqual(["add", "mr:AANobbMI"]);
  expect(vi.mocked(mutate).mock.calls[0][2]).toEqual(instance);
  wrapper.unmount();
});
it("invalidates displayed compatibility after the instance runtime changes", async () => {
  const { wrapper } = await page(`/instance/${instance.id}/browse`);
  expect(wrapper.findAll(".project-card")).toHaveLength(1);
  workspace.detail = {
    desired: { minecraft: "26.3", loader: "fabric" },
  } as Detail;
  await flushPromises();
  expect(wrapper.findAll(".project-card")).toHaveLength(0);
  expect(discoverySession(instance.id).query).toBe("sodium");
  wrapper.unmount();
});
