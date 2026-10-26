import { expect, it } from "vitest";
import { taskOutcome } from "../src/services/task-outcome";

const install = {
  kind: "launcher",
  args: ["install"],
  label: "Install",
} as const;
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
