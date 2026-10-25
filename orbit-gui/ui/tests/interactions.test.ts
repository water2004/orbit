import { beforeEach, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import { defineComponent, reactive } from "vue";
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  Channel: class {},
  convertFileSrc: (p: string) => p,
}));
import { activity } from "../src/services/cli";
import ActivityPanel from "../src/components/ActivityPanel.vue";
const Modal = defineComponent({
  template:
    '<section class="dialog"><slot/><footer><slot name="footer"/></footer></section>',
});
beforeEach(() => activity.tasks.splice(0));
function question(kind: string, id: string) {
  const change = {
    package: "sodium",
    kind: "upgrade",
    current_version: "1",
    selected_version: "2",
    selected_artifact: "sodium-client.jar",
  };
  return {
    interaction_id: id,
    interaction: kind,
    prompt: "Review",
    choices: [
      {
        id: "apply",
        label: "Apply plan",
        data: {
          changes:
            kind === "resolution" ? [{ different: true, change }] : [change],
        },
      },
      { id: "cancel", label: "Cancel", data: {} },
    ],
    allow_cancel: true,
  };
}
it("shows only choices in each factor and confirmation only at the final step", async () => {
  const task = reactive({
    id: 1,
    label: "Upgrade",
    state: "waiting" as const,
    message: "",
    interaction: question("resolution", "factor-1"),
  });
  activity.tasks.push(task);
  const wrapper = mount(ActivityPanel, {
    global: { stubs: { Modal, Teleport: true } },
  });
  expect(wrapper.get(".dialog footer").text()).not.toContain("Apply plan");
  expect(wrapper.get(".dialog").text()).toContain("sodium-client.jar");
  task.interaction = question("resolution", "factor-2");
  await wrapper.vm.$nextTick();
  expect(wrapper.get(".dialog footer").text()).not.toContain("Apply plan");
  task.interaction = question("confirmation", "commit");
  await wrapper.vm.$nextTick();
  expect(wrapper.get(".dialog footer").text()).toContain("Apply plan");
  expect(wrapper.get(".dialog").text()).toContain("sodium-client.jar");
  wrapper.unmount();
});
it("displays the complete EULA before offering explicit acceptance", () => {
  activity.tasks.push({
    id: 3,
    label: "Install server",
    state: "waiting",
    message: "",
    interaction: {
      interaction_id: "eula",
      interaction: "confirmation",
      prompt: "Review the Minecraft EULA",
      choices: [
        {
          id: "accept",
          label: "I agree",
          data: {
            document: {
              text: "First paragraph\n\nLast paragraph",
              url: "https://www.minecraft.net/eula",
              digest_sha256: "digest",
            },
          },
        },
        { id: "decline", label: "Decline", data: {} },
      ],
      allow_cancel: true,
    },
  });
  const wrapper = mount(ActivityPanel, {
    global: { stubs: { Modal, Teleport: true } },
  });
  expect(wrapper.get(".legal-document").text()).toBe(
    "First paragraph\n\nLast paragraph",
  );
  expect(wrapper.get(".dialog footer").text()).toContain("I agree");
  expect(wrapper.get(".dialog footer").text()).toContain("Decline");
  wrapper.unmount();
});
it("renders empty resolution lists safely without warnings or diagnostics counts", () => {
  activity.tasks.push({
    id: 2,
    label: "Fix",
    state: "waiting",
    message: "",
    interaction: {
      interaction_id: "x",
      interaction: "resolution",
      prompt: "Choose",
      choices: [],
      allow_cancel: true,
    },
  });
  const wrapper = mount(ActivityPanel, {
    global: { stubs: { Modal, Teleport: true } },
  });
  expect(wrapper.text()).not.toContain("warnings");
  expect(wrapper.text()).not.toContain("diagnostics");
  wrapper.unmount();
});
