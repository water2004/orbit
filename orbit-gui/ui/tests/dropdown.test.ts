import { expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import DropdownSelect from "../src/components/upstream/DropdownSelect.vue";
const options = [
  { value: "a", label: "Alpha" },
  { value: "b", label: "Beta" },
  { value: "c", label: "Gamma" },
];
it("uses the same bounded searchable control with keyboard selection and Escape dismissal", async () => {
  const wrapper = mount(DropdownSelect, {
    props: { label: "Version", options, modelValue: "a" },
  });
  await wrapper.find("[role=combobox]").trigger("click");
  expect(wrapper.find("[role=listbox]").exists()).toBe(true);
  await wrapper.find("input").setValue("bet");
  expect(wrapper.findAll("[role=option]")).toHaveLength(1);
  await wrapper.find("input").trigger("keydown", { key: "Enter" });
  expect(wrapper.emitted("update:modelValue")?.[0]).toEqual(["b"]);
  await wrapper
    .find("[role=combobox]")
    .trigger("keydown", { key: "ArrowDown" });
  await wrapper.find("input").trigger("keydown", { key: "End" });
  await wrapper.find("input").trigger("keydown", { key: "Enter" });
  expect(wrapper.emitted("update:modelValue")?.[1]).toEqual(["c"]);
  await wrapper.find("[role=combobox]").trigger("click");
  await wrapper.find("input").trigger("keydown", { key: "Escape" });
  expect(wrapper.find("[role=listbox]").exists()).toBe(false);
  wrapper.unmount();
});
it("has a real loading state and no native select popup", async () => {
  const wrapper = mount(DropdownSelect, {
    props: { label: "Loader", options, loading: true },
  });
  expect(wrapper.find("select").exists()).toBe(false);
  expect(wrapper.find("[role=combobox]").attributes("disabled")).toBeDefined();
  expect(wrapper.find(".progress-spinner").exists()).toBe(true);
  await wrapper.find("[role=combobox]").trigger("click");
  expect(wrapper.find("[role=listbox]").exists()).toBe(false);
  wrapper.unmount();
});
