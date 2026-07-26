# Orbit manifest 与 lockfile 规格

本文描述当前代码实际接受的格式。旧的字符串依赖、单一 `provider`/`slug`
锁字段和 `[resolver].platforms` 已删除，不提供兼容解析。

## 1. 身份与职责

Orbit 严格分开四类事实：

- 包：由顶层 JAR 的 loader 元数据实际声明的 `mod_id` 标识。
- 包版本：由同一 JAR 声明的版本字符串表示，用于版本约束。
- 候选：由 Orbit 下载后计算的内容哈希标识。同一 `mod_id` 和版本可以有多个不同
  候选，因为它们的依赖、环境或内嵌内容可能不同。
- 远端：只说明去哪里发现或恢复 JAR；不能决定 `mod_id`、版本或依赖。

`orbit.toml` 声明根包、约束和全部候选远端。`orbit.lock` 锁定实际选择的内容及其
JAR 元数据。内容哈希是内部候选主键，不作为交互选项名称显示。

## 2. `orbit.toml`

### 2.1 完整示例

```toml
[project]
name = "survival"
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.16.10"
description = "example instance"

[platform]
minecraft_jar = { path = "../../versions/1.20.1/1.20.1.jar", sha256 = "..." }
loader_jar = { path = "../../libraries/net/fabricmc/fabric-loader/0.16.10/fabric-loader-0.16.10.jar", sha256 = "..." }
runtime_jars = [
  { path = "../../libraries/org/ow2/asm/asm/9.7/asm-9.7.jar", sha256 = "..." },
  { path = "../../libraries/net/fabricmc/intermediary/1.20.1/intermediary-1.20.1.jar", sha256 = "..." },
]
physical_environment = "client"

[resolver]
catalogs = ["modrinth"]
prerelease = false

[dependencies]
sodium = { version = ">=0.5", remotes = [
  { type = "modrinth", project_id = "AANobbMI" },
  { type = "curseforge", project_id = 394468 },
] }

zoomify = { version = "*", optional = true, env = "client", remotes = [
  { type = "modrinth", project_id = "w7ThoJFB" },
] }

local_helper = { version = "1.0.0", remotes = [
  { type = "file", path = "../sources/local-helper.jar" },
] }

[groups]
benchmark = { dependencies = ["sodium"] }

[overrides]
sodium = { version = ">=0.5.9" }
```

未知字段会报错。每个 `[dependencies]` 根包必须至少有一个 `remotes` 项，重复远端、
空路径、空 Modrinth project ID 和 `0` CurseForge project ID 都无效。

### 2.2 `[project]`

| 字段 | 必填 | 含义 |
|---|:---:|---|
| `name` | 是 | Orbit 实例名称 |
| `mc_version` | 是 | 当前实例的 Minecraft 版本 |
| `modloader` | 是 | `fabric`、`quilt`、`forge` 或 `neoforge` |
| `modloader_version` | 是 | 上次实际探测到的 loader 版本 |
| `description` | 否 | 描述 |
| `authors` | 否 | 作者数组 |
| `version` | 否 | 项目自身版本 |

### 2.3 `[platform]`

`[platform]` 是 `init`/`sync` 写入的完整运行时快照：

| 字段 | 含义 |
|---|---|
| `minecraft_jar` | 精确 Minecraft JAR 路径与 SHA-256 |
| `loader_jar` | 精确 Loader JAR 路径与 SHA-256 |
| `runtime_jars` | launcher 为该平台选择的其余运行时 JAR；按内容去重 |
| `physical_environment` | `client`、`server` 或无法确定时的 `both` |

只有 `init` 和 `sync` 读取 launcher profile、组件、libraries 或文件名候选。
`install`、`add`、`outdated`、`upgrade`、`export`、`audit` 等其它命令只解析这些
精确路径，并在使用前校验 SHA-256、Minecraft `version.json` 以及可解析的 Loader
身份/版本。路径不存在、内容变化、字段缺失、元数据矛盾或列表重复时直接报错并要求
运行 `orbit sync`；不会搜索同目录、按文件名猜替代项、回退到旧路径或静默刷新 TOML。

`sync` 不受旧快照约束，会从当前 launcher 状态重新探测并整体替换快照。loader 或
Minecraft 的变化只有经过 `sync` 才进入后续统一求解与 audit；loader 版本变化本身
仍不被先验判为不兼容。

共享游戏根与隔离版本目录都支持；每个隔离版本目录是独立 Orbit 实例。

### 2.4 `[resolver]`

| 字段 | 默认值 | 含义 |
|---|---|---|
| `catalogs` | `["modrinth"]` | 无限定 `search`/`add` 使用的 provider 目录及展示顺序 |
| `prerelease` | `false` | 预发布偏好 |

`catalogs` 不是包远端优先级。一个包已经声明的所有 `remotes` 都会进入同一次候选
发现；不会在第一个 provider 有结果后停止。

### 2.5 `[dependencies]`

依赖只能使用完整表形式：

```toml
package_id = {
  version = "*",
  optional = false,
  env = "client",
  exclude = ["broken_optional_edge"],
  remotes = [
    { type = "modrinth", project_id = "exact-project-id" },
    { type = "curseforge", project_id = 123456 },
    { type = "file", path = "../sources/package.jar" },
  ],
}
```

