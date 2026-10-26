import { mount } from "@vue/test-utils";
import { expect, it } from "vitest";
import MemorySettings from "../src/components/MemorySettings.vue";
import RangeField from "../src/components/upstream/RangeField.vue";
const props = {
  value: { min_mib: 512, max_mib: 4096 },
  pending: false,
  capabilities: { physical_memory_mib: 16384, slider_max_mib: 16384 },
};
it("keeps sliders and precise MiB inputs in sync and sends one explicit value", async () => {
  const wrapper = mount(MemorySettings, { props });
  expect(wrapper.findAll("input[type=range]")).toHaveLength(2);
  await wrapper.findAll("input[type=range]")[0].setValue(8192);
  expect(wrapper.findAll("input[type=number]")[0].element).toHaveProperty(
    "value",
    "8192",
  );
  await wrapper.findAll("input[type=number]")[1].setValue(1024);
  await wrapper.find("footer button").trigger("click");
  expect(wrapper.emitted("save")).toEqual([[{ min_mib: 1024, max_mib: 8192 }]]);
  expect(wrapper.text()).toContain("16.0 GiB");
});
it("keeps invalid drafts local and resets only on an explicit action", async () => {
  const wrapper = mount(MemorySettings, { props });
  await wrapper.findAll("input[type=number]")[0].setValue(256);
  expect(wrapper.find("footer button").attributes("disabled")).toBeDefined();
  expect(wrapper.emitted("save")).toBeUndefined();
  await wrapper.findAll("footer button")[1].trigger("click");
  expect(wrapper.emitted("reset")).toEqual([[]]);
  await wrapper.setProps({ pending: true });
  expect(
    wrapper
      .findAll("input")
      .every((input) => input.attributes("disabled") !== undefined),
  ).toBe(true);
});
it("does not invent a slider limit when Launcher has not supplied host facts", () => {
  const wrapper = mount(MemorySettings, {
    props: { value: props.value, pending: false, loading: true },
  });
  expect(wrapper.findAll("input[type=range]")).toHaveLength(0);
  expect(wrapper.find(".progress-spinner").exists()).toBe(true);
});
it("extends only the presentation range for explicitly entered values above physical RAM", async () => {
  const wrapper = mount(RangeField, {
    props: {
      modelValue: 32768,
      min: 1,
      max: 8192,
      label: "Memory",
      unit: "MiB",
    },
  });
  expect(wrapper.find("input[type=range]").attributes("max")).toBe("32768");
  await wrapper.find("input[type=number]").setValue(33000);
  expect(wrapper.emitted("update:modelValue")).toEqual([[33000]]);
});
