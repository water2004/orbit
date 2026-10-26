import type { VersionPolicy } from "../types";

// Presentation-to-CLI translation only. Validation and solving stay in Orbit.
export function policyArguments(policy: VersionPolicy): string[] {
  switch (policy.kind) {
    case "any":
      return ["any"];
    case "comparison": {
      const names: Record<string, string> = {
        "=": "exact",
        ">": "greater-than",
        ">=": "at-least",
        "<": "less-than",
        "<=": "at-most",
      };
      if (!names[policy.operator])
        throw new Error("Unknown comparison operator");
      return [names[policy.operator], policy.version];
    }
    case "range":
      return [
        "range",
        policy.lower,
        policy.upper,
        "--lower-bound",
        policy.include_lower ? "inclusive" : "exclusive",
        "--upper-bound",
        policy.include_upper ? "inclusive" : "exclusive",
      ];
    case "custom":
      throw new Error("Choose a numeric policy before applying changes");
  }
}

export function importArguments(path: string, optional: string[]) {
  return [
    "import",
    path,
    "--merge-strategy",
    "prefer-import",
    ...optional.flatMap((file) => ["--optional", file]),
  ];
}

// These builders encode the public CLI grammar only. Package identity is always
// the JAR-declared mod_id; target selection is supplied separately by the page.
export function packageCommands(modId: string) {
  const command = (...args: string[]) => ["package", modId, ...args];
  return {
    versions: () => command("versions"),
    files: () => command("files"),
    activation: (enabled: boolean) => command(enabled ? "enable" : "disable"),
    environment: (value: string) => command("env", value),
    reset: () => command("reset"),
    purge: () => command("purge"),
    constraint: (policy: VersionPolicy, string: string) =>
      command(
        "constraint",
        "set",
        ...policyArguments(policy),
        "--string",
        string,
      ),
    addRemote: (provider: string, locator: string) =>
      command("remote", "add", provider, locator),
    removeRemote: (index: number) =>
      command("remote", "remove", "--index", String(index)),
  };
}

export const accountCommands = {
  default: (account?: string) =>
    account
      ? ["account", "default", "set", account]
      : ["account", "default", "clear"],
  instance: (account?: string) =>
    account
      ? ["instance", "account", "set", account]
      : ["instance", "account", "clear"],
};
