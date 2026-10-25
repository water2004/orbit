export interface DiagnosticDocument {
  path: string;
  text: string;
  truncated: boolean;
}
export interface LaunchDiagnostics {
  reports: DiagnosticDocument[];
  log_tail?: DiagnosticDocument | null;
  warnings: string[];
}
export function launchDiagnostics(
  result: unknown,
): LaunchDiagnostics | undefined {
  if (!result || typeof result !== "object") return;
  const value = (result as { diagnostics?: LaunchDiagnostics }).diagnostics;
  if (!value) return;
  const document = (item: DiagnosticDocument) =>
    item &&
    typeof item.path === "string" &&
    typeof item.text === "string" &&
    typeof item.truncated === "boolean";
  if (
    !Array.isArray(value.reports) ||
    !value.reports.every(document) ||
    (value.log_tail && !document(value.log_tail)) ||
    !Array.isArray(value.warnings) ||
    !value.warnings.every((item) => typeof item === "string")
  )
    throw new Error("Invalid launch diagnostics returned by Launcher");
  return value;
}
