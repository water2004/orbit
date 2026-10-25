import { expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (path: string) => path,
}));
import Avatar from "../src/components/Avatar.vue";
it("a transparent skin gets neither the colored placeholder nor a rounded crop", async () => {
  const wrapper = mount(Avatar, {
    props: { src: "transparent-face.png", skin: true, alt: "Player", size: 64 },
  });
  expect(wrapper.classes()).toContain("avatar-skin");
  expect(wrapper.classes()).not.toContain("avatar-placeholder");
  await wrapper.find("img").trigger("error");
  expect(wrapper.classes()).toContain("avatar-placeholder");
  await wrapper.setProps({ src: "refreshed-face.png" });
  expect(wrapper.classes()).not.toContain("avatar-placeholder");
  wrapper.unmount();
});
it("custom fallback art does not inherit a name-derived placeholder background", () => {
  const wrapper = mount(Avatar, {
    slots: { fallback: '<img src="default.png"/>' },
  });
  expect(wrapper.classes()).not.toContain("avatar-placeholder");
  wrapper.unmount();
});
