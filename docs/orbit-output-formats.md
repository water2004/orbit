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
| stderr | 进度条/交互提示/警告 | NDJSON 进度（若启用）+ NDJSON 交互请求 + 结构化错误 |

**关键约束**：`--format json` 下 stdout 永远是且只是一个完整 JSON 文档（成功时是结果，失败时为空）；调用方可以安全 `orbit --format json ... | jq`。

## 2. JSON 结果 schema 通用约定

每个命令的结果是一个顶层 JSON 对象，固定包含：

| 字段 | 类型 | 说明 |
|------|------|------|
| `schema_version` | number | 统一机器协议版本，当前为 2；破坏性变更才递增 |
| `command` | string | 命令名（如 `"search"`、`"install"`） |
| `ok` | boolean | `true` |
| `result` | object | 命令特定结果体 |

示例：

```json
{
  "schema_version": 2,
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
  "schema_version": 2,
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
        "latest_version": "0.5.8",
        "downloads": 1500000,
        "mc_versions": ["1.20", "1.21"],
        "client_side": "required",
        "server_side": "unsupported",
        "categories": ["optimization"],
        "icon_url": "https://cdn.modrinth.com/data/AANobbMI/icon.png",
        "accent_color": 1193046,
        "compatible": true
      }
    ],
    "truncated": false
  }
}
```

- `ref_mc_version` 缺省时为 `null`；`compatible` 仅在 `ref_mc_version` 非 null 时存在，否则为 `null`。
- `truncated`：是否因 `--limit` 截断了结果。
- `latest_version` 始终是面向用户的版本号。Modrinth 搜索响应中的 opaque version ID 会由
  Orbit 批量查询 `/versions` 后转换为 `version_number`，绝不直接显示为版本。

### `info`

```json
{
  "schema_version": 2,
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
    "icon_url": "https://cdn.modrinth.com/data/AANobbMI/icon.png",
    "accent_color": 1193046,
    "website_url": "https://modrinth.com/mod/sodium",
    "source_url": "https://github.com/CaffeineMC/sodium",
    "issues_url": "https://github.com/CaffeineMC/sodium/issues",
    "wiki_url": null,
    "gallery": [
      {
        "url": "https://cdn.modrinth.com/data/AANobbMI/images/example.png",
        "thumbnail_url": null,
        "title": "In game",
        "description": null
      }
    ],
    "recent_versions": [
      { "version": "0.5.8", "mc_versions": ["1.21"], "loader": "fabric", "released_at": "2024-06-01T00:00:00Z" }
    ],
    "dependencies": [
      { "slug": "fabric-api", "project_id": "P7dR8mSH", "required": true }
    ]
  }
}
```

`icon_url`、链接和 `gallery` 是 provider 托管的纯展示数据，供原生 GUI 等调用方使用；
它们不参与包身份、依赖约束、候选去重或下载校验。`accent_color` 是 `0xRRGGBB` 的十进制
表示。CurseForge 数据严格映射 Core API 的 `logo`、`screenshots` 与 `links`，不从网页
或文件名猜测。

### `list`

```json
{
  "schema_version": 2,
  "command": "list",
  "ok": true,
  "result": {
    "target": null,
    "tree": false,
    "packages": [
      {
        "mod_id": "sodium",
        "version": "0.5.8",
        "icon_path": "/home/user/.cache/orbit/presentation/mod-icons/content.png",
        "remotes": ["modrinth:AANobbMI"],
        "configured_environment": null,
        "environment": "both",
        "root": true,
        "optional": false,
        "dependencies": ["fabric-api"],
        "bundled": [{ "mod_id": "sodium-base", "version": "0.5.8" }]
      }
    ]
  }
}
```

`configured_environment: null` 表示 TOML 使用 `auto`；`environment` 是实际用于根过滤
的有效值：显式 TOML 覆盖优先，否则来自 lock 中精确候选的 JAR 声明。`root=false`
表示传递包，不能直接设置根过滤或 discovery remote。`tree: true` 时额外返回 `roots`
与每个包的 `dependents`，结构见 schema 文档源码。
`icon_path` 是 Orbit CLI 从当前精确 JAR 的 Loader 元数据读取、限制尺寸并规范化为 PNG 后
写入全局展示缓存的本地路径；缺少或无效图标时省略。GUI 不打开 JAR，也不拿远端项目图标
冒充已安装内容的图标。

### `env`

```json
{
  "schema_version": 2,
  "command": "env",
  "ok": true,
  "result": {
    "package": "sodium",
    "configured": null,
    "effective": "client",
    "dry_run": false
  }
}
```

`configured: null` 表示持久化状态为 `auto`；`effective` 来自当前 lock。尚无选中 lock
候选时 `effective` 也为 `null`，将在候选 JAR 解析和选择后确定。

### `migrate check` / `migrate export`

