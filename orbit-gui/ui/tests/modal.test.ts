import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import Modal from "../src/components/Modal.vue";
import ActivityPanel from "../src/components/ActivityPanel.vue";
import { activity } from "../src/services/cli";
import { invoke } from "@tauri-apps/api/core";
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockResolvedValue(undefined),
  Channel: class {},
}));
beforeEach(() => {
  HTMLDialogElement.prototype.showModal = function () {
    this.open = true;
  };
  activity.tasks.splice(0);
  vi.mocked(invoke).mockClear();
});
afterEach(() => {
  document.body.innerHTML = "";
});
it("ordinary dialogs default to closable and expose an accessible close button", async () => {
  const wrapper = mount(Modal, {
    props: { title: "Settings" },
    attachTo: document.body,
  });
  const close = document.querySelector<HTMLButtonElement>(
    "dialog header button",
  )!;
  expect(close).not.toBeNull();
  expect(close.getAttribute("aria-label")).toBeTruthy();
  close.click();
  expect(wrapper.emitted("close")).toHaveLength(1);
  document
    .querySelector("dialog")!
    .dispatchEvent(new Event("cancel", { cancelable: true }));
  expect(wrapper.emitted("close")).toHaveLength(2);
  wrapper.unmount();
});
it("a deliberately locked dialog blocks Escape and backdrop close", () => {
  const wrapper = mount(Modal, {
    props: { title: "Sending", closable: false },
  });
  const dialog = document.querySelector("dialog")!;
  expect(dialog.querySelector("header button")).toBeNull();
  dialog.dispatchEvent(new Event("cancel", { cancelable: true }));
  dialog.click();
  expect(wrapper.emitted("close")).toBeUndefined();
  wrapper.unmount();
});
it("closing a final confirmation sends cancellation, not approval or a hidden pending process", async () => {
  activity.tasks.push({
    id: 9,
    label: "Upgrade",
    state: "waiting",
    message: "",
    interaction: {
      interaction_id: "final",
      interaction: "confirmation",
      prompt: "Review changes",
      allow_cancel: true,
      choices: [{ id: "proceed", label: "Apply changes", data: {} }],
    },
  });
  const wrapper = mount(ActivityPanel, { attachTo: document.body });
  expect(document.querySelector("dialog footer")?.textContent).toContain(
    "Apply changes",
  );
  document.querySelector<HTMLButtonElement>("dialog header button")!.click();
  await vi.waitFor(() =>
    expect(invoke).toHaveBeenCalledWith("answer_cli", {
      task: 9,
      interaction: "final",
      choice: null,
    }),
  );
  wrapper.unmount();
});
