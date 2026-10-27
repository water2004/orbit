import { beforeEach, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/app", () => ({
  getVersion: vi.fn().mockResolvedValue("0.9.0"),
}));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
vi.mock("../src/services/workspace", () => ({
  launcher: vi.fn(),
  orbit: vi.fn(),
  act: (fn: () => unknown) => fn(),
}));
import About from "../src/components/About.vue";
import { launcher } from "../src/services/workspace";
import { openUrl } from "@tauri-apps/plugin-opener";
const release = {
  version: "0.10.0",
  name: "Orbit 0.10.0",
  url: "https://github.com/water2004/orbit/releases/tag/v0.10.0",
  published_at: "2026-10-27T00:00:00Z",
  notes: "<script>alert(1)</script>\nFixes and improvements",
};
beforeEach(() => {
  vi.clearAllMocks();
});
function checkButton(wrapper: ReturnType<typeof mount>) {
  return wrapper
    .findAll("button")
    .find((b) => b.text().includes("Check for updates"))!;
}
it("shows the app version without a background network check and delegates comparison to the CLI", async () => {
  vi.mocked(launcher).mockResolvedValue({
    current_version: "0.9.0",
    status: "update_available",
    release,
  });
  const wrapper = mount(About);
  await flushPromises();
  expect(wrapper.text()).toContain("0.9.0");
  expect(launcher).not.toHaveBeenCalled();
  await checkButton(wrapper).trigger("click");
  await flushPromises();
  expect(launcher).toHaveBeenCalledWith(
    ["update", "check", "--current-version", "0.9.0"],
    expect.any(String),
    null,
    expect.any(Function),
  );
  expect(wrapper.text()).toContain("A new version is available");
  expect(wrapper.find("script").exists()).toBe(false);
  expect(wrapper.find(".about-notes").text()).toContain("<script>");
  await wrapper
    .findAll("button")
    .find((b) => b.text().includes("Download update"))!
    .trigger("click");
  expect(openUrl).toHaveBeenCalledWith(release.url);
  wrapper.unmount();
});
it("does not turn a failed recheck into up-to-date or retain a stale download offer", async () => {
  vi.mocked(launcher)
    .mockResolvedValueOnce({
      current_version: "0.9.0",
      status: "update_available",
      release,
    })
    .mockRejectedValueOnce(new Error("HTTP 403"));
  const wrapper = mount(About);
  await flushPromises();
  await checkButton(wrapper).trigger("click");
  await flushPromises();
  await checkButton(wrapper).trigger("click");
  await flushPromises();
  expect(wrapper.find('[role="alert"]').text()).toContain("HTTP 403");
  expect(wrapper.text()).not.toContain("You're up to date");
  expect(wrapper.text()).not.toContain("Download update");
  wrapper.unmount();
});
it.each([
  ["up_to_date", "You're up to date"],
  ["ahead", "You're using a newer build"],
  ["no_release", "No release has been published yet"],
])("renders %s distinctly", async (status, label) => {
  vi.mocked(launcher).mockResolvedValue({
    current_version: "0.9.0",
    status,
    release: status === "no_release" ? null : release,
  });
  const wrapper = mount(About);
  await flushPromises();
  await checkButton(wrapper).trigger("click");
  await flushPromises();
  expect(wrapper.text()).toContain(label);
  expect(wrapper.text()).not.toContain("Download update");
  wrapper.unmount();
});
it("prevents overlapping checks", async () => {
  let finish!: (result: unknown) => void;
  vi.mocked(launcher).mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const wrapper = mount(About);
  await flushPromises();
  await checkButton(wrapper).trigger("click");
  expect(checkButton(wrapper).attributes("disabled")).toBeDefined();
  await checkButton(wrapper).trigger("click");
  expect(launcher).toHaveBeenCalledTimes(1);
  finish({ current_version: "0.9.0", status: "no_release", release: null });
  await flushPromises();
  wrapper.unmount();
});
