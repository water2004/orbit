import { expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (p: string) => p,
}));
import InstanceCard from "../src/components/upstream/InstanceCard.vue";
import type { Instance } from "../src/types";
const instance: Instance = {
  id: "creative",
  name: "Creative",
  kind: "client",
  is_default: false,
  directory: "/creative",
  launch_account: {
    state: "selected",
    source: "global",
    account: {
      id: "player",
      profile_name: "GBWater",
      provider: { kind: "external-yggdrasil", provider_id: "LittleSkin" },
      authentication_state: "active",
    },
  },
};
it.each([true, false])(
  "client cards show the CLI-resolved account and endpoint (compact=%s)",
  (compact) => {
    const wrapper = mount(InstanceCard, { props: { instance, compact } });
    expect(wrapper.find(".launch-account").text()).toContain("GBWater");
    expect(wrapper.find(".launch-account").text()).toContain("LittleSkin");
    expect(wrapper.find(".launch-account").attributes("title")).toContain(
      "Global default",
    );
    expect(wrapper.text()).not.toContain("external-yggdrasil");
  },
);
it("an unavailable explicit account is not presented as an inherited default", () => {
  const wrapper = mount(InstanceCard, {
    props: {
      instance: {
        ...instance,
        launch_account: {
          state: "unavailable",
          source: "instance",
          account_id: "removed",
        },
      },
    },
  });
  expect(wrapper.text()).toContain("Account unavailable");
  expect(wrapper.text()).not.toContain("GBWater");
});
it("server cards never display a player account", () => {
  const wrapper = mount(InstanceCard, {
    props: { instance: { ...instance, kind: "server" } },
  });
  expect(wrapper.find(".launch-account").exists()).toBe(false);
});