```json
{
  "schema_version": 2,
  "command": "migrate",
  "ok": true,
  "result": {
    "subcommand": "check",
    "dry_run": false,
    "target_directory": "/home/u/instances/1.21-fabric",
    "source_mc_version": "1.20.1",
    "target_mc_version": "1.21",
    "target_loader": "fabric",
    "target_loader_version": "0.16.10",
    "summary": {
      "selected_packages": 12,
      "installs": 0,
      "upgrades": 8,
      "downgrades": 1,
      "replacements": 0,
      "removals": 1
    },
    "changes": [
      {
        "package": "sodium",
        "kind": "upgrade",
        "current_version": "0.5.8",
        "selected_version": "0.6.0",
        "selected_description": "Modrinth project AANobbMI, release mc1.21"
      }
    ],
    "diagnostics": [],
    "warnings": []
  }
}
```

`migrate export` 使用相同结果体并令 `subcommand` 为 `"export"`，另外包含
`export: { "applied": true, "config_files": 14, "config_bytes": 8192 }`。两个子命令
使用同一个目标运行时联合规划器；导出不会逐包重新检查。

### `outdated`

```json
{
  "schema_version": 2,
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
  "schema_version": 2,
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
  "schema_version": 2,
  "command": "instances",
  "ok": true,
  "result": { "subcommand": "default", "name": "alpha" }
}
```

`instances remove <name>`：

```json
{
  "schema_version": 2,
  "command": "instances",
  "ok": true,
  "result": { "subcommand": "remove", "name": "alpha" }
}
```

### `remote list`

```json
{
  "schema_version": 2,
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
  "schema_version": 2,
  "command": "sync",
  "ok": true,
  "result": {
    "dry_run": false,
    "summary": {
      "platform_changes": 1,
      "added": 1,
      "changed": 1,
      "removed": 0,
      "missing": 1
    },
    "platform_changes": [
      { "field": "mc_version", "previous": "1.20", "current": "1.21" }
    ],
    "added": ["sodium"],
    "changed": ["lithium"],
    "missing": ["fabric-api"],
    "removed": [],
    "warnings": []
  }
}
```

### `install`

`install` 只报告精确 lock 物化结果，不返回求解方案或删除动作：

```json
{
  "schema_version": 2,
  "command": "install",
  "ok": true,
  "result": {
    "dry_run": false,
    "summary": { "installed": 3, "already_present": 8, "skipped": 1 },
    "installed": ["sodium", "lithium", "fabric-api"],
    "already_present": ["modmenu"],
    "skipped": ["client-only-package"]
  }
}
```

### `add` / `fix` / `upgrade`

共用事务报告 schema：

