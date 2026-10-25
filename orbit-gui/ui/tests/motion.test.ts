import { afterEach, expect, it, vi } from "vitest";
import { defineComponent, h, nextTick, onMounted, ref } from "vue";
import { mount } from "@vue/test-utils";
import ContentStage from "../src/components/ContentStage.vue";
import ActionMenu from "../src/components/ActionMenu.vue";

afterEach(() => {
  document.body.innerHTML = "";
});

it("a stage retains its controls during progress updates and remounts only on navigation", async () => {
  const mounted = vi.fn();
  const Control = defineComponent({
    setup() {
      onMounted(mounted);
      const value = ref("");
      return () =>
        h("input", {
          value: value.value,
          onInput: (event: Event) =>
            (value.value = (event.target as HTMLInputElement).value),
        });
    },
  });
  const wrapper = mount(ContentStage, {
    props: { stage: "versions" },
    attrs: { "aria-label": "Mod policy" },
    slots: { default: () => h(Control) },
    attachTo: document.body,
  });
  await wrapper.get("input").setValue("local edit");
  const input = wrapper.get("input").element as HTMLInputElement;
  input.focus();
  await wrapper.setProps({ stage: "versions" });
  expect(wrapper.get("input").element).toBe(input);
  expect(document.activeElement).toBe(input);
  expect(input.value).toBe("local edit");
  expect(mounted).toHaveBeenCalledTimes(1);
  expect(wrapper.get(".content-stage").attributes("aria-label")).toBe(
    "Mod policy",
  );
  await wrapper.setProps({ stage: "files" });
  expect(wrapper.get("input").element).not.toBe(input);
  expect(mounted).toHaveBeenCalledTimes(2);
  wrapper.unmount();
});

it("animated menus support keyboard entry, immediate selection and Escape focus restoration", async () => {
  const selected = vi.fn();
  const wrapper = mount(ActionMenu, {
    props: { label: "Manage" },
    slots: { default: () => h("button", { onClick: selected }, "Upgrade") },
    attachTo: document.body,
  });
  const trigger = wrapper.get<HTMLButtonElement>(".menu-trigger");
  expect(trigger.attributes("aria-expanded")).toBe("false");
  await trigger.trigger("keydown", { key: "ArrowDown" });
  expect(document.activeElement).toBe(
    wrapper.get(".menu-panel button").element,
  );
  await wrapper.get(".menu-panel button").trigger("keydown", { key: "Escape" });
  expect(document.activeElement).toBe(trigger.element);
  expect(wrapper.find(".menu-panel").exists()).toBe(false);
  await trigger.trigger("click");
  await wrapper.get(".menu-panel button").trigger("click");
  expect(selected).toHaveBeenCalledTimes(1);
  expect(trigger.attributes("aria-expanded")).toBe("false");
  wrapper.unmount();
});

it("menu dismissal neither steals outside focus nor leaves listeners behind", async () => {
  const wrapper = mount(ActionMenu, {
    props: { label: "Manage" },
    slots: { default: () => h("button", "Upgrade") },
    attachTo: document.body,
  });
  const outside = document.createElement("input");
  document.body.append(outside);
  const trigger = wrapper.get(".menu-trigger");
  await trigger.trigger("click");
  outside.dispatchEvent(new Event("pointerdown", { bubbles: true }));
  outside.focus();
  await nextTick();
  expect(trigger.attributes("aria-expanded")).toBe("false");
  expect(document.activeElement).toBe(outside);
  await trigger.trigger("click");
  trigger.element.focus();
  outside.focus();
  await nextTick();
  expect(trigger.attributes("aria-expanded")).toBe("false");
  const remove = vi.spyOn(document, "removeEventListener");
  wrapper.unmount();
  expect(remove).toHaveBeenCalledWith("pointerdown", expect.any(Function));
  remove.mockRestore();
});
