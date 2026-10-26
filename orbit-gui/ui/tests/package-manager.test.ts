import { beforeEach, expect, it, vi } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import { reactive } from "vue";
import { invoke } from "@tauri-apps/api/core";
import DropdownSelect from "../src/components/upstream/DropdownSelect.vue";
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (p: string) => p,
}));
vi.mock("../src/services/workspace", async () => {
  return {
    orbit: vi.fn(),
    act: (f: () => unknown) => f(),
  };
});
import PackageManager from "../src/components/PackageManager.vue";
import { orbit } from "../src/services/workspace";
import type { InstanceContext } from "../src/services/instance-context";
const mutate = vi.fn();
const context = {
  state: reactive({
    instance: { id: "instance", name: "Creative" },
    packages: [],
  }),
  orbit,
  mutate,
} as unknown as InstanceContext;
import type { Package, PackageVersions } from "../src/types";
import type { Task } from "../src/services/cli";
const pkg = {
  mod_id: "sodium",
  version: "0.9.1",
  enabled: true,
  load_status: "eligible",
  remotes: [],
  dependencies: [],
  bundled_count: 0,
} as Package;
it("shows CLI environment exclusion without treating the installed mod as disabled", async () => {
  const wrapper = mount(PackageManager, {
    props: { pkg: { ...pkg, load_status: "environment_disabled" }, context },
    global: { stubs: { Teleport: true } },
  });
  await flushPromises();
  expect(wrapper.text()).toContain("Not loaded on this side");
  expect(wrapper.text()).toContain("Enabled");
  expect(orbit).not.toHaveBeenCalled();
});
beforeEach(() => {
  vi.mocked(invoke).mockReset();
  vi.mocked(invoke).mockImplementation(async (command) =>
    command === "parse_string_rule"
      ? { initial_all: true, operations: [] }
      : "all",
  );
  vi.mocked(mutate).mockReset();
  vi.mocked(orbit).mockReset();
  HTMLDialogElement.prototype.showModal = function () {
    this.open = true;
  };
});
const versionCatalog: PackageVersions = {
  package: "sodium",
  string: "all",
  policy: { kind: "any" },
  candidates: [
    {
      version: "1.1.3+26.3",
      numeric_core: "1.1.3",
      numeric_filterable: true,
      selected: true,
      matches_constraint: true,
      details: "",
      sources: [],
    },
    {
      version: "1.1.3-hotfix.1+26.3",
      numeric_core: "1.1.3",
      numeric_filterable: true,
      selected: false,
      matches_constraint: true,
      details: "",
      sources: [],
    },
    {
      version: "1.1.2+26.3",
      numeric_core: "1.1.2",
      numeric_filterable: true,
      selected: false,
      matches_constraint: true,
      details: "",
      sources: [],
    },
    {
      version: "custom",
      numeric_filterable: false,
      selected: false,
      matches_constraint: true,
      details: "",
      sources: [],
    },
  ],
};
async function versionPage() {
  vi.mocked(orbit).mockResolvedValue(versionCatalog);
  const wrapper = mount(PackageManager, {
    props: { pkg, context },
    global: { stubs: { Teleport: true } },
  });
  await wrapper.findAll(".tabs button")[1].trigger("click");
  await flushPromises();
  return wrapper;
}
it("numeric comparisons and range boundaries use CLI numeric cores, never suffix-bearing strings", async () => {
  const wrapper = await versionPage();
  await wrapper.findAll(".item-row")[1].find("button").trigger("click");
  const boundary = wrapper
    .findAllComponents(DropdownSelect)
    .find((control) => control.props("label") === "Boundary version")!;
  expect(boundary.props("modelValue")).toBe("1.1.3");
  expect(boundary.props("options").map((option) => option.value)).toEqual([
    "1.1.3",
    "1.1.2",
  ]);
  await wrapper.findAll("footer button").at(-1)!.trigger("click");
  await flushPromises();
  expect(mutate).toHaveBeenCalledWith(
    [
      "package",
      "sodium",
      "constraint",
      "set",
      "exact",
      "1.1.3",
      "--string",
      "all",
    ],
    expect.any(String),
    expect.any(Function),
  );
  wrapper.findComponent(DropdownSelect).vm.$emit("update:modelValue", "range");
  await flushPromises();
  await wrapper.findAll(".item-row")[2].find("button").trigger("click");
  await wrapper.findAll(".item-row")[1].findAll("button")[1].trigger("click");
  await wrapper.findAll("footer button").at(-1)!.trigger("click");
  await flushPromises();
  expect(vi.mocked(mutate).mock.lastCall?.[0]).toEqual([
    "package",
    "sodium",
    "constraint",
    "set",
    "range",
    "1.1.2",
    "1.1.3",
    "--lower-bound",
    "inclusive",
    "--upper-bound",
    "inclusive",
    "--string",
    "all",
  ]);
  wrapper.unmount();
});
it.each([
  [1, "1.1.3-hotfix.1+26.3"],
  [3, "custom"],
])(
  "version %s can be pinned with an exact full-string rule",
  async (index, version) => {
    const wrapper = await versionPage();
    const row = wrapper.findAll(".item-row")[index as number];
    if (index === 3)
      expect(row.find("button").attributes("disabled")).toBeDefined();
    await row.findAll("button").at(-1)!.trigger("click");
    await wrapper.findAll("footer button").at(-1)!.trigger("click");
    await flushPromises();
    expect(invoke).toHaveBeenCalledWith("render_string_rule", {
      draft: {
        initial_all: true,
        operations: [
          {
            operator: "intersect",
            negated: false,
            predicate: "equals",
            value: version,
            case_sensitive: true,
          },
        ],
      },
    });
    expect(vi.mocked(mutate).mock.lastCall?.[0].slice(0, 5)).toEqual([
      "package",
      "sodium",
      "constraint",
      "set",
      "any",
    ]);
    wrapper.unmount();
  },
);
it("opens without fetching a version catalog and files only request ownership", async () => {
  const wrapper = mount(PackageManager, {
    props: { pkg, context },
    global: { stubs: { Teleport: true } },
  });
  expect(orbit).not.toHaveBeenCalled();
  vi.mocked(orbit).mockResolvedValue({
    mod_id: "sodium",
    artifacts: [],
    data: [],
  });
  await wrapper.findAll(".management-actions button")[2].trigger("click");
  await flushPromises();
  expect(orbit).toHaveBeenCalledTimes(1);
  expect(vi.mocked(orbit).mock.calls[0][0]).toEqual([
    "package",
    "sodium",
    "files",
  ]);
  await wrapper.find("footer button").trigger("click");
  expect(wrapper.emitted("close")).toHaveLength(1);
  wrapper.unmount();
});
it("passes a single upgrade target and renders its live progress inside the modal", async () => {
  let finish!: () => void;
  const task = reactive<Task>({
    id: 1,
    label: "Upgrade Sodium",
    state: "running",
    message: "Resolving",
    completed: 3,
    total: 8,
  });
  vi.mocked(mutate).mockImplementation((_args, _label, observe) => {
    observe?.(task);
    return new Promise<void>((resolve) => {
      finish = resolve;
    });
  });
  const wrapper = mount(PackageManager, {
    props: { pkg, context },
    global: { stubs: { Teleport: true } },
  });
  await wrapper.find(".management-actions button").trigger("click");
  expect(mutate).toHaveBeenCalledWith(
    ["upgrade", "sodium"],
    expect.any(String),
    expect.any(Function),
  );
  expect(
    wrapper.find("dialog .task-progress progress").attributes("value"),
  ).toBe("3");
  task.completed = 6;
  task.message = "Applying";
  await flushPromises();
  expect(wrapper.find("dialog .task-progress").text()).toContain("Applying");
  expect(
    wrapper.find("dialog .task-progress progress").attributes("value"),
  ).toBe("6");
  expect(
    wrapper.find(".package-controls").attributes("disabled"),
  ).toBeDefined();
  task.state = "succeeded";
  finish();
  await flushPromises();
  expect(wrapper.find("dialog .task-progress").text()).not.toContain(
    "Applying",
  );
  expect(wrapper.find("dialog .task-progress").text()).toMatch(
    /Completed|已完成/,
  );
  expect(wrapper.find("dialog .task-progress progress").exists()).toBe(false);
  wrapper.unmount();
});
it("shows a completed no-update result and its reasons, not a stalled selection screen", async () => {
  const task = reactive<Task>({
    id: 5,
    label: "Upgrading mod",
    state: "running",
    message: "Choosing sodium",
  });
  vi.mocked(mutate).mockImplementation(async (_args, _label, observe) => {
    observe?.(task);
    task.state = "succeeded";
    task.outcome = {
      title: "No feasible newer version",
      notices: [
        {
          title: "sodium · 0.10",
          lines: ["Requires a different dependency"],
        },
      ],
      omitted: 0,
      detailsExpanded: true,
    };
  });
  const wrapper = mount(PackageManager, {
    props: { pkg, context },
    global: { stubs: { Teleport: true } },
  });
  await wrapper.find(".management-actions button").trigger("click");
  await flushPromises();
  const status = wrapper.find("dialog .task-progress");
  expect(status.text()).toContain("No feasible newer version");
  expect(status.text()).toContain("Requires a different dependency");
  expect(status.text()).not.toContain("Choosing sodium");
  expect(status.find("details").attributes("open")).toBeDefined();
  expect(status.find("progress").exists()).toBe(false);
  expect(status.find("button").exists()).toBe(false);
  wrapper.unmount();
});
it("shows ownership loading and results without waiting for game startup", async () => {
  const task = reactive<Task>({
    id: 2,
    label: "Files",
    state: "running",
    message: "Reading ledger",
  });
  let finish!: (data: unknown) => void;
  vi.mocked(orbit).mockImplementation((_args, _label, _instance, observe) => {
    observe?.(task);
    return new Promise((resolve) => {
      finish = resolve;
    });
  });
  const wrapper = mount(PackageManager, {
    props: { pkg, context },
    global: { stubs: { Teleport: true } },
  });
  await wrapper.findAll(".management-actions button")[2].trigger("click");
  expect(wrapper.find("dialog .task-progress").text()).toContain(
    "Reading ledger",
  );
  task.state = "succeeded";
  finish({
    artifacts: [],
    data: [{ path: "config/sodium.json", kind: "file", preserved: [] }],
  });
  await flushPromises();
  expect(wrapper.text()).toContain("config/sodium.json");
  wrapper.unmount();
});