```json
{
  "schema_version": 2,
  "command": "fix",
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
  "schema_version": 2,
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

导出期间 NDJSON 使用 `phase: "export"`，事件依次为 `ExportStarted`、零个或多个
`ExportAdvanced`、`ExportFinished`。`ExportAdvanced.completed/total` 是校验与实际归档写入
的字节工作量，`completed_packages/packages` 是已完成校验的逻辑包数；事件不包含物理 JAR
文件名。取消进程不会产生伪造的 finished 事件。

```json
{
  "schema_version": 2,
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
  "schema_version": 2,
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
  "schema_version": 2,
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
    "lock_created": true,
    "dependency_error": null
  }
}
```

### `cache clean`

```json
{
  "schema_version": 2,
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

### `config`

`path`：

```json
{
  "schema_version": 2,
  "command": "config",
  "ok": true,
  "result": {
    "subcommand": "path",
    "config_path": "C:\\Users\\user\\AppData\\Roaming\\orbit\\config.toml"
  }
}
```

`list`：

```json
{
  "schema_version": 2,
  "command": "config",
  "ok": true,
  "result": {
    "subcommand": "list",
    "config_path": "C:\\Users\\user\\AppData\\Roaming\\orbit\\config.toml",
    "entries": [
      {
        "key": "cache.capacity-mib",
        "value_type": "integer",
        "sensitive": false,
        "value": 2048
      },
      {
        "key": "auth.curseforge-api-key",
        "value_type": "string",
        "sensitive": true,
        "value": "<redacted>"
      },
      {
        "key": "network.proxy",
        "value_type": "string",
        "sensitive": false
      }
    ]
  }
}
```

`get`、`set` 与 `unset` 返回同一个单项结构：

```json
{
  "schema_version": 2,
  "command": "config",
  "ok": true,
  "result": {
    "subcommand": "set",
    "config_path": "C:\\Users\\user\\AppData\\Roaming\\orbit\\config.toml",
    "dry_run": false,
    "entry": {
      "key": "cache.capacity-mib",
      "value_type": "integer",
      "sensitive": false,
      "value": 2048
    }
  }
}
```

未设置的可选字段省略 `value`。敏感字段无论 text/JSON 都只输出
`"<redacted>"`；环境变量覆盖不进入这些结果。`--dry-run` 时 `dry_run` 为 `true`，
结果展示验证后的目标值，但不修改文件。

### `audit`

`audit` 沿用 `orbit-bytecode-audit` 的 schema 5 `AuditReport`，直接作为 `result` 字段嵌入信封：

```json
{
  "schema_version": 2,
  "command": "audit",
  "ok": true,
  "result": {
    "schema_version": 5,
    "environment": { ... },
    "readiness": { ... },
    "namespace": { ... },
    "artifacts": [ ... ],
    "unary_risks": [ ... ],
    "risks": [ ... ],
    ...
  }
}
```

信封 `schema_version` 是命令信封版本；`result.schema_version`（当前 5）是 audit 自身子
schema 版本。schema 5 的 `environment.loader` 是唯一、已验证的 loader 枚举，不再输出
`declared_loader` / `detected_loader` 双字段。

## 4. NDJSON 进度协议

`--progress-format ndjson` 时，每个进度事件输出一行 JSON 到 **stderr**。Orbit 与
Orbit Launcher 直接使用 `orbit-machine-protocol` 中同一个信封类型，格式：

```json
{"schema_version":2,"type":"progress","command":"install","sequence":1,"phase":"discovery","data":{"event":"DiscoveringProject","provider":"modrinth","locator":"AANobbMI","pending_projects":2,"artifacts_found":8}}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `type` | string | 固定 `"progress"`；最终无单独 `result` 行（结果走 stdout） |
| `schema_version` | number | 与成功/错误信封相同的机器协议版本，当前为 2 |
| `command` | string | 产生事件的现有 CLI 命令 |
| `sequence` | number | 单进程内从 1 开始严格递增 |
| `phase` | string | `discovery` / `download` / `resolution` / `apply` / `audit`；Launcher 另有 `metadata` / `eula` / `java` / `loader` / `authentication` / `launch` / `process` / `supervisor` |
| `data` | object | 内含 `event` 与事件特定字段（计数、阶段、包名等） |

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

## 5. 同进程交互协议

`--format json` 的命令遇到多个真实包身份、多个 Pareto 极大解或写入前确认时，不启动
第二条命令、不返回待恢复 token，也不静默采用第一个选项。CLI 在 **stderr** 输出一行
`interaction` NDJSON，然后暂停并从同一个子进程的 **stdin** 读取一行响应：

```json
{"schema_version":2,"type":"interaction","command":"upgrade","sequence":12,"interaction_id":"resolution-12","interaction":"resolution","prompt":"Choose one Pareto-maximal dependency solution","choices":[{"id":"1","label":"Option 1","description":"2 logical package actions","data":{"changes":[{"different":true,"change":{"package":"sodium","action":"upgrade","current_version":"0.8.9","selected_version":"0.9.1"}}],"warnings":[],"diagnostics":[]}}],"default_choice":"1","allow_cancel":true}
```

调用方写回：

```json
{"schema_version":2,"type":"interaction_response","interaction_id":"resolution-12","selected_choice":"1","cancelled":false}
```

取消时省略 `selected_choice` 并令 `cancelled=true`。响应必须使用当前 schema、完全相同的
`interaction_id` 和请求中存在的 choice ID；stdin 关闭、解析失败、错配或未知 choice
都会终止本次事务，绝不回退到默认项。`sequence` 与该进程的 progress 事件共享同一个
严格递增计数器，因此前端可以按实际发生顺序合并显示。

| `interaction` | 说明 |
|---|---|
| `package` | 在同一 provider locator 返回的多个可行 JAR `mod_id` 中选择 |
| `resolution` | 在完整 Pareto 极大解集合中选择；`data.changes[].different=true` 是不依赖颜色的差异标记 |
| `confirmation` | 查看精确逻辑包事务并决定是否写入 |

`--yes` 只跳过 `confirmation`，不会跳过 `package` 或 `resolution`。唯一包身份/唯一解不会
产生选择请求。交互请求与进度一样不含哈希和物理 JAR 文件名；最终成功结果仍只在 stdout
出现一次。

## 6. 错误协议

`--format json` 下命令失败时：

- **stdout 不输出任何内容**；
- **stderr 输出一行结构化错误 JSON**：

```json
{"schema_version":2,"type":"error","command":"info","ok":false,"code":"mod_not_found","message":"mod 'foo' not found"}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `type` | string | 固定 `"error"` |
| `schema_version` | number | 与成功/进度信封相同，当前为 2 |
| `command` | string | 失败的现有 CLI 命令 |
| `ok` | boolean | 固定 `false` |
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

## 7. 稳定性承诺

- 成功、错误、进度、交互请求和交互响应共用 `orbit-machine-protocol` 的单一
  `schema_version`；破坏性字段变更
  一次性提升整个进程协议，不保留旧信封或备用解析路径。
- 错误码 `code` 是稳定契约，自动化可基于其分支；`message` 可随版本变化，不应作为判据。
- 进度事件名（`data.event`、`phase`）是稳定契约；`data` 字段新增不破坏，删除或重命名
  必须提升现有 `schema_version`。
- 内容哈希、物理 JAR 文件名、provider 密钥永不出现在任何 JSON 输出（结果或进度）。
