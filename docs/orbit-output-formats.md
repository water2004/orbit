# Orbit 输出格式

> 本文约定 `--format` 全局选项、JSON 结果 schema、NDJSON 进度协议和结构化错误。
> 命令语义见 [orbit-cli-commands.md](orbit-cli-commands.md)。

## 1. 全局选项

```text
orbit [--format text|json] [--progress-format none|ndjson] <command> ...
```

| 选项 | 取值 | 默认 | 作用 |
|------|------|------|------|
| `--format` | `text` / `json` | `text` | 结果的渲染格式。`text` 走自适应表格/文本；`json` 输出单个 JSON 文档到 stdout |
| `--progress-format` | `none` / `ndjson` | `none`（当 `--format json`）/ 由 `ui.progress_bar` 决定（当 `--format text`） | 进度的协议。`ndjson` 把进度事件逐行写 stderr，每行一个 JSON 对象 |

`--format json` 隐含 `--progress-format none`，除非显式传 `--progress-format ndjson`。
`--quiet` 始终关闭进度，等价 `--progress-format none`。

### stdout / stderr 分工

| 流 | `--format text` | `--format json` |
|----|-----------------|-----------------|
| stdout | 表格/文本结果 | 单个 JSON 文档（结果） |
| stderr | 进度条/交互提示/警告 | NDJSON 进度（若启用）+ 结构化错误 |

**关键约束**：`--format json` 下 stdout 永远是且只是一个完整 JSON 文档（成功时是结果，失败时为空）；调用方可以安全 `orbit --format json ... | jq`。

## 2. JSON 结果 schema 通用约定

每个命令的结果是一个顶层 JSON 对象，固定包含：

| 字段 | 类型 | 说明 |
|------|------|------|
| `schema_version` | number | 本命令结果的 schema 版本，从 1 开始，破坏性变更才递增 |
| `command` | string | 命令名（如 `"search"`、`"install"`） |
| `ok` | boolean | `true` |
| `result` | object | 命令特定结果体 |

示例：

```json
{
  "schema_version": 1,
  "command": "search",
  "ok": true,
  "result": { ... }
}
```

所有命令的 `result` 体在 §3 逐命令定义。字段命名遵守 snake_case；时间戳为 ISO 8601 字符串；缺省可选字段省略而非输出 `null`。

## 3. 逐命令 JSON 结果

### `search`

```json
{
  "schema_version": 1,
  "command": "search",
  "ok": true,
  "result": {
    "query": "sodium",
    "platforms": ["modrinth"],
    "filters": { "mc_version": "1.21", "modloader": "fabric" },
    "ref_mc_version": "1.21",
    "results": [
      {
        "slug": "sodium",
        "name": "Sodium",
        "project_id": "AANobbMI",
        "platform": "modrinth",
        "description": "Rendering engine replacement",
        "downloads": 1500000,
        "mc_versions": ["1.20", "1.21"],
        "compatible": true
      }
    ],
    "truncated": false
  }
}
```

- `ref_mc_version` 缺省时为 `null`；`compatible` 仅在 `ref_mc_version` 非 null 时存在，否则为 `null`。
- `truncated`：是否因 `--limit` 截断了结果。

### `info`

```json
{
  "schema_version": 1,
  "command": "info",
  "ok": true,
  "result": {
    "provider": "modrinth",
    "project_id": "AANobbMI",
    "slug": "sodium",
    "name": "Sodium",
    "description": "Rendering engine replacement",
    "authors": ["jellysquid3"],
    "latest_version": "0.5.8",
    "downloads": 1500000,
    "license": "MIT",
    "client_side": "required",
    "server_side": "unsupported",
    "categories": ["optimization"],
    "recent_versions": [
      { "version": "0.5.8", "mc_versions": ["1.21"], "loader": "fabric", "released_at": "2024-06-01T00:00:00Z" }
    ],
    "dependencies": [
      { "slug": "fabric-api", "project_id": "P7dR8mSH", "required": true }
    ]
  }
}
```

### `list`

