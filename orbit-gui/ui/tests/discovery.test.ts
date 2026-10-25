import { expect, it } from "vitest";
import {
  addArguments,
  discoverySession,
  installedPackages,
} from "../src/services/discovery";
import { instancePath } from "../src/services/navigation";
import type { Package, SearchResult } from "../src/types";
const project = {
  platform: "modrinth",
  project_id: "AANobbMI",
  name: "Sodium",
} as SearchResult;
it("retains catalog state per instance, not in a global selected-installation slot", () => {
  const a = discoverySession("A"),
    b = discoverySession("B");
  a.query = "sodium";
  a.installing = "modrinth:AANobbMI";
  expect(discoverySession("A")).toBe(a);
  expect(b.query).toBe("");
  expect(b.installing).toBeNull();
  expect(instancePath("a/b", "browse")).toBe("/instance/a%2Fb/browse");
});
it("detects installed UI state by explicit remotes, never mod ID / slug guesses", () => {
  const pkg = {
    mod_id: "not-the-project-id",
    remotes: ["modrinth:AANobbMI"],
  } as Package;
  expect(installedPackages(project, [pkg])).toEqual([pkg]);
  expect(
    installedPackages(project, [
      { ...pkg, mod_id: "sodium", remotes: ["file:local.jar"] },
    ]),
  ).toEqual([]);
});
it("unchecked defaults produce no hidden constraint flags", () => {
  expect(
    addArguments(project, {
      recommended: false,
      environment: "auto",
      optional: false,
    }),
  ).toEqual(["add", "mr:AANobbMI"]);
  expect(
    addArguments(project, {
      recommended: true,
      environment: "server",
      optional: true,
    }),
  ).toEqual([
    "add",
    "mr:AANobbMI",
    "--string",
    'all; intersect not contains(i"beta"); intersect not contains(i"snapshot")',
    "--env",
    "server",
    "--optional",
  ]);
});
