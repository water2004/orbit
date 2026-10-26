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
  const serverStart =
    request.kind === "launcher"
      ? request.args[0] === "server" && request.args[1] === "start"
      : request.args[0] === "launch" && request.args.includes("--server");
  if (serverStart) {
    const state = (
      result as { state?: { child_pid?: unknown; phase?: unknown } } | null
    )?.state;
    if (
      !state ||
      typeof state.child_pid !== "number" ||
      state.child_pid <= 0 ||
      state.phase !== "running"
    )
      throw new Error(
        t(
          "CLI returned an invalid server startup result.",
          "CLI 返回的服务端启动结果无效。",
        ),
      );
    return {
      title: t("Server started in the background", "服务端已在后台启动"),
      notices: [],
      omitted: 0,
      detailsExpanded: false,
    };
  }
  if (request.kind === "launcher" && request.args[0] === "install") {
    const artifacts = (result as { source_pinned_artifacts?: unknown } | null)
      ?.source_pinned_artifacts;
    if (
      !Array.isArray(artifacts) ||
      !artifacts.every((item) => typeof item === "string")
    )
      throw new Error(
        t(
          "CLI returned an invalid installation integrity report.",
          "CLI 返回的安装完整性报告无效。",
        ),
      );
    if (!artifacts.length) return;
    return {
      title: t(
        "Installation completed with source-trusted artifacts",
        "安装完成，部分文件依赖下载源信任",
      ),
      notices: artifacts.slice(0, 16).map((artifact) => ({
        title: artifact,
        lines: [
          t(
            "No upstream checksum was supplied. This file is pinned locally by SHA-256, not verified against an upstream checksum.",
            "来源未提供上游摘要。此文件已用本地 SHA-256 固定内容，但未经上游摘要校验。",
          ),
        ],
      })),
      omitted: Math.max(0, artifacts.length - 16),
      detailsExpanded: true,
    };
  }
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
