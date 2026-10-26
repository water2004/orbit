import { beforeEach, expect, it, vi } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import { createMemoryHistory, createRouter } from "vue-router";
import { reactive } from "vue";
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (path: string) => path,
}));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
vi.mock("../src/services/workspace", async () => {
  const { reactive } = await import("vue");
  return {
    workspace: reactive({ accounts: [], instances: [], accountsError: "" }),
    launcher: vi.fn(),
    refresh: vi.fn(),
    act: (fn: () => unknown) => fn(),
  };
});
import Accounts from "../src/pages/Accounts.vue";
import AccountSwitcher from "../src/components/AccountSwitcher.vue";
import InstanceAccount from "../src/components/InstanceAccount.vue";
import DropdownSelect from "../src/components/upstream/DropdownSelect.vue";
import { workspace, launcher } from "../src/services/workspace";
import type { Account, Instance } from "../src/types";
import type { InstanceContext } from "../src/services/instance-context";
const account = {
  id: "player",
  profile_name: "Player",
  provider: "offline",
  is_default: true,
} as Account;
const instance = {
  id: "client",
  name: "Creative",
  directory: "/creative",
  kind: "client",
  is_default: true,
} as Instance;
beforeEach(() => {
  vi.mocked(launcher).mockReset();
  workspace.accounts = [account];
  workspace.instances = [
    instance,
    { ...instance, id: "server", kind: "server" },
  ];
  vi.mocked(launcher).mockImplementation(async (args) =>
    args[0] === "config" ? { providers: [] } : { accounts: workspace.accounts },
  );
  HTMLDialogElement.prototype.showModal = function () {
    this.open = true;
  };
});
it("Accounts only manages identities and has no global or implicit instance selection actions", async () => {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: "/accounts", component: Accounts }],
  });
  await router.push("/accounts");
  const wrapper = mount(Accounts, {
    global: { plugins: [router], stubs: { Teleport: true } },
  });
  await flushPromises();
  expect(wrapper.text()).toContain("Refresh profile");
  expect(wrapper.text()).toContain("Sign out");
  expect(wrapper.text()).not.toContain("Make default");
  expect(wrapper.text()).not.toContain("Assign to instance");
  expect(wrapper.text()).not.toContain("Use for this instance");
  expect(
    vi
      .mocked(launcher)
      .mock.calls.every(
        ([args]) =>
          !(args[0] === "account" && args[1] === "default") &&
          !(args[0] === "instance" && args[1] === "account"),
      ),
  ).toBe(true);
  wrapper.unmount();
});
it("instance account settings explicitly select or clear only that instance", async () => {
  const context = {
    state: reactive({
      instance,
      detail: { selected_account_id: undefined as string | undefined },
    }),
    refresh: vi.fn(),
  } as unknown as InstanceContext;
  const wrapper = mount(InstanceAccount, { props: { context } });
  const picker = wrapper.findComponent(DropdownSelect);
  expect(picker.props("modelValue")).toBe("inherit");
  picker.vm.$emit("update:modelValue", account.id);
  await flushPromises();
  expect(launcher).toHaveBeenLastCalledWith(
    ["instance", "account", "set", account.id],
    expect.any(String),
    instance,
    expect.any(Function),
  );
  context.state.detail!.selected_account_id = account.id;
  await flushPromises();
  picker.vm.$emit("update:modelValue", "inherit");
  await flushPromises();
  expect(launcher).toHaveBeenLastCalledWith(
    ["instance", "account", "clear"],
    expect.any(String),
    instance,
    expect.any(Function),
  );
  expect(context.refresh).toHaveBeenCalledTimes(2);
  wrapper.unmount();
});
it("the header switcher always sets a global default regardless of route", async () => {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/instance/:id/runtime", component: { template: "<div />" } },
      { path: "/accounts", component: Accounts },
    ],
  });
  await router.push("/instance/client/runtime");
  const wrapper = mount(AccountSwitcher, { global: { plugins: [router] } });
  await wrapper.find("button").trigger("click");
  await flushPromises();
  await wrapper.find(".menu-panel button").trigger("click");
  await flushPromises();
  expect(launcher).toHaveBeenCalledWith(
    ["account", "default", "set", "player"],
    expect.any(String),
    null,
  );
  wrapper.unmount();
});
it("account choices display their endpoint names instead of internal provider codes", async () => {
  workspace.accounts = ["LittleSkin", "Community"].map(
    (provider_id, index) => ({
      ...account,
      id: String(index),
      provider: "external-yggdrasil",
      provider_id,
    }),
  );
  const wrapper = mount(InstanceAccount, {
    props: {
      context: {
        state: reactive({ instance, detail: {} }),
        refresh: vi.fn(),
      } as unknown as InstanceContext,
    },
  });
  const choices = wrapper.findComponent(DropdownSelect).props("options");
  expect(choices.slice(1).map((option) => option.description)).toEqual([
    "LittleSkin",
    "Community",
  ]);
  wrapper.unmount();
});
