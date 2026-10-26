export interface Instance {
  id: string;
  name: string;
  directory: string;
  kind: string;
  is_default: boolean;
  minecraft_directory?: string;
  icon_path?: string | null;
  game_version?: string | null;
  loader?: string | null;
  launch_account?: LaunchAccount | null;
}
export type LaunchAccount =
  | { state: "not-configured" }
  | { state: "unavailable"; account_id: string; source: "instance" | "global" }
  | {
      state: "selected";
      source: "instance" | "global";
      account: {
        id: string;
        profile_name: string;
        avatar_path?: string | null;
        provider: { kind: string; provider_id?: string };
        authentication_state: string;
      };
    };
export interface Runtime {
  minecraft: string;
  loader: string;
  loader_version?: string;
  java?: { major: number; version: string; provider: string };
}
export interface MemoryLimits {
  min_mib: number;
  max_mib: number;
}
export interface MemoryCapabilities {
  physical_memory_mib: number;
  slider_max_mib: number;
}
export interface MemorySettingsResult {
  instance_id: string;
  name: string;
  memory: MemoryLimits;
  capabilities: MemoryCapabilities;
}
export interface Detail extends Instance {
  memory: MemoryLimits;
  desired: Runtime;
  installed?: Runtime;
  selected_account_id?: string;
  client_resolution?: { width: number; height: number };
  java_selection?: string | null;
}
export interface Package {
  mod_id: string;
  version: string;
  icon_path?: string;
  enabled: boolean;
  optional: boolean;
  environment: string;
  configured_environment?: string;
  load_status: "eligible" | "disabled" | "environment_disabled";
  remotes: string[];
  dependencies: string[];
  bundled_count: number;
}
export type VersionPolicy =
  | { kind: "any" }
  | { kind: "comparison"; operator: string; version: string }
  | {
      kind: "range";
      lower: string;
      upper: string;
      include_lower: boolean;
      include_upper: boolean;
    }
  | { kind: "custom"; requirement: string };
export interface VersionCandidate {
  version: string;
  selected: boolean;
  matches_constraint: boolean;
  details: string;
  sources: string[];
  numeric_core?: string;
  numeric_filterable: boolean;
  numeric_error?: string;
}
export interface PackageVersions {
  package: string;
  string: string;
  policy: VersionPolicy;
  selected_version?: string;
  candidates: VersionCandidate[];
}
export interface Ownership {
  mod_id: string;
  artifacts: { path: string; scope: string; present: boolean }[];
  data: {
    path: string;
    scope: string;
    kind: string;
    preserved: { path: string; scope: string }[];
  }[];
}
export interface StringOperation {
  operator: "intersect" | "union" | "complement";
  negated: boolean;
  predicate:
    "empty" | "present" | "equals" | "contains" | "starts_with" | "ends_with";
  value: string | null;
  case_sensitive: boolean;
}
export interface StringRule {
  initial_all: boolean;
  operations: StringOperation[];
}
export interface SearchResult {
  name: string;
  project_id: string;
  platform: string;
  description: string;
  icon_url?: string;
  latest_version: string;
  downloads: number;
  categories: string[];
  compatible?: boolean;
}
export interface Account {
  id: string;
  profile_name: string;
  avatar_path?: string;
  provider: string;
  provider_id?: string;
  is_default: boolean;
  authentication_state: string;
  login_name?: string;
}
export interface MinecraftVersion {
  id: string;
  version_type: string;
}
export interface LoaderVersion {
  version: string;
  stable?: boolean;
}
export interface Change {
  package: string;
  kind: string;
  current_version?: string;
  selected_version?: string;
  selected_artifact?: string;
  selected_description?: string;
  different?: boolean;
}
export interface FeatureSupport {
  available: boolean;
  reason?: string;
}
export interface InstanceFeatureSupport {
  audit: FeatureSupport;
  runtime_observation: FeatureSupport;
}
