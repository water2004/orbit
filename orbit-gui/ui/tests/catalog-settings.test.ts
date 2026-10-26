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
  expect(wrapper.find("button").exists()).toBe(false);
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
  expect(
    wrapper.find('input[value="modrinth"]').attributes("disabled"),
  ).toBeDefined();
  expect(orbit).toHaveBeenCalledTimes(1);
  wrapper.unmount();
});
it("rolls back failed autosave without claiming a successful change", async () => {
  vi.mocked(orbit)
    .mockResolvedValueOnce(response())
    .mockRejectedValueOnce(new Error("save failed"));
  const wrapper = mount(CatalogSettings, { props: { instance, ready: true } });
  await flushPromises();
  await wrapper.find('input[value="curseforge"]').setValue(true);
  await flushPromises();
  expect(wrapper.find('[role="alert"]').text()).toContain("save failed");
  expect(wrapper.emitted("changed")).toBeUndefined();
  expect(wrapper.find('input[value="curseforge"]').element.checked).toBe(false);
  wrapper.unmount();
});
it("auto-saving captures the target, blocks further clicks and ignores a late save for a different instance", async () => {
  let finish!: (data: unknown) => void;
  vi.mocked(orbit)
    .mockResolvedValueOnce(response())
    .mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    )
    .mockResolvedValueOnce(response());
  const wrapper = mount(CatalogSettings, { props: { instance, ready: true } });
  await flushPromises();
  await wrapper.find('input[value="curseforge"]').setValue(true);
  expect(wrapper.find('[role="status"]').text()).toContain("Saving");
  expect(
    wrapper.find('input[value="modrinth"]').attributes("disabled"),
  ).toBeDefined();
  await wrapper.setProps({ instance: { ...instance, id: "other" } });
  await flushPromises();
  finish({ entry: response(true, true).entries[0] });
  await flushPromises();
  expect(wrapper.find('input[value="curseforge"]').element.checked).toBe(false);
  expect(vi.mocked(orbit).mock.calls[1][2]).toEqual(instance);
  expect(wrapper.emitted("changed")).toBeUndefined();
  expect(wrapper.emitted("busy")).toEqual([
    [instance.id, true],
    [instance.id, false],
  ]);
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
