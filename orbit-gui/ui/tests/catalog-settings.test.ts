import { beforeEach, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../src/services/workspace", () => ({ orbit: vi.fn() }));
import CatalogSettings from "../src/components/CatalogSettings.vue";
import { orbit } from "../src/services/workspace";
import type { Instance } from "../src/types";
const instance = { id: "catalog-test", directory: "test" } as Instance;
function response(available = true, enabled = false) {
  return {
    entries: [
      {
        key: "resolver.catalogs",
        value: enabled ? ["modrinth", "curseforge"] : ["modrinth"],
        choices: [
          { id: "modrinth", name: "Modrinth", available: true },
          { id: "curseforge", name: "CurseForge", available },
        ],
      },
    ],
  };
}
beforeEach(() => {
  vi.mocked(orbit).mockReset();
});
it("reads and saves instance catalogs exclusively through the CLI", async () => {
  vi.mocked(orbit)
    .mockResolvedValueOnce(response())
    .mockResolvedValueOnce({ entry: response(true, true).entries[0] });
  const wrapper = mount(CatalogSettings, { props: { instance, ready: true } });
  await flushPromises();
  expect(orbit).toHaveBeenCalledWith(
    ["config", "--scope", "instance", "list"],
    expect.any(String),
    instance,
  );
  await wrapper.find('input[value="curseforge"]').setValue(true);
  await wrapper.find("button").trigger("click");
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
  expect(wrapper.emitted("changed")).toHaveLength(1);
  wrapper.unmount();
});
it("renders credential availability from CLI and prevents empty selection", async () => {
  vi.mocked(orbit).mockResolvedValue(response(false));
  const wrapper = mount(CatalogSettings, { props: { instance, ready: true } });
  await flushPromises();
  expect(
    wrapper.find('input[value="curseforge"]').attributes("disabled"),
  ).toBeDefined();
  await wrapper.find('input[value="modrinth"]').setValue(false);
  expect(wrapper.find("button").attributes("disabled")).toBeDefined();
  expect(orbit).toHaveBeenCalledTimes(1);
  wrapper.unmount();
});
it("keeps edits and error visible on failure without reporting a successful change", async () => {
  vi.mocked(orbit)
    .mockResolvedValueOnce(response())
    .mockRejectedValueOnce(new Error("save failed"));
  const wrapper = mount(CatalogSettings, { props: { instance, ready: true } });
  await flushPromises();
  await wrapper.find('input[value="curseforge"]').setValue(true);
  await wrapper.find("button").trigger("click");
  await flushPromises();
  expect(wrapper.find('[role="alert"]').text()).toContain("save failed");
  expect(wrapper.emitted("changed")).toBeUndefined();
  expect(wrapper.find('input[value="curseforge"]').element.checked).toBe(true);
  wrapper.unmount();
});
it("ignores a previous instance response after switching instances", async () => {
  let finish!: (data: ReturnType<typeof response>) => void;
  vi.mocked(orbit).mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  vi.mocked(orbit).mockResolvedValueOnce(response(true, true));
  const wrapper = mount(CatalogSettings, { props: { instance, ready: true } });
  await wrapper.setProps({ instance: { ...instance, id: "other" } });
  await flushPromises();
  finish(response());
  await flushPromises();
  expect(wrapper.find('input[value="curseforge"]').element.checked).toBe(true);
  wrapper.unmount();
});