```json
{
  "schema_version": 1,
  "command": "list",
  "ok": true,
  "result": {
    "target": null,
    "tree": false,
    "packages": [
      {
        "mod_id": "sodium",
        "version": "0.5.8",
        "remotes": ["modrinth:AANobbMI"],
        "environment": "both",
        "optional": false,
        "dependencies": ["fabric-api"],
        "bundled": [{ "mod_id": "sodium-base", "version": "0.5.8" }]
      }
    ]
  }
}
```

`tree: true` 时额外返回 `roots` 与每个包的 `dependents`，结构见 schema 文档源码。

### `check`

```json
{
  "schema_version": 1,
  "command": "check",
  "ok": true,
  "result": {
    "target_mc_version": "1.21",
    "target_loader": "fabric",
    "summary": { "total": 12, "compatible": 10, "blocking": 2 },
    "results": [
      {
        "mod_name": "sodium",
        "current_version": "0.5.8",
        "provider": "modrinth",
        "compatible": true,
        "available_version": "0.5.8"
      },
      {
        "mod_name": "voxy",
        "current_version": "1.0",
        "provider": "modrinth",
        "compatible": false,
        "available_version": null
      }
    ]
  }
}
```

### `outdated`

```json
{
  "schema_version": 1,
  "command": "outdated",
  "ok": true,
  "result": {
    "package": null,
    "summary": { "upgrades": 3, "up_to_date": 0 },
    "updates": [
      { "mod_id": "sodium", "current_version": "0.5.7", "new_version": "0.5.8" }
    ],
    "diagnostics": [
      {
        "package": "voxy",
        "selected_version": "1.0",
        "candidate_version": "2.0",
        "kind": "excluded_by_propagation",
        "facts": ["voxy 2.0 requires sodium =0.8.9"]
      }
    ],
    "warnings": []
  }
}
```

`kind` 枚举：`no_compatible_candidate` / `excluded_by_propagation` / `backtracked` / `unexplained`。
`summary.up_to_date` 为 1 表示无更新且无诊断，否则 0。

### `instances`

`instances list`：

```json
{
  "schema_version": 1,
  "command": "instances",
  "ok": true,
  "result": {
    "subcommand": "list",
    "instances": [
      {
        "name": "alpha",
        "path": "/home/u/alpha",
        "mc_version": "1.21",
        "modloader": "fabric",
        "is_default": true,
        "is_current": false
      }
    ]
  }
}
```

`instances default <name>`：

```json
{
  "schema_version": 1,
  "command": "instances",
  "ok": true,
  "result": { "subcommand": "default", "name": "alpha" }
}
```

`instances remove <name>`：

```json
{
  "schema_version": 1,
  "command": "instances",
  "ok": true,
  "result": { "subcommand": "remove", "name": "alpha" }
}
```

### `remote list`

```json
{
  "schema_version": 1,
  "command": "remote",
  "ok": true,
  "result": {
    "subcommand": "list",
    "package": "sodium",
    "changed": false,
    "remotes": [
      { "index": 1, "provider": "modrinth", "locator": "modrinth:AANobbMI" }
    ]
  }
}
```

`remote add` / `remote remove` 使用相同 schema，`changed` 标记是否修改了 manifest。

### `sync`

```json
{
  "schema_version": 1,
  "command": "sync",
  "ok": true,
  "result": {
    "dry_run": false,
    "summary": {
      "platform_changes": 1,
      "added": 1,
      "changed": 1,
      "removed": 0,
      "missing": 1,
      "unlocked": 0
    },
    "platform_changes": [
      { "field": "mc_version", "previous": "1.20", "current": "1.21" }
    ],
    "added": ["sodium"],
    "changed": ["lithium"],
    "missing": ["fabric-api"],
    "unlocked": [],
    "removed": [],
    "diagnostics": [],
    "warnings": []
  }
}
```

### `install` / `add` / `upgrade`

共用事务报告 schema：

