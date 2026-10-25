import { t } from "../i18n";
import type { Request } from "./cli";

export interface TaskOutcome {
  title: string;
  notices: { title: string; lines: string[] }[];
  omitted: number;
  detailsExpanded: boolean;
}

// Presentation of the CLI's final transaction, never a second update check.
export function taskOutcome(
  request: Request,
  result: unknown,
): TaskOutcome | undefined {
  if (request.kind !== "orbit" || request.args[0] !== "upgrade") return;
  const invalid = () =>
    new Error(
      t(
        "CLI returned an invalid upgrade result. No update status can be displayed.",
        "CLI 返回的升级结果无效，无法显示升级结论。",
      ),
    );
  if (!result || typeof result !== "object") throw invalid();
  const report = result as {
    changes?: unknown[];
    diagnostics?: {
      package: string;
      candidate_version: string;
      facts: string[];
    }[];
    warnings?: string[];
  };
  if (
    !Array.isArray(report.changes) ||
    !Array.isArray(report.diagnostics) ||
    !Array.isArray(report.warnings)
  )
    throw invalid();
  if (
    !report.diagnostics.every(
      (diagnostic) =>
        diagnostic &&
        typeof diagnostic.package === "string" &&
        typeof diagnostic.candidate_version === "string" &&
        Array.isArray(diagnostic.facts) &&
        diagnostic.facts.every((fact) => typeof fact === "string"),
    ) ||
    !report.warnings.every((warning) => typeof warning === "string")
  )
    throw invalid();
  const target = request.args[1];
  const title = report.changes.length
    ? t(
        `Update completed · ${report.changes.length} package changes`,
        `升级完成 · ${report.changes.length} 项包变更`,
      )
    : t(
        "No feasible newer version under the current constraints",
        "当前约束下没有可行的更高版本",
      );
  const notices = [
    ...report.diagnostics.map((diagnostic) => ({
      title: `${diagnostic.package} · ${diagnostic.candidate_version}`,
      lines: diagnostic.facts,
    })),
    ...report.warnings.map((warning) => ({
      title: t("Notice", "提示"),
      lines: [warning],
    })),
  ];
  return {
    title: target ? `${target} · ${title}` : title,
    notices: notices.slice(0, 16),
    omitted: Math.max(0, notices.length - 16),
    detailsExpanded: report.changes.length === 0,
  };
}
