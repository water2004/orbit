import { beforeEach, expect, it, vi } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import { createMemoryHistory, createRouter, RouterView } from "vue-router";
import { defineComponent, h } from "vue";
vi.mock("../src/services/workspace", async () => {
  const { reactive } = await import("vue");
  return {
    workspace: reactive({ instances: [], initialized: true, loading: false }),
    launcher: vi.fn(),
    orbit: vi.fn(),
  };
});
import InstanceContext from "../src/components/InstanceContext.vue";
import { useInstanceContext } from "../src/services/instance-context";
import { workspace, launcher, orbit } from "../src/services/workspace";
import type { Instance } from "../src/types";
const a = {
  id: "a",
  name: "A",
  directory: "/a",
  kind: "client",
  is_default: true,
} as Instance;
const b = { ...a, id: "b", name: "B", directory: "/b" };
const Child = defineComponent({
  setup() {
    const context = useInstanceContext();
    return () => h("div", { class: "child" }, context.state.instance.name);
  },
});
beforeEach(() => {
  vi.mocked(launcher).mockReset();
  vi.mocked(orbit).mockReset();
  workspace.instances = [a, b];
  vi.mocked(launcher).mockImplementation(async (_args, _label, target) => ({
    ...target,
    desired: { minecraft: "26.2", loader: "fabric" },
  }));
  vi.mocked(orbit).mockResolvedValue({ packages: [] });
});
async function page(path: string) {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/", component: { template: "<div />" } },
      {
        path: "/instance/:id",
        component: InstanceContext,
        children: [{ path: "mods", component: Child }],
      },
    ],
  });
  await router.push(path);
  const wrapper = mount(RouterView, { global: { plugins: [router] } });
  await flushPromises();
  return { router, wrapper };
}
it("invalid route targets cannot load the CLI default or first instance", async () => {
  const { wrapper } = await page("/instance/removed/mods");
  expect(wrapper.text()).toContain("Instance not found");
  expect(launcher).not.toHaveBeenCalled();
  expect(orbit).not.toHaveBeenCalled();
  wrapper.unmount();
});
it("refreshing inventory updates the same page context without a hidden selection", async () => {
  const { wrapper } = await page("/instance/b/mods");
  expect(wrapper.find(".child").text()).toBe("B");
  workspace.instances = [a, { ...b, name: "Renamed B" }];
  await flushPromises();
  expect(wrapper.find(".child").text()).toBe("Renamed B");
  expect(
    vi.mocked(launcher).mock.calls.every((call) => call[2]?.id === "b"),
  ).toBe(true);
  wrapper.unmount();
});
it("navigation replaces context and late responses cannot overwrite the new page", async () => {
  let finish!: (value: unknown) => void;
  vi.mocked(launcher).mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const { wrapper, router } = await page("/instance/a/mods");
  await router.push("/instance/b/mods");
  await flushPromises();
  expect(wrapper.find(".child").text()).toBe("B");
  finish({ ...a, desired: { loader: "fabric" } });
  await flushPromises();
  expect(wrapper.find(".child").text()).toBe("B");
  expect(vi.mocked(orbit).mock.calls.map((call) => call[2]?.id)).toEqual(["b"]);
  wrapper.unmount();
});
