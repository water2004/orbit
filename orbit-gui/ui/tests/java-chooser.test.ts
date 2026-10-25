import { beforeEach, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("../src/services/workspace", () => ({
  launcher: vi.fn(),
  act: (fn: () => Promise<unknown>) => fn(),
}));
import JavaChooser from "../src/components/JavaChooser.vue";
import { launcher } from "../src/services/workspace";
import { open } from "@tauri-apps/plugin-dialog";
import DropdownSelect from "../src/components/upstream/DropdownSelect.vue";
const candidates = [
  {
    provider: "system",
    executable: "C:/Java/bin/java.exe",
    major: 25,
    version: "25.0.2",
  },
  {
    provider: "mojang",
    executable: "C:/managed/bin/java.exe",
    major: 25,
    version: "25.0.1",
  },
];
beforeEach(() => {
  vi.mocked(launcher).mockReset();
});
it("exposes local Java and policies without asking users to type CLI values", async () => {
  const wrapper = mount(JavaChooser, { props: { candidates, inherit: true } });
  expect(launcher).not.toHaveBeenCalled();
  expect(
    wrapper
      .findComponent(DropdownSelect)
      .props("options")
      .map((o: { value: string }) => o.value),
  ).toEqual(["inherit", "auto", "managed", "C:/Java/bin/java.exe"]);
  wrapper
    .findComponent(DropdownSelect)
    .vm.$emit("update:modelValue", "C:/Java/bin/java.exe");
  expect(wrapper.emitted("update:modelValue")?.[0]).toEqual([
    "C:/Java/bin/java.exe",
  ]);
  wrapper
    .findComponent(DropdownSelect)
    .vm.$emit("update:modelValue", "inherit");
  expect(wrapper.emitted("update:modelValue")?.[1]).toEqual([null]);
  wrapper.unmount();
});
it("passes a chosen executable unchanged and leaves validation to Launcher", async () => {
  vi.mocked(open).mockResolvedValue("C:/Other Java/bin/java.exe");
  const wrapper = mount(JavaChooser, { props: { candidates } });
  await wrapper.findAll("button")[1].trigger("click");
  await flushPromises();
  expect(wrapper.emitted("update:modelValue")?.[0]).toEqual([
    "C:/Other Java/bin/java.exe",
  ]);
  wrapper.unmount();
});
