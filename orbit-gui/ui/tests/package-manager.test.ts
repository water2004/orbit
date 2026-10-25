import { beforeEach, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (p: string) => p,
}));
vi.mock("../src/services/workspace", async () => {
  const { reactive } = await import("vue");
  return {
    workspace: reactive({
      packages: [],
      selected: { id: "A", directory: "A" },
    }),
    orbit: vi.fn(),
    mutate: vi.fn(),
    act: (fn: () => Promise<unknown>) => fn(),
  };
});
import PackageManager from "../src/components/PackageManager.vue";
import { orbit } from "../src/services/workspace";
import type { Package } from "../src/types";
const pkg: Package = {
  mod_id: "sodium",
  version: "1",
  enabled: true,
  optional: false,
  environment: "client",
  remotes: ["modrinth:test"],
  dependencies: [],
  bundled_count: 0,
};
beforeEach(() => {
  vi.mocked(orbit).mockReset();
  HTMLDialogElement.prototype.showModal = function () {
    this.open = true;
  };
});
it("opening management does not download the version catalog; files only call ownership", async () => {
  const wrapper = mount(PackageManager, {
    props: { pkg },
    global: { stubs: { Teleport: true } },
  });
  expect(orbit).not.toHaveBeenCalled();
  vi.mocked(orbit).mockResolvedValue({
    mod_id: "sodium",
    artifacts: [],
    data: [],
  });
  const files = wrapper
    .findAll("nav button")
    .find((b) => /Files|文件/.test(b.text()))!;
  await files.trigger("click");
  await flushPromises();
  expect(orbit).toHaveBeenCalledTimes(1);
  expect(vi.mocked(orbit).mock.calls[0][0]).toEqual(["ownership", "sodium"]);
  expect(wrapper.find("footer button").text()).toMatch(/Close|关闭/);
  await wrapper.find("footer button").trigger("click");
  expect(wrapper.emitted("close")).toHaveLength(1);
  wrapper.unmount();
});
