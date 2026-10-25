import { beforeEach, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
import { createMemoryHistory, createRouter } from "vue-router";
const calls = vi.hoisted(() => ({ launcher: vi.fn(), run: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (p: string) => p,
}));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
vi.mock("../src/services/task-scope", () => ({
  useCommandScope: () => ({ ...calls, task: undefined }),
}));
vi.mock("../src/services/workspace", async () => {
  const { reactive } = await import("vue");
  return {
    workspace: reactive({ accounts: [], selected: null, accountsError: "" }),
    act: (fn: () => Promise<unknown>) => fn(),
  };
});
import AuthEndpointDialog from "../src/components/AuthEndpointDialog.vue";
import AuthEndpointDrop from "../src/components/AuthEndpointDrop.vue";
import Accounts from "../src/pages/Accounts.vue";
import DropdownSelect from "../src/components/upstream/DropdownSelect.vue";
import {
  endpointDialog,
  openAuthEndpoint,
} from "../src/services/auth-endpoint";
import { language } from "../src/i18n";
import config from "../../tauri.conf.json";
const raw =
  "authlib-injector:yggdrasil-server:https%3A%2F%2Fskin.example%2Fapi";
const preview = {
  api_root: "https://skin.example/api/",
  server_name: "Example Skin",
  suggested_id: "Example Skin",
  existing_provider_id: null,
  insecure_http: false,
};
beforeEach(() => {
  language.value = "en";
  endpointDialog.value = undefined;
  calls.launcher.mockReset();
  calls.run.mockReset();
  calls.launcher.mockImplementation(async (args: string[]) => {
    if (args[2] === "inspect") return preview;
    if (args[2] === "add") return { provider: { id: "Example Skin" } };
    return { providers: [{ id: "Example Skin", api_root: preview.api_root }] };
  });
  HTMLDialogElement.prototype.showModal = function () {
    this.open = true;
  };
});
const options = { global: { stubs: { Teleport: true } } };
it("previews a raw drag URI via Launcher and writes only after explicit confirmation", async () => {
  const wrapper = mount(AuthEndpointDialog, {
    ...options,
    props: { address: raw },
  });
  await flushPromises();
  expect(calls.launcher).toHaveBeenCalledExactlyOnceWith(
    ["config", "yggdrasil", "inspect", raw],
    expect.any(String),
    null,
  );
  expect(wrapper.text()).toContain(preview.api_root);
  await wrapper
    .findAll("button")
    .find((button) => button.text() === "Add service")!
    .trigger("click");
  await flushPromises();
  expect(calls.launcher).toHaveBeenLastCalledWith(
    ["config", "yggdrasil", "add", "Example Skin", preview.api_root],
    expect.any(String),
    null,
  );
  expect(wrapper.emitted("added")).toEqual([["Example Skin"]]);
  expect(calls.run).not.toHaveBeenCalled();
  wrapper.unmount();
});
it("cancelling a preview never writes configuration", async () => {
  const wrapper = mount(AuthEndpointDialog, {
    ...options,
    props: { address: raw },
  });
  await flushPromises();
  await wrapper
    .findAll("button")
    .find((button) => button.text() === "Cancel")!
    .trigger("click");
  expect(wrapper.emitted("close")).toHaveLength(1);
  expect(calls.launcher).toHaveBeenCalledTimes(1);
  wrapper.unmount();
});
it("editing an inspected address invalidates confirmation and forwards pasted text unchanged", async () => {
  const wrapper = mount(AuthEndpointDialog, {
    ...options,
    props: { address: raw },
  });
  await flushPromises();
  const pasted =
    "authlib-injector:yggdrasil-server:https%3A%2F%2Fother.example";
  await wrapper.find(".field input").setValue(pasted);
  expect(
    wrapper
      .findAll("button")
      .find((button) => button.text() === "Add service")!
      .attributes("disabled"),
  ).toBeDefined();
  await wrapper
    .findAll("button")
    .find((button) => button.text() === "Check service")!
    .trigger("click");
  await flushPromises();
  expect(calls.launcher).toHaveBeenLastCalledWith(
    ["config", "yggdrasil", "inspect", pasted],
    expect.any(String),
    null,
  );
  wrapper.unmount();
});
it("HTTP permission is explicit and failed inspection cannot enable saving", async () => {
  calls.launcher.mockRejectedValueOnce(new Error("HTTPS required"));
  const wrapper = mount(AuthEndpointDialog, {
    ...options,
    props: { address: raw },
  });
  await flushPromises();
  expect(wrapper.find('[role="alert"]').text()).toContain("HTTPS required");
  expect(
    wrapper
      .findAll("button")
      .find((button) => button.text() === "Add service")!
      .attributes("disabled"),
  ).toBeDefined();
  await wrapper.find('input[type="checkbox"]').setValue(true);
  expect(calls.launcher).toHaveBeenCalledTimes(1);
  await wrapper
    .findAll("button")
    .find((button) => button.text() === "Check service")!
    .trigger("click");
  await flushPromises();
  expect(calls.launcher.mock.lastCall![0]).toContain("--allow-insecure-http");
  wrapper.unmount();
});
it("recognizes an existing endpoint without writing a duplicate", async () => {
  calls.launcher.mockResolvedValueOnce({
    ...preview,
    existing_provider_id: "already-added",
  });
  const wrapper = mount(AuthEndpointDialog, {
    ...options,
    props: { address: raw },
  });
  await flushPromises();
  await wrapper
    .findAll("button")
    .find((button) => button.text() === "Use existing service")!
    .trigger("click");
  expect(wrapper.emitted("added")).toEqual([["already-added"]]);
  expect(calls.launcher).toHaveBeenCalledTimes(1);
  wrapper.unmount();
});
function drop(text: string, types = ["text/plain"]) {
  const event = new Event("drop", { bubbles: true, cancelable: true });
  Object.defineProperty(event, "dataTransfer", {
    value: { types, getData: () => text },
  });
  window.dispatchEvent(event);
}
async function shell(path = "/") {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/", component: { template: "<div />" } },
      { path: "/accounts", component: Accounts },
    ],
  });
  await router.push(path);
  await router.isReady();
  const wrapper = mount(AuthEndpointDrop, {
    global: { ...options.global, plugins: [router] },
  });
  return { wrapper, router };
}
it("accepts a browser button anywhere and routes to accounts only after confirmation", async () => {
  const { wrapper, router } = await shell();
  drop(raw);
  await flushPromises();
  expect(router.currentRoute.value.path).toBe("/");
  expect(calls.launcher.mock.calls.map(([args]) => args[2])).toEqual([
    "inspect",
  ]);
  await wrapper
    .findAll("button")
    .find((button) => button.text() === "Add service")!
    .trigger("click");
  await flushPromises();
  expect(router.currentRoute.value.path).toBe("/accounts");
  expect(router.currentRoute.value.query.provider).toBe("Example Skin");
  wrapper.unmount();
});
it("manual and dropped endpoints use the same dialog and preserve the manual callback", async () => {
  const callback = vi.fn();
  const { wrapper } = await shell();
  openAuthEndpoint("", callback);
  await flushPromises();
  expect(calls.launcher).not.toHaveBeenCalled();
  drop(raw);
  await flushPromises();
  expect(wrapper.findAllComponents(AuthEndpointDialog)).toHaveLength(1);
  await wrapper
    .findAll("button")
    .find((button) => button.text() === "Add service")!
    .trigger("click");
  await flushPromises();
  expect(callback).toHaveBeenCalledWith("Example Skin");
  wrapper.unmount();
});
it("a second drop cannot replace an endpoint while its CLI request is running", async () => {
  let finish!: (value: typeof preview) => void;
  calls.launcher.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const { wrapper } = await shell();
  drop(raw);
  await flushPromises();
  drop("authlib-injector:yggdrasil-server:https%3A%2F%2Fother.example");
  await flushPromises();
  expect(endpointDialog.value?.address).toBe(raw);
  expect(calls.launcher).toHaveBeenCalledTimes(1);
  finish(preview);
  await flushPromises();
  wrapper.unmount();
});
it("ignores files, unrelated text and local text drags, and unregisters listeners", async () => {
  const { wrapper } = await shell();
  drop("ordinary text");
  drop(raw, ["Files", "text/plain"]);
  window.dispatchEvent(new Event("dragstart"));
  drop(raw);
  window.dispatchEvent(new Event("dragend"));
  await flushPromises();
  expect(calls.launcher).not.toHaveBeenCalled();
  wrapper.unmount();
  drop(raw);
  expect(endpointDialog.value).toBeUndefined();
});
it("selects a confirmed provider for login without authenticating automatically", async () => {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: "/accounts", component: Accounts }],
  });
  await router.push("/accounts?provider=Example%20Skin");
  await router.isReady();
  const wrapper = mount(Accounts, {
    global: { ...options.global, plugins: [router] },
  });
  await flushPromises();
  expect(wrapper.findComponent(DropdownSelect).props("modelValue")).toBe(
    "Example Skin",
  );
  expect(router.currentRoute.value.query.provider).toBeUndefined();
  expect(calls.run).not.toHaveBeenCalled();
  expect(calls.launcher.mock.calls.every(([args]) => args[2] === "list")).toBe(
    true,
  );
  wrapper.unmount();
});
it("enables browser text drag events in the packaged Windows WebView", () => {
  expect(config.app.windows[0].dragDropEnabled).toBe(false);
});