| 字段 | 默认值 | 含义 |
|---|---|---|
| `version` | `"*"` | 根包版本约束 |
| `optional` | `false` | 是否可由 `--no-optional` 跳过 |
| `env` | 无 | `client`、`server`；无值表示两端 |
| `exclude` | `[]` | 明确排除的依赖边 |
| `remotes` | 无 | 非空候选来源集合 |

表键必须是 JAR 实际 `mod_id`。Modrinth 使用 project ID，CurseForge 使用数值 project
ID；slug 只允许用于搜索和展示，不能持久化为包身份。显式添加远端时 Orbit 会下载并
确认目标 project 的 JAR 确实声明该 `mod_id`。

### 2.6 本地远端

普通 `file` 远端可以是相对实例目录的路径或绝对路径。若本地源位于 `mods/`，Orbit
会在写事务前复制到 `.orbit/sources/<content>.jar`，因为 `mods/` 是事务输出，未选
版本可能被删除。这个内部文件名不会出现在正常 CLI 输出；`orbit remote list` 将它
显示为 `managed local source`，并允许按列表序号删除。恢复时优先沿用 lock 中原文件名；
空 lock 重建则使用安全化的 `mod_id-version.jar`，内部内容哈希不会成为 `mods/` 文件名。

### 2.7 `[groups]` 与 `[overrides]`

组只列根包 ID。override 用相同表结构解析，但不是新的根包，因此不要求 `remotes`；
当前使用 `version` 覆盖依赖边约束，并使用 `exclude` 排除指定边。

## 3. `orbit.lock`

### 3.1 示例

```toml
[meta]
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.16.10"

[[package]]
mod_id = "sodium"
version = "0.5.8"
sha1 = "..."
sha256 = "..."
sha512 = "..."
filename = "sodium.jar"
remotes = [
  { type = "modrinth", project_id = "AANobbMI" },
  { type = "curseforge", project_id = 394468 },
]
artifact_sources = [
  { type = "modrinth", project_id = "AANobbMI", version_id = "release-id", download_url = "https://..." },
  { type = "curseforge", project_id = 394468, file_id = 1234567, download_url = "https://..." },
]
dependencies = []
environment = "both"
provides = []
embedded_artifacts = []
bundled = []
```

`remotes` 与 `artifact_sources` 不可混为一谈：

- `remotes` 枚举该逻辑包以后可以重新发现哪些候选；
- `artifact_sources` 只枚举能恢复当前选中内容哈希的精确工件。

同一字节内容来自多个 provider 时，lock 仍只有一个包候选，但会保留多个精确
`artifact_sources`。不同字节即使声明相同版本也保持为不同候选。

`dependencies`、`environment`、`provides`、`language_loader`、
`embedded_artifacts` 与递归 `bundled` 均来自已下载 JAR，不采用平台展示数据。
正常稳定状态下一个 `mod_id` 只有一个顶层 `[[package]]`。持久 lock 中 `sha512`、
非空 `remotes` 与非空 `artifact_sources` 都是必需项。旧的无内容身份或单 provider
字段不会被兼容读取。

## 4. 读写与求解约束

1. Provider 先从每个配置远端递归枚举当前 Minecraft/loader 的 project/artifact
   闭包。
2. 所有发现的 artifact 先进入统一队列，再批量查缓存或下载。
3. Orbit 对每个字节流自行计算哈希、读取 loader 元数据，并按哈希去重。
4. 远端 relation 只用于继续发现 project；真实依赖只来自 JAR。
5. 下载闭包缺少某个 JAR-declared required `mod_id` 时，离线求解正常返回无解。
6. 同版本不同哈希候选都交给 PubGrub；哈希只保持候选唯一性，不参与版本高低比较。
7. 唯一 Pareto 极大解自动采用；多个解必须询问。候选相同版本时，CLI 用 provider
   project/release 与依赖差异说明选项，绝不显示哈希。
8. 任何会移除未选包版本的方案都在写盘前列出并确认。

`sync` 是纯本地重新探测与对账，不调用 provider、不下载修复；它保留 manifest 已知
远端，并把当前本地内容写为锁定工件来源。`install` 才执行完整联网发现与修复，但只
消费 `[platform]` 快照，不承担平台探测或快照刷新。

## 5. 远端管理

```text
orbit remote list <package>
orbit remote add <package> modrinth <project-id>
orbit remote add <package> curseforge <numeric-project-id>
orbit remote add <package> file <jar-path>
orbit remote remove <package> modrinth <project-id>
orbit remote remove <package> --index <number>
```

`add` 会下载候选并验证 JAR 实际声明目标包；若用户给 Modrinth slug，持久化前会规范成
API 返回的 project ID。`remove` 不能删除最后一个远端。列表序号用于引用不应展示其
内部内容寻址路径的 managed local source。删除候选发现远端不会破坏当前 lock：
已选内容的精确 `artifact_sources` 会保留到下一次求解选择其它内容，保证当前锁仍可
恢复；后续候选枚举不再访问已删除的 remote。

## 6. 版本控制

应同时提交 `orbit.toml` 与 `orbit.lock`。`.orbit/sources/` 是本地远端的持久源，
若团队或构建机需要还原这些本地包，也必须随项目分发，或先为包增加可访问的网络远端。
