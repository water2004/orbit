import { beforeEach, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { shallowRef } from "vue";
import { createMemoryHistory, createRouter } from "vue-router";
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (path: string) => path,
}));
vi.mock("../src/services/workspace", () => ({
  orbit: vi.fn(),
  launcher: vi.fn(),
  refresh: vi.fn(),
  launch: vi.fn(),
  act: (fn: () => Promise<unknown>) => fn(),
}));
import InstancePage from "../src/pages/Instance.vue";
import CatalogSettings from "../src/components/CatalogSettings.vue";
import {
  createInstanceContext,
  instanceContextKey,
} from "../src/services/instance-context";
import { orbit } from "../src/services/workspace";
import { discoverySession } from "../src/services/discovery";
import type { Detail, Instance, Package } from "../src/types";

const instance = {
  id: "sources-test",
  name: "Creative",
  kind: "client",
  directory: "X:/test",
} as Instance;
const entry = (value = ["modrinth"]) => ({
  key: "resolver.catalogs",
  value,
  choices: [
    { id: "modrinth", name: "Modrinth", available: true },
    { id: "curseforge", name: "CurseForge", available: true },
  ],
});
const updates = {
  updates: [{ mod_id: "sodium", current_version: "1", new_version: "2" }],
  diagnostics: [],
  warnings: [],
};
beforeEach(() => {
  vi.mocked(orbit).mockReset();
  vi.mocked(orbit).mockImplementation(async (args) =>
    args[0] === "outdated"
      ? updates
      : args.includes("set")
        ? { entry: entry(["modrinth", "curseforge"]) }
        : { entries: [entry()] },
  );
});
async function page() {
  const context = createInstanceContext(instance);
  context.state.detail = {
    ...instance,
    desired: { minecraft: "26.2", loader: "fabric" },
  } as Detail;
  context.state.packages = [
    { mod_id: "sodium", version: "1", enabled: true } as Package,
  ];
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/", component: { template: "<div />" } },
      { path: "/instance/:id/:tab", component: InstancePage },
    ],
  });
  await router.push(`/instance/${instance.id}/mods`);
  const wrapper = mount(InstancePage, {
    global: {
      plugins: [router],
      provide: { [instanceContextKey as symbol]: shallowRef(context) },
    },
  });
  await flushPromises();
  return { wrapper, context };
}
function button(
  wrapper: Awaited<ReturnType<typeof page>>["wrapper"],
  text: string,
) {
  return wrapper.findAll("button").find((item) => item.text().includes(text))!;
}
it("exposes the same auto-saving source control in mod management, targeting this instance", async () => {
  const { wrapper } = await page();
  Object.assign(discoverySession(instance.id), {
    searched: true,
    query: "sodium",
  });
  expect(wrapper.findComponent(CatalogSettings).props("instance")).toEqual(
    instance,
  );
  expect(orbit).toHaveBeenCalledWith(
    ["config", "--scope", "instance", "list"],
    expect.any(String),
    instance,
  );
  await button(wrapper, "Check for updates").trigger("click");
  await flushPromises();
  expect(button(wrapper, "Update all")).toBeDefined();
  await wrapper.find('input[value="curseforge"]').setValue(true);
  await flushPromises();
  expect(orbit).toHaveBeenLastCalledWith(
    [
      "config",
      "--scope",
      "instance",
      "set",
      "resolver.catalogs",
      "modrinth,curseforge",
    ],
    expect.any(String),
    instance,
  );
  expect(button(wrapper, "Update all")).toBeUndefined();
  expect(button(wrapper, "Installed").classes()).toContain("active");
  expect(discoverySession(instance.id).searched).toBe(false);
  expect(discoverySession(instance.id).query).toBe("sodium");
  wrapper.unmount();
});
it("blocks update checks while a source change is saving", async () => {
  const { wrapper } = await page();
  let finish!: (data: unknown) => void;
  vi.mocked(orbit).mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  await wrapper.find('input[value="curseforge"]').setValue(true);
  expect(
    button(wrapper, "Check for updates").attributes("disabled"),
  ).toBeDefined();
  finish({ entry: entry(["modrinth", "curseforge"]) });
  await flushPromises();
  expect(
    button(wrapper, "Check for updates").attributes("disabled"),
  ).toBeUndefined();
  wrapper.unmount();
});
it("does not show stale update results after the inventory changes", async () => {
  const { wrapper, context } = await page();
  let finish!: (data: unknown) => void;
  vi.mocked(orbit).mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  await button(wrapper, "Check for updates").trigger("click");
  expect(wrapper.findComponent(CatalogSettings).props("disabled")).toBe(true);
  context.state.packages = [];
  await flushPromises();
  finish(updates);
  await flushPromises();
  expect(button(wrapper, "Update all")).toBeUndefined();
  expect(wrapper.findComponent(CatalogSettings).props("disabled")).toBe(false);
  wrapper.unmount();
});
