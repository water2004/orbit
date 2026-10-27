import { beforeEach, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("../src/services/workspace", async () => {
  const { reactive } = await import("vue");
  return {
    workspace: reactive({
      preferences: {
        close_after_launch: false,
        language: "en",
        theme_mode: "light",
        accent_theme: "indigo",
      },
    }),
    orbit: vi.fn(),
    launcher: vi.fn(),
    act: (fn: () => unknown) => fn(),
  };
});
import Settings from "../src/pages/Settings.vue";
import DropdownSelect from "../src/components/upstream/DropdownSelect.vue";
import { orbit, launcher, workspace } from "../src/services/workspace";
import { invoke } from "@tauri-apps/api/core";
beforeEach(() => {
  workspace.preferences!.close_after_launch = false;
  vi.mocked(invoke).mockClear();
  vi.mocked(orbit).mockResolvedValue({
    entries: [
      {
        key: "ui.progress-bar",
        value: "modern",
        allowed_values: ["modern", "plain", "off"],
      },
      { key: "cache.dir", value: null, allowed_values: [] },
      { key: "repository.dir", value: null, allowed_values: [] },
      {
        key: "auth.modrinth-token",
        value: "<redacted>",
        sensitive: true,
        allowed_values: [],
      },
    ],
    resolved_paths: {
      cache_directory: "X:/default/cache",
      repository_directory: "X:/default/repository",
    },
  });
  vi.mocked(launcher).mockImplementation(async (args) =>
    args[0] === "minecraft"
      ? { directory: "X:/minecraft" }
      : {
          settings: [
            {
              key: "ui.progress-bar",
              value: "auto",
              allowed_values: ["auto", "always", "never"],
            },
          ],
        },
  );
});

it("keeps the launch preference text in a flexible label and saves through the existing preference API", async () => {
  const wrapper = mount(Settings);
  await flushPromises();
  await wrapper
    .findAll(".settings-nav button")
    .find((button) => button.text() === "Launcher")!
    .trigger("click");
  const toggle = wrapper.find('[role="switch"]');
  const title = wrapper.find(`#${toggle.attributes("aria-labelledby")}`);
  expect(title.text()).toBe("Close Launcher after the game starts");
  expect(title.element.parentElement?.classList.contains("setting-label")).toBe(
    true,
  );
  expect(
    wrapper.find(`#${toggle.attributes("aria-describedby")}`).text(),
  ).toContain("The game keeps running");
  expect(toggle.attributes("aria-checked")).toBe("false");
  await toggle.trigger("click");
  await flushPromises();
  expect(invoke).toHaveBeenCalledWith("save_preferences", {
    value: expect.objectContaining({ close_after_launch: true }),
  });
  expect(toggle.attributes("aria-checked")).toBe("true");
  wrapper.unmount();
});
it("renders canonical choices from each CLI and resolved default locations without revealing credentials", async () => {
  const wrapper = mount(Settings);
  await flushPromises();
  await wrapper
    .findAll(".settings-nav button")
    .find((button) => button.text() === "Orbit")!
    .trigger("click");
  const select = wrapper.findComponent(DropdownSelect);
  expect(
    select.props("options").map((option: { value: string }) => option.value),
  ).toEqual(["modern", "plain", "off"]);
  expect(select.props("modelValue")).toBe("modern");
  expect(wrapper.find('input[placeholder="X:/default/cache"]').exists()).toBe(
    true,
  );
  expect(
    wrapper.find('input[placeholder="X:/default/repository"]').exists(),
  ).toBe(true);
  expect(wrapper.find("input[type=password]").element.value).toBe("");
  expect(wrapper.html()).not.toContain("&lt;redacted&gt;");
  await wrapper
    .findAll(".settings-nav button")
    .find((button) => button.text() === "Launcher")!
    .trigger("click");
  expect(
    wrapper
      .findComponent(DropdownSelect)
      .props("options")
      .map((option: { value: string }) => option.value),
  ).toEqual(["auto", "always", "never"]);
  wrapper.unmount();
});
