import { reactive } from "vue";
import type { Package, SearchResult } from "../types";

// Presentation state only. Catalog/compatibility and all mutations stay in CLI.
export interface DiscoveryState {
  scope: string;
  query: string;
  results: SearchResult[];
  loading: boolean;
  searched: boolean;
  revision: number;
  hideInstalled: boolean;
  recommended: boolean;
  environment: string;
  optional: boolean;
  installing: string | null;
  error: string;
}
const sessions = new Map<string, DiscoveryState>();
export function discoverySession(id?: string) {
  const key = id ?? "global";
  let state = sessions.get(key);
  if (!state) {
    state = reactive<DiscoveryState>({
      scope: "",
      query: "",
      results: [],
      loading: false,
      searched: false,
      revision: 0,
      hideInstalled: false,
      recommended: true,
      environment: "auto",
      optional: false,
      installing: null,
      error: "",
    });
    sessions.set(key, state);
  }
  return state;
}
export function projectKey(project: SearchResult) {
  return `${project.platform}:${project.project_id}`;
}
export function installedPackages(project: SearchResult, packages: Package[]) {
  return packages.filter((pkg) => pkg.remotes.includes(projectKey(project)));
}
export function addArguments(
  project: SearchResult,
  options: Pick<DiscoveryState, "recommended" | "environment" | "optional">,
) {
  return [
    "add",
    `${project.platform === "modrinth" ? "mr" : "cf"}:${project.project_id}`,
    ...(options.recommended
      ? [
          "--string",
          'all; intersect not contains(i"beta"); intersect not contains(i"snapshot")',
        ]
      : []),
    ...(options.environment !== "auto" ? ["--env", options.environment] : []),
    ...(options.optional ? ["--optional"] : []),
  ];
}
