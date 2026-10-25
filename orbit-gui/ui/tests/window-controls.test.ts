import { beforeEach, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";
const controls = vi.hoisted(() => ({
  isMaximized: vi.fn(),
  minimize: vi.fn(),
  toggleMaximize: vi.fn(),
  close: vi.fn(),
}));
vi.mock("@tauri-apps/api/core", () => ({ isTauri: () => true }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => controls }));
vi.mock("../src/services/workspace", () => ({
  act: (fn: () => Promise<unknown>) => fn(),
}));
import WindowControls from "../src/components/WindowControls.vue";
beforeEach(() => {
  for (const fn of Object.values(controls)) fn.mockReset();
  controls.isMaximized.mockResolvedValue(false);
});
it("uses native window controls and reflects maximize/restore state", async () => {
  const wrapper = mount(WindowControls);
  await flushPromises();
  const buttons = wrapper.findAll("button");
  await buttons[0].trigger("click");
  expect(controls.minimize).toHaveBeenCalledOnce();
  controls.isMaximized.mockResolvedValue(true);
  await buttons[1].trigger("click");
  await flushPromises();
  expect(controls.toggleMaximize).toHaveBeenCalledOnce();
  expect(buttons[1].attributes("aria-label")).toMatch(/Restore|还原/);
  await buttons[2].trigger("click");
  expect(controls.close).toHaveBeenCalledOnce();
  wrapper.unmount();
});
