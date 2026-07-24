# Orbit.toml 规格说明书

> 本文档定义 `orbit.toml` 的完整 schema、语义及配套的 `orbit.lock` 锁文件格式。
> 这是 Orbit CLI 开发的**唯一参照标准**——代码行为必须与此文档一致。

---

## 目录

1. [概述](#1-概述)
2. [文件关系](#2-文件关系)
3. [`orbit.toml` 完整 Schema](#3-orbittoml-完整-schema)
   - [3.1 `[project]` — 项目元数据](#31-project--项目元数据)
   - [3.2 `[resolver]` — 解析策略](#32-resolver--解析策略)
   - [3.3 `[dependencies]` — 模组依赖](#33-dependencies--模组依赖)
   - [3.4 `[groups]` — 模组分组](#34-groups--模组分组)
   - [3.5 `[overrides]` — 依赖覆盖](#35-overrides--依赖覆盖)
4. [`orbit.lock` 完整 Schema](#4-orbitlock-完整-schema)
5. [字段速查表](#5-字段速查表)
6. [语义规则](#6-语义规则)
   - [6.1 版本约束语法](#61-版本约束语法)
   - [6.2 平台自动解析](#62-平台自动解析)
   - [6.3 依赖来源类型](#63-依赖来源类型)
   - [6.4 环境过滤与分组加载](#64-环境过滤与分组加载)
   - [6.5 冲突解决](#65-冲突解决)
   - [6.6 `exclude` — 传递依赖排除](#66-exclude--传递依赖排除)
7. [完整示例](#7-完整示例)

---

## 1. 概述

`orbit.toml` 是 Orbit 项目目录的**唯一真实数据源 (Single Source of Truth)**。它记录：

- **项目元数据**：Minecraft 版本、模组加载器类型
- **模组声明**：需要哪些模组、版本约束
- **解析策略**：多平台优先级、预发布版本的取舍

`orbit.toml` 由**用户手动编辑**（或通过 `orbit add/remove` 等命令自动维护），纳入 Git 版本控制。

与之配套的 `orbit.lock` 由 Orbit **自动生成**，精确锁定每个依赖的版本和校验值，确保可复现安装。`orbit.lock` 应纳入 Git；用户不应手动编辑它。

---

## 2. 文件关系

```
.minecraft/           （或任意 Orbit 项目根目录）
├── orbit.toml        ← 声明式清单：用户编辑，表达意图
├── orbit.lock        ← 锁定文件：自动生成，记录事实
└── mods/
    ├── sodium-0.8.10.jar
    ├── jei-12.0.0.jar
    └── ...
```

| 文件 | 角色 | 类比 | 是否纳入 Git |
|------|------|------|--------------|
| `orbit.toml` | 开发者声明依赖意图（"我要 sodium >= 0.5"） | `Cargo.toml` / `package.json` | **是** |
| `orbit.lock` | Orbit 记录精确解析结果（"sodium 已解析为 0.8.10, sha256=abc"） | `Cargo.lock` / `package-lock.json` | **是** |

> **关键原则**：`orbit.toml` 表达 **What**（想要什么），`orbit.lock` 记录 **How**（具体是什么）。

---

## 3. `orbit.toml` 完整 Schema

### 3.1 `[project]` — 项目元数据

```toml
[project]
name = "my-pack"                  # 必填。实例名称，唯一标识符，用于 `orbit instances` 管理
mc_version = "1.21.5"             # 必填。目标 Minecraft 版本，严格按 "主.次.补丁" 格式
modloader = "fabric"              # 必填。模组加载器
modloader_version = "0.16.10"     # 必填。加载器版本，直接影响 API 兼容性

# --- 以下为可选字段 ---
# description = "这是我的生存向整合包"
# authors = ["GBwater", "friend"]
# version = "1.0.0"               # 整合包自身的版本号
```

**字段规范**：

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `name` | `String` | **是** | 实例名，仅允许 `[a-zA-Z0-9_-]+`，长度 1–64 |
| `mc_version` | `String` | **是** | Minecraft 版本，如 `1.20.1`、`1.21` |
| `modloader` | `String` | **是** | 枚举值：`fabric` \| `forge` \| `neoforge` \| `quilt` |
| `modloader_version` | `String` | **是** | 加载器版本号，如 `0.16.10`。Fabric Loader / Forge 的 API 随此版本变动，锁定它是可复现构建的前提 |
| `description` | `String` | 否 | 自由文本，单行 |
| `authors` | `[String]` | 否 | 作者列表 |
| `version` | `String` | 否 | 整合包版本，推荐 semver |

---

### 3.2 `[resolver]` — 解析策略

```toml
[resolver]
platforms = ["modrinth"]   # 平台优先级，依次尝试直到找到匹配
prerelease = false         # 是否使用 alpha/beta/预发布版本
```

**字段规范**：

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `platforms` | `[String]` | `["modrinth"]` | 当依赖未显式指定平台时，按此顺序查找 |
| `prerelease` | `bool` | `false` | 预发布偏好 schema；当前 provider 查询尚未消费 |

> 编排层按 `platforms[0]` → `platforms[1]` → … 回退。`modrinth` 与
> `curseforge` 均可用；后者需要全局 CurseForge API Key。未知名称不会被静默跳过。

---

### 3.3 `[dependencies]` — 模组依赖

每个依赖的**键**是 JAR 元数据声明的模组 ID（统一记为 `mod_id`），**值**可以是简写字符串或内联表 (inline table)。

> **重要**：`orbit.toml` 中不包含 `platform`、`slug`、`type`、`path`、`url`、`sha256` 字段。这些字段属于 `orbit.lock` 锁文件。manifest 仅声明"我要哪个模组 + 什么版本"，具体来源和校验由 lock 文件负责。

#### 3.3.1 简写形式

```toml
[dependencies]
# 通配符：接受任意版本，由 Orbit 自动选择最新兼容版
sodium = "*"

# 语义化版本约束
lithium = ">=0.11 <0.14"

# 精确版本（= 前缀）
iris = "=1.7.0"

# 脱字符约束（兼容同一大版本）
fabric-api = "^0.92"
```

当使用简写形式时，`optional` 和 `env` 取默认值（`false` 和 `"both"`）。

#### 3.3.2 完整内联表形式

```toml
[dependencies]
# 完整形式：version, optional, env, exclude 四个字段
sodium = "^0.5"
fabric-api = "*"
zoomify = { version = "*", optional = true, env = "client" }
inventory-hud = { version = "*", env = "client" }
some-bloated-mod = { version = "^2", exclude = ["annoying-library"] }
```

**内联表字段规范**：

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `version` | `String` | `"*"` | 版本约束表达式，详见 [§6.1](#61-版本约束语法) |
| `optional` | `bool` | `false` | 标记为可选的模组会被 `--no-optional` 跳过 |
| `env` | `String` | `"both"` | 运行环境限定：`"client"` \| `"server"` \| `"both"` |
| `exclude` | `[String]` | `[]` | 排除指定传递依赖。填入要跳过的 `mod_id` 列表，防止上游夹带无用库 |

> **仅此四个字段**。`DependencySpec` 枚举仅支持 `Short(String)` 和 `Full { version, optional, env, exclude }` 两种变体。

---

### 3.4 `[groups]` — 模组分组 (高级场景)

分组用于**自定义安装场景**（如基准测试、调试工具集等）。对于基础的端侧分离（客户端/服务端），**优先使用内联 `env` 字段**（见 [§3.3.2](#332-完整内联表形式)），内聚性更好，无需在两个表之间跳转。

```toml
# 基础端侧分离 — 用 env 字段（推荐）
# zoomify = { version = "*", env = "client" }
# spark   = { version = "*", env = "server" }

# 高级自定义分组 — 用 [groups]（benchmark、debug 等非标准场景）
[groups.benchmark]
dependencies = ["spark", "carpet", "lithium"]

[groups.debug]
dependencies = ["spark", "ledger"]
```

**语义**：

- `env` 字段和 `[groups]` **可以共存**。一个依赖可以同时设置 `env = "client"` 并属于 `[groups.benchmark]`。
- 分组仅过滤**是否安装**，不改变依赖版本解析。
- 分组内引用的名称必须已在 `[dependencies]` 中声明。
- 命令用法：`orbit install --group benchmark` 仅安装 benchmark 组 + 通用依赖。
- 未被任何分组引用且未设置 `env` 的依赖视为**通用依赖**，始终安装。
- `env` 的过滤行为：
  - `env = "client"`：仅在 `orbit install`（默认含客户端）时安装；`--target server` 时跳过。
  - `env = "server"`：仅在 `--target server` 时安装；默认安装时跳过。
  - `env = "both"`（默认）：始终安装。

---

### 3.5 `[overrides]` — 依赖覆盖

用于紧急情况下**强制指定**某个传递依赖的版本，覆盖上游解析结果。

```toml
[overrides]
# 强制锁定 fabric-api 的版本（无论传递依赖需要什么版本）
"fabric-api" = { version = "=0.90.0" }
```

**使用约束**：
- `[overrides]` 中的条目格式与 `[dependencies]` 的内联表完全一致（`version`, `optional`, `env`, `exclude`）。
- 覆盖是**全局**的 — 即使某个依赖未被顶级 `[dependencies]` 声明（仅作为传递依赖出现），覆盖也会生效。
- 覆盖不能新增依赖；它只修改已有依赖的版本。
- **不推荐日常使用**。仅在遇到依赖冲突、上游未及时更新时使用。

---


## 4. `orbit.lock` 完整 Schema

`orbit.lock` 是从真实 JAR 元数据生成的 Fat Lockfile。它记录已解析事实，不保存旧
`implanted` schema，也不按 loader 分裂字段。

```toml
[meta]
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.16.10"

[[package]]
mod_id = "example"
version = "1.2.3"
sha1 = "..."
sha256 = "..."
sha512 = "..."
filename = "example-1.2.3.jar"
provider = "modrinth"
environment = "both"

[package.modrinth]
project_id = "project-id"
version_id = "version-id"
slug = "example"
download_url = "https://cdn.example/..."

# CurseForge 来源改用独立子表（与上面的 Modrinth 示例二选一）
# [package.curseforge]
# project_id = 123456
# file_id = 789012
# slug = "example"
# download_url = "https://example.invalid/example.jar"

[[package.dependencies]]
expression = "only"
[package.dependencies.value]
id = "minecraft"
requirement = ">=1.20.1"
kind = "required"
environment = "both"
ordering = "none"
reason = "requires modern game APIs"

[[package.provides]]
id = "example_api"
version = "1.2.3"

[package.language_loader]
id = "javafml"
requirement = "[47,)"

[[package.embedded_artifacts]]
id = "org.example:shared"
requirement = "[1,2)"
version = "1.5"
path = "META-INF/jarjar/shared.jar"
obfuscated = false

[[package.bundled]]
mod_id = "example_internal"
version = "1.2.3"
environment = "both"
```

顶层 `[[package]]` 字段：

| 字段 | 说明 |
|---|---|
| `mod_id` / `version` | loader 元数据声明的逻辑 ID 与版本 |
| `sha1` / `sha256` / `sha512` | 同一个物理 JAR 的哈希 |
| `filename` | 实例中的物理文件名 |
| `provider` | 当前为 `modrinth`、`curseforge` 或 `file` |
| `environment` | `client`、`server`、`both` |
| `dependencies` | 递归 `DependencyExpression` |
| `provides` | 该逻辑模组提供的能力 ID/版本 |
| `language_loader` | Forge-family language provider 要求 |
| `embedded_artifacts` | Jar-in-Jar Maven artifact |
| `bundled` | 同一物理文件提供的其他逻辑模组，可递归 |

`DependencyExpression` 使用 serde tagged enum：

- `expression = "only"` 的 `value` 是 `ModDependency`；
- `expression = "any"` / `"all"` 的 `value` 是表达式数组；
- 单一关系保留 `id`、`requirement`、`kind`、`environment`、`ordering`、
  可选 `reason` 和可选递归 `unless`。

`[[package.bundled]]` 不含哈希、provider 或 filename，因为它不是独立物理文件。它保存
与顶层逻辑模组相同的 environment、dependencies、provides、language loader、
embedded artifacts 和递归 bundled。

provider 专属字段只进入 `[package.modrinth]`、`[package.curseforge]` 或
`[package.file]`，不扁平污染公共 schema。远端子表只保存恢复下载所需的
project/version-file ID、slug 和 URL；不保存或信任平台展示版本。

## 5. 字段速查表

### orbit.toml 顶级表一览

| 表 | 必填 | 说明 |
|----|------|------|
| `[project]` | **是** | 项目元数据 |
| `[resolver]` | 否 | 全局解析策略（有默认值） |
| `[dependencies]` | 否 | 模组依赖声明 |
| `[groups.<name>]` | 否 | 按场景分组 |
| `[overrides]` | 否 | 强制覆盖上游版本 |

### 依赖值的两种写法

| 写法 | 示例 | 含义 |
|------|------|------|
| 字符串 | `sodium = "^0.5"` | 简写：版本约束，`optional`/`env` 取默认值 |
| 内联表 | `zoomify = { version = "*", optional = true, env = "client" }` | 完整指定：version, optional, env, exclude |

### orbit.toml vs orbit.lock 字段归属

| 字段 | `orbit.toml` | `orbit.lock` |
|------|:---:|:---:|
| 版本约束 (`version`) | 声明意图 | — |
| 解析后版本 (`version`) | — | 记录事实 |
| `optional` | 声明意图 | — |
| `env` | 声明意图 | — |
| `exclude` | 声明意图 | — |
| `mod_id` | 作为键使用 | 存储 JAR loader 元数据声明的 ID |
| `sha1` / `sha256` / `sha512` | — | 本地 JAR 计算 |
| `provider` | — | 安装时确定 |
| `[package.modrinth]` | — | Modrinth 下载定位信息 |
| `[package.curseforge]` | — | CurseForge 下载定位信息 |
| `[package.file]` | — | 文件路径 |
| `[[package.dependencies]]` | — | JAR 声明的依赖 |
| `[[package.bundled]]` | — | 同一 JAR 内的其他逻辑模组 |
| `[[package.provides]]` | — | loader alias/capability |
| `[[package.embedded_artifacts]]` | — | Forge-family Jar-in-Jar artifact |

---

## 6. 语义规则

### 6.1 版本约束语法

Orbit 的版本约束借鉴 Cargo/npm，但针对 Minecraft 模组的非标准版本号做了适配。

| 表达式 | 含义 | 示例（匹配/不匹配） |
|--------|------|---------------------|
| `*` | 任意版本 | 匹配一切 |
| `>=X.Y.Z <A.B.C` | Fabric/Quilt 范围约束（空格为 AND） | `>=0.5 <1.0` |
| `^X.Y.Z` | Fabric caret | `^0.5.8` ≡ `>=0.5.8 <1.0.0-` |
| `~X.Y.Z` | 固定前两个组件 | `~0.5.8` ≡ `>=0.5.8 <0.6.0` |
| `=X.Y.Z` | 精确版本 | 仅匹配 `X.Y.Z` |
| `[A,B)` | Forge/NeoForge Maven 范围 | `>=A` 且 `<B` |
| `[A]` | Forge/NeoForge Maven 精确范围 | 仅匹配 `A` |

版本模型按项目 loader 选择。Fabric/Quilt 使用 Fabric predicate；Forge/NeoForge
使用 Maven range。Orbit 不从平台展示名猜测 semver 尾部，实际比较值来自 JAR
自声明版本；无法解析的 Fabric/Quilt 版本回退为原始字符串精确匹配。Maven 裸版本
是 recommendation（允许任意版本），不是精确范围；精确 Forge 约束写作 `[A]`。

### 6.2 平台自动解析

`[resolver].platforms` 按顺序遍历：

1. 遍历 `[resolver].platforms` 列表。
2. 对每个平台，使用依赖的键名（即 `mod_id`）作为平台内的搜索标识符。
3. 第一个返回有效结果的平台即为该依赖的来源。
4. 若所有可用平台均无匹配，命令报错退出。

当前可用平台为 Modrinth 与 CurseForge。默认值仍是 `["modrinth"]`，避免没有
CurseForge API Key 的用户在普通工作流中遇到认证失败。显式加入 `curseforge` 后按列表
顺序回退；已有 lockfile 来源会在恢复、检查和升级时自动补入 provider 集合。

### 6.3 依赖来源类型

| 类型 | `orbit.toml` | `orbit.lock` `provider` | jar 获取方式 |
|------|-------------|------------------------|-------------|
| 平台在线 | 仅声明 `mod_id` + 版本约束 | `"modrinth"` / `"curseforge"` | 通过原始平台 API 下载 |
| 本地文件 | 仅声明 `mod_id` + 版本约束 | `"file"` | 已存在于本地 `mods/` 目录 |

> **设计原则**：`orbit.toml` 中不再出现 `type`、`path`、`url`、`sha256` 字段。文件路径和校验值只在 `orbit.lock` 的 `[package.file]` 子表中出现。这保持了 manifest 和 lockfile 的职责分离。

### 6.4 环境过滤与分组加载

环境过滤 (`env`) 和分组 (`[groups]`) 控制的是**安装行为**，不改变依赖解析逻辑。

#### 6.4.1 为什么需要 `env` —— Minecraft 的三种部署场景

模组开发者已经在 `fabric.mod.json` / `mods.toml` 里声明了该模组能在哪端运行，平台 API 也会返回这个元数据（Modrinth 的 `client_side` / `server_side`）。但 Minecraft 独有的**内置服务端（Integrated Server）**机制——单机游戏由客户端后台启动一个内置服务端并自连接——使环境划分比常规软件的前端/后端二分要复杂。具体来说，存在三种截然不同的部署场景：

**场景一：纯服务器环境 (Dedicated Server)**

运行在 Linux VPS 上的无头（Headless）服务端，没有图形界面。**绝对不能加载任何客户端渲染代码**（光影前置、小地图、UI 修改），否则服务器启动时会抛 `NoClassDefFoundError: net/minecraft/client/Minecraft` 直接崩溃。

服主执行：
```bash
orbit install --target server
```
Orbit 严格剔除所有 `env = "client"` 的模组，仅安装 `server` + `both`。

**场景二：纯客户端环境 (Pure Client — 只连别人的服务器)**

玩家为某个大型多人服务器专门做的客户端包，不需要运行单机存档，因此完全不需要服务端模组（防作弊、自动备份、实体寻路优化等）。虽然客户端加载服务端模组通常不会崩溃（会被静默忽略），但会**白白占用内存和启动时间**。

玩家执行：
```bash
orbit install --target client
```
Orbit 剔除所有 `env = "server"` 的模组，仅保留 `client` + `both`，打造最轻量的纯净客户端。

**场景三：单机/局域网环境 (Singleplayer — Client + Integrated Server)**

绝大多数普通玩家的场景——自己玩单机生存，或开局域网联机。单机游戏本质上是**客户端内嵌了一个内置服务端**，因此既需要光影、小地图（client 模组），也能跑服务端性能优化如 Lithium、Carpet（server 模组）。

直接执行默认命令：
```bash
orbit install
```
Orbit 全量安装——`client` + `server` + `both`——因为单机环境兼具两端特性。

---

**额外的两层动机：**

**平台元数据纠错**：现实中大量模组作者上传时乱填标签（纯客户端模目标成 `both` 或不填）。`env` 字段允许整合包作者在本地强制修正上游错误，这个覆盖优先于平台 API 返回的 `client_side` / `server_side`。

**业务层可选体验**：有些模组确实是双端通用的（`both`），但你的整合包设计只希望它出现在某一端——比如某个语音模组，虽然 jar 两端都能跑，但你有独立部署的服务端程序，不希望它出现在客户端分发。设 `env = "server"` 即可。

> **总结**：模组开发者决定了模组**能不能**在客户端/服务端运行；Orbit 的 `env` 字段决定了包管理器**要不要**在这个环境下下载和安装它。这类似于 `npm install --production` 跳过 `devDependencies`——不是库本身不能跑在生产环境，而是构建工具根据指令做了调度。

#### 6.4.2 行为规则

`env` 与 `--target` 的完整映射：

| 安装命令 | `env="client"` | `env="server"` | `env="both"` | 适用场景 |
|:---|:---:|:---:|:---:|:---|
| `orbit install` (默认) | ✅ 安装 | ✅ 安装 | ✅ 安装 | **单机生存玩家**（全要） |
| `orbit install --target client` | ✅ 安装 | ❌ 跳过 | ✅ 安装 | **纯联机玩家**（只连服，省内存） |
| `orbit install --target server` | ❌ 跳过 | ✅ 安装 | ✅ 安装 | **服主/运维**（开服，防崩溃） |

**分组行为**：

```
orbit install --group benchmark # 安装 benchmark 分组 + 未分组的通用依赖
orbit install --no-optional     # 跳过标记 optional = true 的依赖
```

`env` 和 `[groups]` 可以共存——一个依赖可以同时设置 `env = "client"` 并属于 `[groups.benchmark]`：

```toml
[dependencies]
sodium = { version = "*", env = "client" }

[groups.benchmark]
dependencies = ["sodium", "spark"]
```

#### 6.4.3 Lockfile 策略：Fat Lockfile

`orbit.lock` 始终保存**所有环境的完整依赖树**（Fat Lockfile），不论安装时使用了什么 `--target`。

- 每个 `[[package]]` 条目记录的是模组本身的属性（版本、hash、传递依赖），这些属性不随 target 变化。
- 安装阶段的过滤是 CLI 调度层的职责，不应污染 lock 文件。
- 这确保了另一台机器 clone 仓库后执行 `orbit install --target server` 时拿到完全一致的服务端依赖集合，无需重新解析。

### 6.5 冲突解决

当两个模组依赖同一个前置但要求不同的版本范围时：

1. 若能找到**同时满足两个范围**的版本，使用该版本。
2. 若范围不兼容，Orbit 报错，列出冲突的依赖和各自要求的版本范围。
3. 用户可以通过 `[overrides]` 强制指定版本来解决冲突。

### 6.6 `exclude` — 传递依赖排除

当一个模组声明了你不想要的传递依赖时（例如某个模组强制依赖了一个你确定不需要的配置屏幕库），使用 `exclude` 排除：

```toml
"some-bloated-mod" = { version = "*", exclude = ["annoying-library"] }
```

**语义**：

- `exclude` 仅阻止**自动安装**被排除的传递依赖，不会删除已安装的 jar。
- 被排除的依赖如果在 `[dependencies]` 中被显式声明（或作为其他模组的传递依赖被需要），仍然会被安装。
- `exclude` 不会修改上游模组的元数据声明；它只影响 Orbit 的安装决策。

> **警告**：排除依赖可能导致运行时 `ClassNotFoundException` 或 `NoClassDefFoundError`。仅在确认被排除的库确实无需时才使用。

---

## 7. 完整示例

### orbit.toml

```toml
[project]
name = "survival-plus"
mc_version = "1.21.5"
modloader = "fabric"
modloader_version = "0.16.10"
description = "优化原版体验的轻量整合包"
authors = ["GBwater"]
version = "1.0.0"

[resolver]
platforms = ["modrinth"]
prerelease = false

[dependencies]
# 键名 = JAR loader 元数据声明的 mod_id
# 简写形式 — 版本约束字符串
sodium = "^0.5"
lithium = ">=0.11 <0.14"
fabric-api = "*"

# 完整内联表形式 — version, optional, env, exclude
journeymap = { version = "^5.9", env = "client" }
jei = { version = "^12" }
zoomify = { version = "*", optional = true, env = "client" }

# 高级自定义分组 — benchmark 场景
[groups.benchmark]
dependencies = ["spark", "lithium"]
```

### 对应的 orbit.lock（节选）

```toml
[meta]
mc_version = "1.21.5"
modloader = "fabric"
modloader_version = "0.16.10"

[[package]]
mod_id = "sodium"
version = "0.8.10"
sha256 = "e3b0c442..."
filename = "sodium-0.8.10.jar"
provider = "modrinth"
environment = "client"

[package.modrinth]
project_id = "AANobbMI"
version_id = "SIrB5bCM"
slug = "sodium"
download_url = "https://cdn.modrinth.com/..."

[[package.dependencies]]
expression = "only"
[package.dependencies.value]
id = "minecraft"
requirement = ">=1.21.5"
kind = "required"
environment = "both"
ordering = "none"

[[package.bundled]]
mod_id = "sodium_internal"
version = "0.8.10"
environment = "both"
```

完整 lockfile 由 Orbit 序列化，不建议手写。示例省略了其他包、哈希和可选元数据；
真实文件会保留 loader JAR 中的完整依赖表达式。
