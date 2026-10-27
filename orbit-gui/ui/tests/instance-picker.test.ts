import { expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (path: string) => path,
}));
import InstancePicker from "../src/components/InstancePicker.vue";
import type { Instance } from "../src/types";
const instances = [
  {
    id: "first",
    name: "26.1.2-Fabric",
    game_version: "26.1.2",
    loader: "fabric",
    kind: "client",
  },
  {
    id: "long",
    name: "一个名称很长的建筑与生存联机测试实例 · Fabric",
    game_version: "26.3",
    loader: "fabric",
    kind: "server",
  },
] as Instance[];
it("keeps the complete instance names and runtime descriptions searchable before selection", async () => {
  const wrapper = mount(InstancePicker, {
    props: { modelValue: "", instances, label: "Target instance" },
  });
  expect(wrapper.classes()).toContain("instance-picker");
  await wrapper.find('[role="combobox"]').trigger("click");
  const options = wrapper.findAll('[role="option"]');
  expect(options.map((option) => option.find("strong").text())).toEqual(
    instances.map((instance) => instance.name),
  );
  expect(options[0].text()).toContain("26.1.2 · fabric · Client");
  await wrapper.find("input").setValue("联机测试");
  expect(wrapper.findAll('[role="option"]')).toHaveLength(1);
  await wrapper.find('[role="option"]').trigger("click");
  expect(wrapper.emitted("update:modelValue")).toEqual([["long"]]);
  await wrapper.setProps({ modelValue: "long" });
  expect(wrapper.find(".instance-picker-name").attributes("title")).toBe(
    instances[1].name,
  );
  wrapper.unmount();
});
