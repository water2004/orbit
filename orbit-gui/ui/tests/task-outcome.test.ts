import { expect, it } from "vitest";
import { taskOutcome } from "../src/services/task-outcome";

const install = {
  kind: "launcher",
  args: ["install"],
  label: "Install",
} as const;
it("a finite server start finishes as background-running, not an endless launch", () => {
  for (const request of [
    { kind: "launcher" as const, args: ["server", "start"], label: "Start" },
    { kind: "orbit" as const, args: ["launch", "--server"], label: "Start" },
  ]) {
    const outcome = taskOutcome(request, {
      state: { child_pid: 42, phase: "running" },
    });
    expect(outcome?.title).toContain("background");
    expect(() =>
      taskOutcome(request, { state: { child_pid: null, phase: "stopped" } }),
    ).toThrow();
  }
});
it("presents checksum absence explicitly instead of claiming upstream verification", () => {
  const request = { ...install, args: [...install.args] };
  expect(taskOutcome(request, { source_pinned_artifacts: [] })).toBeUndefined();
  const outcome = taskOutcome(request, {
    source_pinned_artifacts: ["Fabric loader"],
  })!;
  expect(outcome.detailsExpanded).toBe(true);
  expect(outcome.notices[0].title).toBe("Fabric loader");
  expect(outcome.notices[0].lines.join(" ")).toContain("SHA-256");
  expect(() =>
    taskOutcome(request, { source_pinned_artifacts: [3] }),
  ).toThrow();
});
