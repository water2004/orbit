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
