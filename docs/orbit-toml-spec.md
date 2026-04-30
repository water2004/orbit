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
   - [6.6 `type = "file"` 的 SHA-256 重算策略](#66-type--file-的-sha-256-重算策略)
   - [6.7 `exclude` — 传递依赖排除](#67-exclude--传递依赖排除)
7. [完整示例](#7-完整示例)

---

## 1. 概述

`orbit.toml` 是 Orbit 项目目录的**唯一真实数据源 (Single Source of Truth)**。它记录：

- **项目元数据**：Minecraft 版本、模组加载器类型
- **模组声明**：需要哪些模组、从哪里获取、版本约束
- **解析策略**：多平台优先级、预发布版本的取舍

`orbit.toml` 由**用户手动编辑**（或通过 `orbit install/remove` 等命令自动维护），纳入 Git 版本控制。

与之配套的 `orbit.lock` 由 Orbit **自动生成**，精确锁定每个依赖的版本、URL 和校验值，确保可复现安装。`orbit.lock` 应纳入 Git；用户不应手动编辑它。

---

## 2. 文件关系

```
.minecraft/           （或任意 Orbit 项目根目录）
├── orbit.toml        ← 声明式清单：用户编辑，表达意图
├── orbit.lock        ← 锁定文件：自动生成，记录事实
└── mods/
    ├── sodium-0.5.8.jar
    ├── jei-12.0.0.jar
    └── ...
```

| 文件 | 角色 | 类比 | 是否纳入 Git |
|------|------|------|--------------|
| `orbit.toml` | 开发者声明依赖意图（"我要 sodium >= 0.5"） | `Cargo.toml` / `package.json` | **是** |
| `orbit.lock` | Orbit 记录精确解析结果（"sodium 已解析为 0.5.8, sha256=abc"） | `Cargo.lock` / `package-lock.json` | **是** |

> **关键原则**：`orbit.toml` 表达 **What**（想要什么），`orbit.lock` 记录 **How**（具体是什么）。  
> `orbit install` 读取前者，写入后者；`orbit sync` 双向操作，以实际 `mods/` 目录为参考。

---

## 3. `orbit.toml` 完整 Schema

### 3.1 `[project]` — 项目元数据

```toml
[project]
name = "my-pack"                  # 必填。实例名称，唯一标识符，用于 `orbit instances` 管理
mc_version = "1.20.1"             # 必填。目标 Minecraft 版本，严格按 "主.次.补丁" 格式
modloader = "fabric"              # 必填。模组加载器
modloader_version = "0.15.7"      # 必填。加载器版本，直接影响 API 兼容性

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
| `modloader_version` | `String` | **是** | 加载器版本号，如 `0.15.7`。Fabric Loader / Forge 的 API 随此版本变动，锁定它是可复现构建的前提 |
| `description` | `String` | 否 | 自由文本，单行 |
| `authors` | `[String]` | 否 | 作者列表 |
| `version` | `String` | 否 | 整合包版本，推荐 semver |

---

### 3.2 `[resolver]` — 解析策略

```toml
[resolver]
platforms = ["modrinth", "curseforge"]   # 平台优先级，依次尝试直到找到匹配
prerelease = false                        # 是否使用 alpha/beta/预发布版本
```

**字段规范**：

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `platforms` | `[String]` | `["modrinth", "curseforge"]` | 当依赖未显式指定平台时，按此顺序查找 |
| `prerelease` | `bool` | `false` | `true` 时解析器会考虑 alpha/beta 版本 |

> **解析逻辑**：当用户在 `[dependencies]` 中只写 `sodium = "*"`（无 `platform` 字段），Orbit 依次查询 `platforms[0]` → `platforms[1]` → …，返回第一个匹配成功的结果。

---

### 3.3 `[dependencies]` — 模组依赖

每个依赖以 **键 = 值** 形式声明，值可以是简写字符串或内联表 (inline table)。

#### 3.3.1 简写形式

```toml
[dependencies]
# 通配符：接受任意版本，由 Orbit 自动选择最新兼容版
sodium = "*"

# 语义化版本约束
lithium = ">=0.11, <0.14"

# 精确版本（= 前缀）
iris = "=1.7.0"

# 脱字符约束（兼容同一大版本）
fabric-api = "^0.92"
```

当使用简写形式时，`platform` 由 `[resolver].platforms` 决定搜索顺序。

#### 3.3.2 内联表完整形式

```toml
[dependencies]
# 指定平台 + 版本约束
"fabric-api" = { platform = "modrinth", version = "^0.92" }

# 显式 mod ID / slug（当名称与平台内标识符不同时）
"jei" = { platform = "curseforge", slug = "238222", version = "^12" }

# 精确锁定，禁止升级
"iris" = { platform = "modrinth", version = "=1.7.0" }

# 可选依赖 — orbit install 默认安装，orbit install --no-optional 跳过
"zoomify" = { platform = "modrinth", optional = true }

# 环境限定：仅客户端 / 仅服务端（默认 both）
"inventory-hud" = { platform = "modrinth", env = "client" }
"spark" = { platform = "modrinth", env = "server" }

# 排除特定传递依赖（该模组强制依赖了一个你不需要的库）
"some-bloated-mod" = { platform = "modrinth", exclude = ["annoying-library"] }

# 本地文件
"my-custom-mod" = { type = "file", path = "mods/custom/mymod.jar" }

# 直链下载
"rare-mod" = { type = "url", url = "https://ci.example.com/latest.jar", sha256 = "e3b0c442..." }
```

#### 3.3.3 内联表字段全表

| 字段 | 类型 | 适用场景 | 说明 |
|------|------|----------|------|
| `platform` | `String` | 在线依赖 | 枚举：`modrinth` \| `curseforge`。若不指定，由 resolver 自动选择 |
| `slug` | `String` | 在线依赖 | 平台内唯一标识符。不指定时使用键名作为 slug |
| `version` | `String` | 在线依赖 | 版本约束表达式，详见 [§6.1](#61-版本约束语法) |
| `optional` | `bool` | 在线依赖 | 默认 `false`。标记为可选的模组会被 `--no-optional` 跳过 |
| `env` | `String` | 在线依赖 | 默认 `"both"`。运行环境限定：`"client"` \| `"server"` \| `"both"`。替代简单场景下的 `[groups]` |
| `exclude` | `[String]` | 在线依赖 | 排除指定传递依赖。填入要跳过的模组名称列表，防止上游夹带无用库 |
| `type` | `String` | 特殊来源 | `file` \| `url`。若不指定，默认走平台在线解析 |
| `path` | `String` | `type = "file"` | 相对于 `orbit.toml` 所在目录的路径 |
| `url` | `String` | `type = "url"` | 直链 URL |
| `sha256` | `String` | `type = "url"` | **(推荐)** 下载后的 SHA-256 校验值，用于安全校验 |

---

### 3.4 `[groups]` — 模组分组 (高级场景)

分组用于**自定义安装场景**（如基准测试、调试工具集等）。对于基础的端侧分离（客户端/服务端），**优先使用内联 `env` 字段**（见 [§3.3.3](#333-内联表字段全表)），内聚性更好，无需在两个表之间跳转。

```toml
# 基础端侧分离 — 用 env 字段（推荐）
# zoomify = { platform = "modrinth", env = "client" }
# spark   = { platform = "modrinth", env = "server" }

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
"fabric-api" = { platform = "modrinth", version = "=0.90.0" }
```

**使用约束**：
- `[overrides]` 中的条目格式与 `[dependencies]` 的内联表完全一致。
- 覆盖是**全局**的 — 即使某个依赖未被顶级 `[dependencies]` 声明（仅作为传递依赖出现），覆盖也会生效。
- 覆盖不能新增依赖；它只修改已有依赖的版本。
- **不推荐日常使用**。仅在遇到依赖冲突、上游未及时更新时使用。

---

## 4. `orbit.lock` 完整 Schema

```toml
# ============================================================
# 自动生成，禁止手动编辑
# 由 orbit install / orbit upgrade 维护
# ============================================================

[meta]
mc_version = "1.20.1"             # 锁定时的 MC 版本
modloader = "fabric"              # 锁定时的加载器
modloader_version = "0.15.7"      # 锁定时的加载器版本

# --- 每个已解析的模组一个 [[lock]] 条目 ---

[[lock]]
name = "sodium"                                    # 对应 orbit.toml [dependencies] 中的键名
platform = "modrinth"                              # 解析到的平台
mod_id = "AANobbMI"                                # 平台内唯一 ID
version = "0.5.8"                                  # 实际安装的版本号
filename = "sodium-fabric-mc1.20.1-0.5.8.jar"      # 文件名
url = "https://cdn.modrinth.com/data/AANobbMI/versions/abc123/sodium-fabric-mc1.20.1-0.5.8.jar"
sha256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
dependencies = [                                   # 此模组的前置依赖（含锁定版本）
    { name = "fabric-api", version = "0.92.0" },
]

[[lock]]
name = "my-custom-mod"
type = "file"
path = "mods/custom/mymod.jar"
sha256 = "a7ffc6f8bf1ed76651c14756a061d662f580ff4de43b49fa82d80a4b80f8434a"
dependencies = []

[[lock]]
name = "fabric-api"
platform = "modrinth"
mod_id = "P7dR8mSH"
version = "0.92.0"
filename = "fabric-api-0.92.0+1.20.1.jar"
url = "https://cdn.modrinth.com/data/P7dR8mSH/versions/def456/fabric-api-0.92.0+1.20.1.jar"
sha256 = "d7e9b8a71cfe2e3f5a6b1c8d9e0f1234567890abcdef0123456789abcdef012345"
dependencies = []
```

**`[[lock]]` 条目字段全表**：

| 字段 | 类型 | 说明 |
|------|------|------|
| `name` | `String` | 对应 `orbit.toml` `[dependencies]` 中的键名 |
| `platform` | `String` (可选) | `modrinth` \| `curseforge`。`type = "file"/"url"` 时不出现 |
| `type` | `String` (可选) | `file` \| `url`。平台在线依赖时不出现 |
| `mod_id` | `String` (可选) | 平台内项目 ID。type 为 file/url 时不出现 |
| `version` | `String` | 实际安装的版本字符串 |
| `filename` | `String` | jar 文件的磁盘名称 |
| `url` | `String` (可选) | 下载源 URL。type = file 时不出现 |
| `path` | `String` (可选) | 本地文件相对路径。仅 type = file 时出现 |
| `sha256` | `String` | SHA-256 校验值 |
| `dependencies` | `[{name, version}]` | 此模组的前置依赖列表，每个条目包含锁定版本号，防止上游元数据变更导致解析漂移 |

---

## 5. 字段速查表

### orbit.toml 顶级表一览

| 表 | 必填 | 说明 |
|----|------|------|
| `[project]` | **是** | 项目元数据 |
| `[resolver]` | 否 | 全局解析策略（有默认值） |
| `[dependencies]` | 否 | 模组依赖声明 |
| `[groups.<name>]` | 否 | 按场景分组 |
| `[overrides]` | 否 | 强制覆盖上游版本 |

### 依赖值的三种写法

| 写法 | 示例 | 含义 |
|------|------|------|
| 字符串 | `sodium = "*"` | 任何版本，平台由 resolver 决定 |
| 字符串 | `iris = "=1.7.0"` | 精确版本，平台由 resolver 决定 |
| 内联表 | `sodium = { platform = "modrinth", version = "^0.5" }` | 完整指定 |

---

## 6. 语义规则

### 6.1 版本约束语法

Orbit 的版本约束借鉴 Cargo/npm，但针对 Minecraft 模组的非标准版本号做了适配。

| 表达式 | 含义 | 示例（匹配/不匹配） |
|--------|------|---------------------|
| `*` | 任意版本 | 匹配一切 |
| `>=X.Y.Z, <A.B.C` | 范围约束 | `>=0.5, <1.0` |
| `^X.Y.Z` | 兼容更新（左起第一个非零位不变） | `^0.5.8` ≡ `>=0.5.8, <0.6.0` |
| `~X.Y.Z` | 补丁更新（仅允许最后一位变化） | `~0.5.8` ≡ `>=0.5.8, <0.5.9` |
| `=X.Y.Z` | 精确版本 | 仅匹配 `X.Y.Z` |

**MC 模组特殊处理**：许多模组版本形如 `mc1.20.1-0.5.8` 或 `1.20.1-0.5.8`。Orbit 解析时先提取版本号中**可解析为 semver 的尾部**进行约束比较。若无法提取，则回退为字符串精确匹配。

### 6.2 平台自动解析

当一条依赖未指定 `platform` 字段时：

1. 遍历 `[resolver].platforms` 列表。
2. 对每个平台，使用依赖的键名（或 `slug` 字段）作为平台内的搜索标识符。
3. 第一个返回有效结果的平台即为该依赖的来源。
4. 若所有平台均无匹配，`orbit install` 报错退出。

> **示例**：`platforms = ["modrinth", "curseforge"]`，依赖写 `jei = "*"`。先在 Modrinth 搜 `jei`；若未找到，再在 CurseForge 搜 `jei`。

### 6.3 依赖来源类型

| 类型 | `type` 值 | `platform` 字段 | jar 获取方式 |
|------|-----------|-----------------|-------------|
| 平台在线 | (不写) | 必填（或自动选择） | 通过平台的 API 下载 |
| 本地文件 | `"file"` | 不出现 | 直接使用本地路径的 jar |
| 直链下载 | `"url"` | 不出现 | HTTP GET 下载 + sha256 校验 |

> **互斥约束**：`type = "file"` 或 `type = "url"` 时，`platform`、`slug`、`version`（版本约束）字段**不得出现**。反之，平台在线依赖不出现 `type`、`path`、`url`（`platform` 可省略，由 resolver 决定）。

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
sodium = { platform = "modrinth", env = "client" }

[groups.benchmark]
dependencies = ["sodium", "spark"]
```

#### 6.4.3 Lockfile 策略：Fat Lockfile

`orbit.lock` 始终保存**所有环境的完整依赖树**（Fat Lockfile），不论安装时使用了什么 `--target`。

- 每个 `[[lock]]` 条目记录的是模组本身的属性（版本、hash、传递依赖），这些属性不随 target 变化。
- 安装阶段的过滤是 CLI 调度层的职责，不应污染 lock 文件。
- 这确保了另一台机器 clone 仓库后执行 `orbit install --target server` 时拿到完全一致的服务端依赖集合，无需重新解析。

### 6.5 冲突解决

当两个模组依赖同一个前置但要求不同的版本范围时：

1. 若能找到**同时满足两个范围**的版本，使用该版本。
2. 若范围不兼容，Orbit 报错，列出冲突的依赖和各自要求的版本范围。
3. 用户可以通过 `[overrides]` 强制指定版本来解决冲突。

### 6.6 `type = "file"` 的 SHA-256 重算策略

本地文件依赖 (`type = "file"`) 的 `sha256` **并非静态配置**，而是每次关键操作时重新计算：

- `orbit install`：对每个 `type = "file"` 依赖计算 SHA-256，与 `orbit.lock` 中记录的值比对。若有差异，说明文件已更新，自动更新 lock 中的 `sha256` 和 `version`。
- `orbit sync`：同样触发重算，确保 lock 文件反映磁盘上 jar 的真实状态。
- 这保证了**他人拉取代码后执行 `orbit install` 时，文件变更能被正确检测**，避免因文件名未变而遗漏更新。

> **设计意图**：本地文件依赖没有远程版本号可查询，SHA-256 是唯一可验证的"版本"标识。每次操作重算确保了可复现构建的一致性。

### 6.7 `exclude` — 传递依赖排除

当一个模组声明了你不想要的传递依赖时（例如某个模组强制依赖了一个你确定不需要的配置屏幕库），使用 `exclude` 排除：

```toml
"some-bloated-mod" = { platform = "modrinth", exclude = ["annoying-library"] }
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
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.15.7"
description = "优化原版体验的轻量整合包"
authors = ["GBwater"]
version = "1.0.0"

[resolver]
platforms = ["modrinth", "curseforge"]
prerelease = false

[dependencies]
# 性能三件套 — 不指定平台，让 resolver 自动匹配
sodium = "^0.5"
lithium = ">=0.11, <0.14"
phosphor = { platform = "modrinth" }

# 小地图 — 客户端专用
"journeymap" = { platform = "curseforge", version = "^5.9", env = "client" }

# JEI — 显式指定 CurseForge 上的 slug
"jei" = { platform = "curseforge", slug = "238222", version = "^12" }

# 可选：缩放 mod（客户端）
"zoomify" = { platform = "modrinth", optional = true, env = "client" }

# 服务端性能分析
"spark" = { platform = "modrinth", env = "server" }

# 某大型模组带了不需要的库，排除之
"big-pack" = { platform = "curseforge", exclude = ["redundant-lib"] }

# 自己做的补丁 mod
"my-tweaks" = { type = "file", path = "mods/local/mytweaks-1.0.jar" }

# 高级自定义分组 — benchmark 场景
[groups.benchmark]
dependencies = ["spark", "lithium"]
```

### 对应的 orbit.lock（安装后自动生成）

```toml
[meta]
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.15.7"

[[lock]]
name = "sodium"
platform = "modrinth"
mod_id = "AANobbMI"
version = "0.5.8"
filename = "sodium-fabric-mc1.20.1-0.5.8.jar"
url = "https://cdn.modrinth.com/data/AANobbMI/versions/u4ZoB70l/sodium-fabric-mc1.20.1-0.5.8.jar"
sha256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
dependencies = []

[[lock]]
name = "lithium"
platform = "modrinth"
mod_id = "gvQqBUqZ"
version = "0.12.1"
filename = "lithium-fabric-mc1.20.1-0.12.1.jar"
url = "https://cdn.modrinth.com/data/gvQqBUqZ/versions/x98ZyK1m/lithium-fabric-mc1.20.1-0.12.1.jar"
sha256 = "aa1b2c3d4e5f67890123456789abcdef0123456789abcdef0123456789abcdef01"
dependencies = []

[[lock]]
name = "phosphor"
platform = "modrinth"
mod_id = "hEOCdOgW"
version = "0.2.0"
filename = "phosphor-fabric-mc1.20.1-0.2.0.jar"
url = "https://cdn.modrinth.com/data/hEOCdOgW/versions/p3Rq7Sm2/phosphor-fabric-mc1.20.1-0.2.0.jar"
sha256 = "bb2c3d4e5f67890123456789abcdef0123456789abcdef0123456789abcdef0123"
dependencies = []

[[lock]]
name = "journeymap"
platform = "curseforge"
mod_id = "32274"
version = "5.9.18"
filename = "journeymap-1.20.1-5.9.18-fabric.jar"
url = "https://edge.forgecdn.net/files/.../journeymap-1.20.1-5.9.18-fabric.jar"
sha256 = "cc3d4e5f67890123456789abcdef0123456789abcdef0123456789abcdef0123456"
dependencies = []

[[lock]]
name = "jei"
platform = "curseforge"
mod_id = "238222"
version = "12.0.0"
filename = "jei-1.20.1-fabric-12.0.0.jar"
url = "https://edge.forgecdn.net/files/.../jei-1.20.1-fabric-12.0.0.jar"
sha256 = "dd4e5f67890123456789abcdef0123456789abcdef0123456789abcdef012345678"
dependencies = []

[[lock]]
name = "zoomify"
platform = "modrinth"
mod_id = "w7ThoJFB"
version = "2.11.1"
filename = "zoomify-2.11.1.jar"
url = "https://cdn.modrinth.com/data/w7ThoJFB/versions/y8Lt4WnX/zoomify-2.11.1.jar"
sha256 = "ee5f67890123456789abcdef0123456789abcdef0123456789abcdef0123456789a"
dependencies = []

[[lock]]
name = "spark"
platform = "modrinth"
mod_id = "l6YH9Als"
version = "1.10.53"
filename = "spark-1.10.53-fabric.jar"
url = "https://cdn.modrinth.com/data/l6YH9Als/versions/z1M4Kp8w/spark-1.10.53-fabric.jar"
sha256 = "99a67890123456789abcdef0123456789abcdef0123456789abcdef0123456789a"
dependencies = []

[[lock]]
name = "big-pack"
platform = "curseforge"
mod_id = "99999"
version = "3.2.0"
filename = "big-pack-3.2.0.jar"
url = "https://edge.forgecdn.net/files/.../big-pack-3.2.0.jar"
sha256 = "0af67890123456789abcdef0123456789abcdef0123456789abcdef0123456789b"
dependencies = [
    { name = "useful-lib", version = "2.1.0" },
    # 注意：redundant-lib 已被 exclude，不出现在此处
]

[[lock]]
name = "my-tweaks"
type = "file"
path = "mods/local/mytweaks-1.0.jar"
version = "1.0"
filename = "mytweaks-1.0.jar"
sha256 = "ff67890123456789abcdef0123456789abcdef0123456789abcdef0123456789abc"
dependencies = []
```

---