```json
{
  "schema_version": 1,
  "command": "install",
  "ok": true,
  "result": {
    "dry_run": false,
    "summary": {
      "installed": 3,
      "removed": 1,
      "already_satisfied": 0,
      "skipped_optional": 0
    },
    "changes": [
      {
        "package": "sodium",
        "kind": "upgrade",
        "current_version": "0.5.7",
        "selected_version": "0.5.8",
        "selected_description": "Modrinth project AANobbMI, release mc1.21"
      }
    ],
    "installed": [
      { "mod_id": "sodium", "version": "0.5.8" }
    ],
    "removed": [
      { "mod_id": "voxy", "version": "1.0" }
    ],
    "already_satisfied": [],
    "diagnostics": [],
    "warnings": []
  }
}
```

`changes[].kind` 枚举：`install` / `upgrade` / `downgrade` / `replace` / `remove`。
内容哈希、物理 JAR 文件名、provider 密钥永不进入 JSON，与 text 表格一致（CLAUDE.md #41/#50）。

### `remove` / `purge`

```json
{
  "schema_version": 1,
  "command": "remove",
  "ok": true,
  "result": {
    "mod_id": "sodium",
    "jar_deleted": true
  }
}
```

`purge` 额外含 `configs_removed` 数组（路径字符串）。

### `import` / `export`

```json
{
  "schema_version": 1,
  "command": "import",
  "ok": true,
  "result": {
    "dry_run": false,
    "added": ["sodium"],
    "merged": ["fabric-api"],
    "replaced": [],
    "kept": ["lithium"],
    "extracted": []
  }
}
```

```json
{
  "schema_version": 1,
  "command": "export",
  "ok": true,
  "result": {
    "dry_run": false,
    "path": "/tmp/my-pack-1.0.0.zip",
    "packages": 12,
    "bytes": 52428800
  }
}
```

### `init`

```json
{
  "schema_version": 1,
  "command": "init",
  "ok": true,
  "result": {
    "dry_run": false,
    "name": "my-instance",
    "mc_version": "1.21",
    "modloader": "fabric",
    "modloader_version": "0.15.11",
    "locked_packages": 8,
    "scanned_mods": 8,
    "identified": 6,
    "unknown": 2,
    "removed": [
      { "mod_id": "duplicate-mod", "version": "1.0" }
    ],
    "dependency_error": null
  }
}
```

### `cache clean`

```json
{
  "schema_version": 1,
  "command": "cache",
  "ok": true,
  "result": {
    "subcommand": "clean",
    "dry_run": false,
    "cache_path": "/home/u/.cache/orbit",
    "files_before": 42,
    "bytes_before": 1073741824,
    "files_removed": 42,
    "bytes_freed": 1073741824
  }
}
```

### `audit`

`audit` 沿用 `orbit-bytecode-audit` 的 schema 3 `AuditReport`，直接作为 `result` 字段嵌入信封：

```json
{
  "schema_version": 1,
  "command": "audit",
  "ok": true,
  "result": {
    "schema_version": 3,
    "environment": { ... },
    "readiness": { ... },
    "artifacts": [ ... ],
    "risks": [ ... ],
    ...
  }
}
```

信封 `schema_version` 是命令信封版本；`result.schema_version`（当前 3）是 audit 自身子 schema 版本。

## 4. NDJSON 进度协议

`--progress-format ndjson` 时，每个进度事件输出一行 JSON 到 **stderr**，格式：

