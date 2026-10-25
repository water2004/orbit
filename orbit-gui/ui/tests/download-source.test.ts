import { beforeEach, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
vi.mock("../src/services/cli", () => ({ run: vi.fn() }));
import { run } from "../src/services/cli";
import DownloadSourcePicker from "../src/components/DownloadSourcePicker.vue";
import DropdownSelect from "../src/components/upstream/DropdownSelect.vue";
beforeEach(() => vi.mocked(run).mockReset());
it("reads and saves the source through Launcher config commands and credits the service", async () => {
  vi.mocked(run)
    .mockResolvedValueOnce({
      value: "official",
      allowed_values: ["official", "bmclapi"],
    })
    .mockResolvedValueOnce({});
  const wrapper = mount(DownloadSourcePicker);
  await flushPromises();
  expect(vi.mocked(run).mock.calls[0][0].args).toEqual([
    "config",
    "get",
    "network.download-source",
  ]);
  wrapper
    .findComponent(DropdownSelect)
    .vm.$emit("update:modelValue", "bmclapi");
  await flushPromises();
  expect(vi.mocked(run).mock.calls[1][0]).toMatchObject({
    kind: "launcher",
    args: ["config", "set", "network.download-source", "bmclapi"],
  });
  expect(wrapper.emitted("changed")).toHaveLength(1);
  expect(wrapper.find("a").attributes("href")).toBe(
    "https://bmclapidoc.bangbang93.com/",
  );
});
it("does not show a setting as saved when the CLI rejected it", async () => {
  vi.mocked(run)
    .mockResolvedValueOnce({
      value: "official",
      allowed_values: ["official", "bmclapi"],
    })
    .mockRejectedValueOnce(new Error("config is locked"));
  const wrapper = mount(DownloadSourcePicker);
  await flushPromises();
  wrapper
    .findComponent(DropdownSelect)
    .vm.$emit("update:modelValue", "bmclapi");
  await flushPromises();
  expect(wrapper.findComponent(DropdownSelect).props("modelValue")).toBe(
    "official",
  );
  expect(wrapper.emitted("changed")).toBeUndefined();
  expect(wrapper.text()).toContain("config is locked");
});