```json
{"type":"progress","phase":"discovery","event":"DiscoveringProject","data":{...}}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `type` | string | 固定 `"progress"`；最终无单独 `result` 行（结果走 stdout） |
| `phase` | string | `discovery` / `download` / `resolution` / `apply` / `audit` |
| `event` | string | 事件名，对应 core `ProgressEvent` / `AuditProgressEvent` 变体名 |
| `data` | object | 事件特定字段（计数、阶段、包名等） |

进度事件不包含内容哈希、物理 JAR 文件名或 provider 密钥。调用方按 `phase`/`event` 分流，无需解析自然语言。

### 包操作进度事件

| phase | event | data |
|-------|-------|------|
| `discovery` | `DiscoveryStarted` | `{}` |
| `discovery` | `DiscoveringProject` | `{provider, locator, pending_projects, artifacts_found}` |
| `discovery` | `DiscoveryFinished` | `{projects, artifacts}` |
| `download` | `CandidateDownloadStarted` | `{total}` |
| `download` | `CandidateArtifact` | `{completed, total, state}`，`state` ∈ `started`/`finished`/`cached`/`failed` |
| `download` | `CandidateDownloadFinished` | `{total}` |
| `resolution` | `ResolutionStarted` | `{packages, candidates}` |
| `resolution` | `ResolutionWorkStarted` | `{work}`，`work` 见下 |
| `resolution` | `ResolutionWorkFinished` | `{work}` |
| `resolution` | `ResolutionActivity` | `{activity}` |
| `resolution` | `ResolutionFinished` | `{solutions}` |
| `apply` | `ApplyStarted` | `{total}` |
| `apply` | `ApplyArtifact` | `{completed, total, state}` |
| `apply` | `ApplyFinished` | `{total}` |

`work` 对象：`{"kind":"enumeration_run","run":1}` 或 `{"kind":"maximality_probe","package":"sodium"}`。
`activity` 对象：`{"kind":"decision","package":"sodium"}` / `{"kind":"propagation",...}` / `{"kind":"backtrack","from_level":3,"to_level":1}` / `{"kind":"conflict"}` / `{"kind":"solution"}`。

### audit 进度事件

| phase | event | data |
|-------|-------|------|
| `audit` | `StageStarted` | `{stage, total}`，`stage` ∈ `prepare_inputs`/`scan_artifacts`/`readiness`/`analyze_mixins`/`analyze_transformers`/`detect_conflicts` |
| `audit` | `Advanced` | `{stage, completed, total}` |
| `audit` | `StageFinished` | `{stage, completed}` |

## 5. 错误协议

`--format json` 下命令失败时：

- **stdout 不输出任何内容**；
- **stderr 输出一行结构化错误 JSON**：

```json
{"type":"error","code":"mod_not_found","message":"mod 'foo' not found","detail":null}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `type` | string | 固定 `"error"` |
| `code` | string | 稳定错误码，见下表 |
| `message` | string | 面向人类的简短描述（不含敏感数据） |
| `detail` | object \| null | 可选的结构化补充（如冲突涉及的包列表） |

### 错误码

| code | 退出码 | 触发场景 |
|------|--------|----------|
| `manifest_not_found` | 1 | 当前目录无 orbit.toml |
| `manifest_parse` | 1 | orbit.toml 解析失败 |
| `lockfile_not_found` | 1 | 需要 lockfile 但不存在 |
| `mod_not_found` | 1 | 指定的 mod/包不存在 |
| `version_mismatch` | 1 | 无版本满足约束 |
| `dependency_conflict` | 1 | 求解器证明依赖无解 |
| `checksum_mismatch` | 1 | 下载内容校验失败（不含哈希值） |
| `provider_api_key_required` | 1 | provider 缺认证 |
| `io` | 1 | 文件系统错误 |
| `network` | 1 | 网络错误 |
| `json` | 1 | JSON 序列化错误 |
| `zip` | 1 | 压缩包错误 |
| `argument` | 2 | clap 参数错误（由 clap 处理，不经此协议） |
| `cancelled` | 3 | 用户交互取消 |
| `internal` | 1 | 未分类错误（兜底） |

`checksum_mismatch` 的 `message` 不包含期望/实际哈希值（CLAUDE.md #41）。

## 6. 稳定性承诺

- 信封 `schema_version` 按命令独立维护；破坏性字段变更才递增，非破坏性（新增可选字段）不递增。
- 错误码 `code` 是稳定契约，自动化可基于其分支；`message` 可随版本变化，不应作为判据。
- 进度事件名（`event`、`phase`）是稳定契约；`data` 字段新增不破坏，删除或重命名才需协议版本（暂定从 1 起，未来若破坏性变更引入 `protocol_version` 字段）。
- 内容哈希、物理 JAR 文件名、provider 密钥永不出现在任何 JSON 输出（结果或进度）。
