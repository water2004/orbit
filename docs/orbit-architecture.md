# Orbit 项目结构文档

> 本文档定义 Orbit Monorepo 的目录布局、各 crate 职责边界、内部依赖关系及核心抽象接口。
> 这是项目开发的**唯一架构参照**——所有模块边界和依赖方向必须与此文档一致。
> 文中“不得包含”和“依赖铁律”是规范；若当前代码不满足，应记录为实现缺口，不能反向修改规范。

---

## 目录

1. [设计总览](#1-设计总览)
2. [目录布局](#2-目录布局)
3. [根 Cargo.toml 工作区配置](#3-根-cargotoml-工作区配置)
4. [Crate 职责定义](#4-crate-职责定义)
   - [modrinth-wrapper](#modrinth-wrapper)
   - [curseforge-wrapper](#curseforge-wrapper)
   - [orbit-core](#orbit-core)
   - [orbit-cli](#orbit-cli)
5. [架构分层与依赖方向](#5-架构分层与依赖方向)
6. [orbit-core 内部模块](#6-orbit-core-内部模块)
7. [核心抽象：Provider 特质](#7-核心抽象provider-特质)
8. [Provider 实现示例](#8-provider-实现示例)
9. [数据流全景](#9-数据流全景)
10. [历史迁移路线与剩余工作](#10-历史迁移路线与剩余工作)

---

## 1. 设计总览

Orbit 采用 **Monorepo + 分层架构**。当前 workspace 有三个 crate；另有一个暂时排除在
workspace 外的定制 PubGrub 源码目录：

```
                    ┌──────────────┐
                    │  orbit-cli   │  ← 极薄 CLI 层：clap 解析 + 格式化输出 + 进度条
                    └──────┬───────┘
                           │ 依赖
                    ┌──────▼───────┐
                    │  orbit-core  │  ← 纯业务逻辑：TOML 解析、依赖解决、sync 算法
                    └──────┬───────┘
                           │ 依赖
              ┌────────────┴────────────┐
              │                         │
    ┌─────────▼────────┐    ┌──────────▼─────────┐
    │ modrinth-wrapper │    │ curseforge-wrapper  │  ← 纯 SDK：HTTP 请求 + JSON 反序列化
    └──────────────────┘    └────────────────────┘
```

`curseforge-wrapper` 是目标架构，当前尚未创建。`orbit-core` 还通过 path 依赖
`pubgrub-fork`；等 fork 远端和发布方式稳定后，应改为可复现的远端 revision 或发布版本。

**核心原则**：

- **Wrapper 不知道 Orbit 的存在**。它们是通用的、可独立发布的平台 API 客户端。
- **orbit-core 不知道 CLI 的存在**。它是一组纯函数和异步 trait，可以被 CLI、GUI、或 Web 服务复用。
- **orbit-cli 不包含任何业务逻辑**。它只负责解析命令行参数，调用 `orbit-core`，然后格式化输出。

---

## 2. 目录布局

```
ORBIT/
├── Cargo.toml                    # 根工作区配置 (定义 members, workspace.dependencies)
├── Cargo.lock                    # 整个工作区共享的锁文件
├── pubgrub-fork/                 # 定制 PubGrub 0.4（暂时 exclude，不是 workspace member）
├── README.md
├── LICENSE
├── docs/                         # 设计文档
│   ├── orbit-toml-spec.md        #   orbit.toml / orbit.lock 格式规格
│   ├── orbit-global-config.md    #   config.toml 全局配置规格
│   ├── orbit-cli-commands.md     #   命令行为规格
│   ├── orbit-metadata.md         #   文件元数据解析层设计
│   ├── orbit-detection.md        #   实例环境检测层设计
│   ├── orbit-architecture.md     #   本文档
│   └── orbit-status.md           #   项目完成度追踪
│
├── modrinth-wrapper/             # 🧩 Modrinth API v2 客户端 (独立发布)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs                #   公共 API 入口
│       ├── client.rs             #   HTTP 客户端构造
│       ├── api.rs                #   所有 API 端点方法
│       ├── models.rs             #   Project, Version, SearchHit 等结构体
│       └── error.rs              #   ModrinthError 枚举
│
├── curseforge-wrapper/           # 🧩 CurseForge API 客户端 (独立发布, 待创建)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── client.rs
│       ├── api.rs
│       ├── models.rs
│       └── error.rs
│
├── orbit-core/                   # 🧠 业务逻辑层 (可独立发布)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs                #   公共 API 入口，暴露核心类型
│       ├── manifest.rs           #   orbit.toml 解析 + mc_version_from_dir()
│       ├── lockfile.rs           #   orbit.lock 读写 (PackageEntry with mod_id/version/hashes/provider/modrinth sub-table)
│       ├── identification.rs     #   模组来源识别 (批量哈希反查)
│       ├── init.rs               #   init 编排: scan_mods_dir + run_init
│       ├── versions/             #   版本号解析 (PubGrub 集成)
│       │   ├── mod.rs            #     Version enum + parse() + parse_constraint()
│       │   └── fabric.rs         #     SemanticVersion + satisfies() + parse_constraint()
│       ├── resolver/             #   依赖解析引擎 (PubGrub)
│       │   ├── mod.rs            #     公共 API + 顶层编排
│       │   ├── graph.rs          #     求解图构建与候选注册
│       │   ├── retry.rs          #     求解循环与缺失依赖补抓
│       │   ├── local.rs          #     纯本地依赖图校验
│       │   ├── diagnostics/      #     SolverEvent 采集、解释渲染和契约测试
│       │   ├── provider.rs       #     OrbitDependencyProvider + ProviderError
│       │   └── types.rs          #     PackageId + CandidateVersion + ImplantedCandidate
│       ├── sync.rs               #   双向同步 (Err 占位)
│       ├── installer.rs          #   install_to_instance + remove_from_instance
│       ├── outdated.rs           #   候选发现、BFS 下载与可升级集合查询
│       ├── checker.rs            #   跨版本预检 (Err 占位)
│       ├── purge.rs              #   深度清理 (Err 占位)
│       ├── workspace.rs          #   ManifestFile / Lockfile 文件 I/O 封装
│       ├── jar_cache.rs          #   全局 JAR 缓存（SHA-1/256/512 索引） 
│       ├── jar/                  #   JAR 文件处理 (哈希 + loader 分发元数据提取)
│       │   ├── mod.rs            #     JarModMetadata + 哈希函数 + loader 分发
│       │   └── fabric.rs         #     fabric.mod.json 查找与解析 (ZIP → 内容)
│       ├── metadata/             #   文件格式解析 (纯解析，无 I/O)
│       │   ├── mod.rs            #     MetadataParser trait + ModMetadata + Extractor
│       │   ├── fabric.rs         #     fabric.mod.json (JSON)
│       │   ├── forge.rs          #     META-INF/mods.toml (TOML) [future]
│       │   ├── neoforge.rs       #     META-INF/neoforge.mods.toml (TOML) [future]
│       │   ├── quilt.rs          #     quilt.mod.json (JSON) [future]
│       │   ├── mojang.rs         #     version.json (JSON, 纯函数)
│       │   └── version_profile.rs #     launcher 版本 JSON (libraries 列表)
│       ├── detection/            #   实例环境检测 (策略模式)
│       │   ├── mod.rs            #     LoaderDetector trait + LoaderDetectionService
│       │   ├── fabric.rs         #     FabricDetector
│       │   ├── forge.rs          #     ForgeDetector [future]
│       │   ├── neoforge.rs       #     NeoForgeDetector [future]
│       │   └── quilt.rs          #     QuiltDetector [future]
│       ├── providers/            #   平台 Provider 特质 + 各平台实现
│       │   ├── mod.rs            #     ModProvider trait, ResolvedMod 等核心类型
│       │   ├── rate_limiter.rs   #     RateLimiter — Semaphore 并发控制
│       │   ├── modrinth.rs       #     ModrinthProvider (封装 modrinth-wrapper)
│       │   └── curseforge.rs     #     CurseForgeProvider (封装 curseforge-wrapper)
│       ├── error.rs              #   Orbit 统一错误类型
│       └── config.rs             #   全局 Orbit 配置 (~/.orbit/instances.toml 等)
│
└── orbit-cli/                    # 💻 CLI 入口 (极薄层)
    ├── Cargo.toml
    └── src/
        ├── main.rs               #   程序入口
        ├── cli.rs                #   clap 命令定义
        └── commands/             #   每个命令一个文件，仅负责调用 orbit-core
            ├── mod.rs
            ├── init.rs           #   → core::manifest + core::jar
            ├── add.rs            #   → core::resolver + core::installer
            ├── install.rs        #   → core::resolver + core::installer
            ├── remove.rs         #   → core::manifest + fs 删除
            ├── purge.rs          #   → core::purge
            ├── sync.rs           #   → core::sync
            ├── outdated.rs       #   → core::resolver (只读)
            ├── upgrade.rs        #   → core::resolver + core::installer
            ├── search.rs         #   → providers (只读)
            ├── info.rs           #   → providers (只读)
            ├── list.rs           #   → core::lockfile (只读)
            ├── check.rs          #   → core::checker
            ├── import_cmd.rs     #   → core::manifest (合并)
            ├── export_cmd.rs     #   → core::lockfile + zip
            ├── instances.rs      #   → core::config
            └── cache.rs          #   → fs 操作
```

---

## 3. 根 Cargo.toml 工作区配置

```toml
[workspace]
members = [
    "orbit-cli",
    "orbit-core",
    "modrinth-wrapper",
]
exclude = ["pubgrub-fork"]
resolver = "2"

[workspace.dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
toml = "0.8"
sha2 = "0.10"
hex = "0.4"
reqwest = { version = "0.13.2", features = ["json"] }
tokio = { version = "1.51.0", features = ["macros", "rt-multi-thread", "sync"] }
async-trait = "0.1"
clap = { version = "4.6.0", features = ["derive"] }
anyhow = "1.0.102"
thiserror = "2.0.18"
url = "2.5.8"
zip = "3.0.0"
indexmap = { version = "2.0", features = ["serde"] }
```

PubGrub 当前不是 workspace dependency。`orbit-core/Cargo.toml` 直接使用
`pubgrub = { path = "../pubgrub-fork" }`，对应带类型化 solver observer 的定制
`0.4.0`。这是远端确定前的过渡接法，不应误写成 crates.io 的 `0.5`。

---

## 4. Crate 职责定义

### modrinth-wrapper

**定位**：通用的、平台无关的 Modrinth API v2 Rust 客户端。可独立发布到 crates.io。

**职责**：
- 构造 HTTP 请求（User-Agent、Base URL、API Key header）
- 所有 Modrinth API 端点的方法封装（项目查询、版本列表、hash 反查、搜索、分类等）
- 将 API 返回的 JSON 反序列化为 Rust 结构体
- 错误处理：将 HTTP 错误 / JSON 解析错误转换为 `ModrinthError`

**不得包含**：
- 任何 Minecraft 整合包管理概念（orbit.toml、lock 文件、sync 算法等）
- 对其他 wrapper 或 `orbit-core` 的依赖
- 任何 CLI 或 UI 代码

**`Cargo.toml` 依赖**（现有）：
```toml
[package]
name = "modrinth-wrapper"
version = "0.1.0"
edition = "2024"

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
reqwest = { workspace = true }
tokio = { workspace = true }
url = { workspace = true }
thiserror = { workspace = true }
```

---

### curseforge-wrapper

**定位**：通用的、平台无关的 CurseForge API Rust 客户端。可独立发布到 crates.io。

**职责**：与 `modrinth-wrapper` 对称——封装 CurseForge API 的所有端点。

**当前状态**：尚未创建。旧的 `orbit-cli/src/adaptors/` 已删除；
`orbit-core/src/providers/curseforge.rs` 仅保留返回未实现错误的骨架。

**`Cargo.toml` 依赖**（规划）：
```toml
[package]
name = "curseforge-wrapper"
version = "0.1.0"
edition = "2024"

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
reqwest = { workspace = true }
tokio = { workspace = true }
url = { workspace = true }
thiserror = { workspace = true }
```

---

### orbit-core

**定位**：Orbit 的全部业务逻辑。目标是可独立发布到 crates.io（允许第三方构建自己的
Orbit 前端）；当前本地 `pubgrub-fork` path 依赖尚未满足这一发布条件。

**职责**：
- `orbit.toml` 和 `orbit.lock` 的解析、序列化、校验
- 依赖解析引擎（版本约束匹配、传递依赖展开、冲突检测）
- 双向同步算法（五态比对：NEW / MISSING / CHANGED / UNLOCKED / OK）
- 模组下载与安装逻辑
- 跨版本升级预检
- 深度清理启发式搜索
- JAR 文件元数据提取与哈希计算
- 全局配置管理（`~/.orbit/instances.toml`、缓存目录）
- 定义统一的 `ModProvider` trait
- 封装各 wrapper 为 provider 实现，完成平台数据到 Orbit 内部类型的转换

**不得包含**：
- 任何 CLI 参数解析（clap derive 宏等）
- 打印格式化输出、进度条、颜色
- 对 `orbit-cli` 的反向依赖

**`Cargo.toml` 依赖**（规划）：
```toml
[package]
name = "orbit-core"
version = "0.1.0"
edition = "2024"

[dependencies]
# 内部依赖：通过 path 引用本地 wrapper 源码
modrinth-wrapper = { path = "../modrinth-wrapper", version = "0.1.0" }
# curseforge-wrapper = { path = "../curseforge-wrapper", version = "0.1.0" }

# 工作区共享依赖
tokio = { workspace = true }
reqwest = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
toml = { workspace = true }
sha2 = { workspace = true }
hex = { workspace = true }
url = { workspace = true }
zip = { workspace = true }
async-trait = { workspace = true }
anyhow = { workspace = true }
thiserror = { workspace = true }
pubgrub = { path = "../pubgrub-fork" } # 过渡状态；远端确定后固定 revision
```

> **关于 path + version 双重指定的说明**：当 `orbit-core` 在 Workspace 内开发时，Cargo 使用 `path` 指向本地源码（wrapper 修改立即生效）。当 `orbit-core` 作为独立 crate 发布到 crates.io 时，Cargo 忽略 `path`，仅使用 `version` 从注册中心拉取 `modrinth-wrapper`。这是 Monorepo 的标准实践。

---

### orbit-cli

**定位**：Orbit 的命令行入口。极薄层，所有逻辑委托给 `orbit-core`。

**职责**：
- 使用 clap derive 定义命令结构
- 解析命令行参数
- 调用 `orbit-core` 的对应函数
- 格式化输出（表格、树状、颜色、进度条）
- 处理交互式确认提示
- 全局标志（`--verbose`、`--quiet`、`--yes`、`--dry-run`）的实现

**不得包含**：
- TOML 解析逻辑
- 依赖解析算法
- HTTP 请求（除了通过 `orbit-core`）
- 文件哈希计算

**`Cargo.toml` 依赖**（规划）：
```toml
[package]
name = "orbit"
version = "0.1.0"
edition = "2024"

[[bin]]
name = "orbit"
path = "src/main.rs"

[dependencies]
orbit-core = { path = "../orbit-core", version = "0.1.0" }

clap = { workspace = true }
tokio = { workspace = true }
anyhow = { workspace = true }
```

> 注意：`orbit-cli` **不再直接依赖** `modrinth-wrapper` 或 `curseforge-wrapper`。所有平台交互通过 `orbit-core` 的 provider 层中转。

---

## 5. 架构分层与依赖方向

```
┌─────────────────────────────────────────────┐
│                 orbit-cli                    │
│   clap 定义 → 调用 orbit-core → 格式化输出    │
└─────────────────┬───────────────────────────┘
                  │ 依赖 (path)
┌─────────────────▼───────────────────────────┐
│                orbit-core                    │
│   manifest │ resolver │ sync │ installer     │
│   providers/modrinth │ providers/curseforge  │
└────────┬──────────────────────┬─────────────┘
         │ 依赖 (path)           │ 依赖 (path)
┌────────▼──────────┐  ┌────────▼──────────┐
│ modrinth-wrapper  │  │ curseforge-wrapper │
│ (纯 HTTP + JSON)  │  │ (纯 HTTP + JSON)   │
└───────────────────┘  └───────────────────┘
```

**依赖铁律**：
- 依赖方向严格向下：`cli → core → wrapper`
- wrapper 之间不得互相依赖
- 不得出现循环依赖
- `core` 不得依赖 `cli`
- wrapper 不得依赖 `core` 或 `cli`

这些规则是规范，不是现状快照。当前 core 已不再直接写 stdout/stderr，provider
选择和用户可见诊断也由编排结果交给 CLI。仍存在的发布边界是：CurseForge provider
尚未实现，以及本地 PubGrub path 依赖在上游远端接入前尚不可独立发布。

---

## 6. orbit-core 内部模块

### 6.1 模块依赖图

```
lib.rs                    ← 公共 API 入口，暴露 install_to_instance / remove_from_instance 等
├── manifest.rs           ← orbit.toml serde + mc_version_from_dir()
├── lockfile.rs           ← orbit.lock serde (PackageEntry with mod_id/version/hashes/provider/modrinth sub-table)
├── identification.rs     ← 模组来源识别 (批量 hash 反查)
├── init.rs               ← init 编排: scan_mods_dir + run_init (JAR id/version as source of truth)
├── versions/             ← Version enum + parse + parse_constraint
│   └── fabric.rs         ← SemanticVersion + satisfies()
├── resolver/             ← 定制 PubGrub 求解器
│   ├── mod.rs            ← resolve_with_candidates + check_local_graph + 查询 API
│   ├── graph.rs          ← 求解图构建、候选与 lockfile 真实依赖注入
│   ├── retry.rs          ← resolve_with_observer + 缺失依赖补抓
│   ├── local.rs          ← 不联网的本地安装图校验
│   ├── diagnostics/      ← 类型化事件采集、领域渲染和测试
│   ├── provider.rs       ← OrbitDependencyProvider + ProviderError
│   └── types.rs          ← 候选输入 + ResolutionReport / CandidateDiagnostic
├── sync.rs               ← 双向同步 (Err 占位)
├── installer.rs          ← install_to_instance + remove_from_instance
├── outdated.rs           ← 候选 BFS 下载 + 可升级集合查询
├── checker.rs            ← 跨版本预检 (Err 占位)
├── purge.rs              ← 深度清理 (Err 占位)
├── workspace.rs          ← ManifestFile / Lockfile 文件 I/O 封装
├── jar_cache.rs          ← 全局 JAR 缓存（SHA-1/256/512 索引）
├── jar/                  ← JAR 文件处理 (哈希 + loader 分发元数据提取)
│   ├── mod.rs            ← JarModMetadata + 哈希 + loader 分发
│   └── fabric.rs         ← fabric.mod.json 查找与解析
├── metadata/             ← 文件格式解析 (纯解析，无 I/O)
│   ├── mod.rs            ← MetadataParser trait + ModMetadata + Extractor
│   ├── fabric.rs         ← fabric.mod.json
│   └── mojang.rs         ← version.json (纯函数)
├── detection/            ← 实例环境检测 (策略模式)
│   ├── mod.rs            ← LoaderDetector trait + LoaderDetectionService
│   └── fabric.rs         ← FabricDetector
├── config.rs             ← 全局配置 (GlobalConfig 待实际接入)
├── error.rs              ← 统一错误类型 (OrbitError)
└── providers/
    ├── mod.rs            ← ModProvider trait + create_providers() 工厂
    ├── rate_limiter.rs   ← RateLimiter (acquire 返回 Result)
    ├── modrinth.rs       ← ModrinthProvider（批量 API + version_constraint + slug 解析 + date_published 填充）
    └── curseforge.rs     ← CurseForgeProvider (Err 占位)
```

### 6.2 关键类型

**`manifest.rs` — orbit.toml 的 Rust 表示**：

```rust
// orbit-core/src/manifest.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrbitManifest {
    pub project: ProjectMeta,
    #[serde(default)]
    pub resolver: ResolverConfig,
    #[serde(default)]
    pub dependencies: indexmap::IndexMap<String, DependencySpec>,
    #[serde(default)]
    pub groups: indexmap::IndexMap<String, GroupSpec>,
    #[serde(default)]
    pub overrides: indexmap::IndexMap<String, DependencySpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMeta {
    pub name: String,
    pub mc_version: String,
    pub modloader: String,
    pub modloader_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authors: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolverConfig {
    #[serde(default = "default_platforms")]
    pub platforms: Vec<String>,
    #[serde(default)]
    pub prerelease: bool,
}

fn default_platforms() -> Vec<String> {
    vec!["modrinth".into(), "curseforge".into()]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DependencySpec {
    /// 简写形式：`sodium = "*"` 或 `sodium = "^0.5"`
    Short(String),
    /// 完整内联表形式
    Full {
        #[serde(skip_serializing_if = "Option::is_none")]
        platform: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        slug: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        version: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        optional: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        env: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        exclude: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none", rename = "type")]
        source_type: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        sha256: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupSpec {
    pub dependencies: Vec<String>,
}
```

**`lockfile.rs` — orbit.lock 的 Rust 表示**：

```rust
// orbit-core/src/lockfile.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrbitLockfile {
    pub meta: LockMeta,
    #[serde(rename = "lock")]
    pub entries: Vec<LockEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockMeta {
    pub mc_version: String,
    pub modloader: String,
    pub modloader_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockEntry {
    pub name: String,
    pub version: String,
    pub filename: String,
    pub sha256: String,
    pub dependencies: Vec<LockDependency>,

    // 平台在线依赖字段
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mod_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    // 本地/直链依赖字段
    #[serde(skip_serializing_if = "Option::is_none", rename = "type")]
    pub source_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockDependency {
    pub name: String,
    pub version: String,
}
```

**`error.rs` — 统一错误类型**：

```rust
// orbit-core/src/error.rs
use thiserror::Error;

#[derive(Error, Debug)]
pub enum OrbitError {
    #[error("orbit.toml not found in this directory")]
    ManifestNotFound,

    #[error("failed to parse orbit.toml: {0}")]
    ManifestParse(#[from] toml::de::Error),

    #[error("mod '{0}' not found")]
    ModNotFound(String),

    #[error("no version of '{mod_name}' satisfies constraint '{constraint}'")]
    VersionMismatch { mod_name: String, constraint: String },

    #[error("dependency conflict: {0}")]
    Conflict(String),

    #[error("checksum mismatch for '{name}': expected {expected}, got {actual}")]
    ChecksumMismatch { name: String, expected: String, actual: String },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
```

### 6.3 并发下载设计 (Installer Concurrency)

`orbit install` 全量安装时可能涉及数十甚至上百个模组的下载。单线程排队不可接受。`installer.rs` 应暴露支持并发的下载接口，使用 `tokio::task::JoinSet` 或 `futures::stream::StreamExt::buffer_unordered` 控制并发度（建议默认 8 个并发连接）。

```rust
// orbit-core/src/installer.rs 示意
use tokio::task::JoinSet;

pub async fn install_all(
    mods: Vec<ResolvedMod>,
    concurrency: usize,
) -> Result<InstallReport, OrbitError> {
    let mut set = JoinSet::new();
    let mut report = InstallReport::default();

    for m in mods {
        set.spawn(async move {
            download_and_verify(m).await
        });
        if set.len() >= concurrency {
            let result = set.join_next().await.unwrap()?;
            report.record(result);
        }
    }
    // 排空剩余任务
    while let Some(result) = set.join_next().await {
        report.record(result?);
    }
    Ok(report)
}

async fn download_and_verify(m: ResolvedMod) -> Result<InstalledMod, OrbitError> {
    let bytes = reqwest::get(&m.download_url).await?.bytes().await?;
    let sha256 = sha256_digest(&bytes);
    if sha256 != m.sha256 {
        return Err(OrbitError::ChecksumMismatch { ... });
    }
    let path = format!("mods/{}", m.filename);
    tokio::fs::write(&path, &bytes).await?;
    Ok(InstalledMod { name: m.name, version: m.version, sha256 })
}
```

CLI 层（`orbit-cli`）可在此并发模型上挂载 `indicatif::MultiProgress` 进度条，每个下载任务对应一个进度条子项。

### 6.4 错误处理分层

Orbit 采用两层错误模型：

| 层 | 错误类型 | 职责 |
|----|---------|------|
| `orbit-core` | `OrbitError` (thiserror) | 定义所有业务错误枚举，提供结构化错误信息 |
| `orbit-cli` | `anyhow::Result<()>` | 捕获核心错误，附加上下文后以友好格式输出到 stderr |

**`orbit-cli/src/main.rs` 模式**：

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command.execute().await {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("✗ {}", e);
            // anyhow 的链式错误上下文自动包含原始 OrbitError 信息
            std::process::exit(1);
        }
    }
}
```

`anyhow` 配合 `thiserror` 的 `#[error(transparent)]` 变体，可以自动将底层的 `OrbitError::VersionMismatch` 等变体透传为带上下文的友好报错，而非 Rust panic 堆栈。

---

## 7. 核心抽象：Provider 特质

`orbit-core/src/providers/mod.rs` 定义了 Orbit 与各个平台 SDK 之间的统一接口。

**为什么需要 `async-trait`**：resolver 需要将多个 provider 存在一个 `Vec<Box<dyn ModProvider>>` 中按 `[resolver].platforms` 顺序轮询。Rust 原生的 `async fn` 在 trait 中构建 `dyn Trait` 对象时有编译器限制，因此采用 `#[async_trait]` 宏来消除这些限制，使动态分发成为可能。

```rust
// orbit-core/src/providers/mod.rs
use async_trait::async_trait;
use crate::error::OrbitError;

/// 平台解析后的统一模组信息
#[derive(Debug, Clone)]
pub struct ResolvedMod {
    /// Orbit 依赖树中的名称（即 orbit.toml 的键名）
    pub name: String,
    /// 平台内唯一标识符
    pub mod_id: String,
    /// 实际安装的版本号
    pub version: String,
    /// 下载 URL
    pub download_url: String,
    /// jar 文件名
    pub filename: String,
    /// SHA-256 校验值
    pub sha256: String,
    /// 前置依赖列表
    pub dependencies: Vec<ResolvedDependency>,
    /// 平台元数据声明的 client_side（用于比对 env 覆盖）
    pub client_side: Option<SideSupport>,
    /// 平台元数据声明的 server_side
    pub server_side: Option<SideSupport>,
}

#[derive(Debug, Clone)]
pub struct ResolvedDependency {
    pub name: String,
    pub slug: Option<String>,
    pub required: bool,
}

#[derive(Debug, Clone)]
pub enum SideSupport {
    Required,
    Optional,
    Unsupported,
}

/// 统一搜索返回结果
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub mod_id: String,
    pub slug: String,
    pub name: String,
    pub description: String,
    pub latest_version: String,
    pub downloads: u64,
    pub mc_versions: Vec<String>,
    pub client_side: Option<SideSupport>,
    pub server_side: Option<SideSupport>,
    pub categories: Vec<String>,
}

/// 统一平台提供者特质
///
/// 每个支持的平台（Modrinth、CurseForge）各自实现此 trait。
/// orbit-core 的其他模块仅依赖此 trait，不依赖具体平台的 SDK。
#[async_trait]
pub trait ModProvider: Send + Sync {
    /// 提供者名称（"modrinth", "curseforge"）
    fn name(&self) -> &'static str;

    /// 搜索模组
    async fn search(
        &self,
        query: &str,
        mc_version: Option<&str>,
        loader: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SearchResult>, OrbitError>;

    /// 获取模组详情（用于 orbit info）
    async fn get_mod_info(&self, slug: &str) -> Result<ModInfo, OrbitError>;

    /// 解析模组：根据 slug 和版本约束，找到最匹配的版本
    async fn resolve(
        &self,
        slug: &str,
        version_constraint: &str,
        mc_version: &str,
        loader: &str,
    ) -> Result<ResolvedMod, OrbitError>;

    /// 根据 SHA-256 哈希反查版本
    async fn get_version_by_hash(
        &self,
        hash: &str,
    ) -> Result<Option<ResolvedMod>, OrbitError>;

    /// 获取模组的所有版本列表
    async fn get_versions(
        &self,
        slug: &str,
        mc_version: Option<&str>,
        loader: Option<&str>,
    ) -> Result<Vec<ResolvedMod>, OrbitError>;

    /// 获取平台的分类列表
    async fn get_categories(&self) -> Result<Vec<String>, OrbitError>;
}

/// orbit info 命令的完整输出结构
#[derive(Debug, Clone)]
pub struct ModInfo {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub authors: Vec<String>,
    pub latest_version: String,
    pub downloads: u64,
    pub license: Option<String>,
    pub client_side: Option<SideSupport>,
    pub server_side: Option<SideSupport>,
    pub categories: Vec<String>,
    pub recent_versions: Vec<ModVersionInfo>,
    pub dependencies: Vec<ResolvedDependency>,
}

#[derive(Debug, Clone)]
pub struct ModVersionInfo {
    pub version: String,
    pub mc_versions: Vec<String>,
    pub loader: String,
    pub released_at: String,
}
```

---

## 8. Provider 实现示例

**`orbit-core/src/providers/modrinth.rs`** — 封装 `modrinth-wrapper`：

```rust
// orbit-core/src/providers/modrinth.rs
use async_trait::async_trait;
use modrinth_wrapper::Client;

use super::{ModInfo, ModProvider, ModVersionInfo, ResolvedMod, ResolvedDependency,
            SearchResult, SideSupport};
use crate::error::OrbitError;

pub struct ModrinthProvider {
    client: Client,
}

impl ModrinthProvider {
    pub fn new(user_agent: &str) -> Self {
        Self {
            client: Client::new(user_agent),
        }
    }
}

#[async_trait]
impl ModProvider for ModrinthProvider {
    fn name(&self) -> &'static str {
        "modrinth"
    }

    async fn search(
        &self,
        query: &str,
        mc_version: Option<&str>,
        loader: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SearchResult>, OrbitError> {
        let results = self.client
            .search_projects(query, mc_version, loader, None, 0, limit, None, None)
            .await
            .map_err(|e| OrbitError::Other(e.into()))?;

        Ok(results.hits.into_iter().map(|hit| {
            SearchResult {
                mod_id: hit.project_id,
                slug: hit.slug,
                name: hit.title,
                description: hit.description,
                latest_version: hit.latest_version.unwrap_or_default(),
                downloads: hit.downloads as u64,
                mc_versions: hit.versions,
                client_side: map_side(hit.client_side.as_deref()),
                server_side: map_side(hit.server_side.as_deref()),
                categories: hit.categories,
            }
        }).collect())
    }

    async fn get_mod_info(&self, slug: &str) -> Result<ModInfo, OrbitError> {
        todo!()
    }

    async fn resolve(
        &self,
        slug: &str,
        version_constraint: &str,
        mc_version: &str,
        loader: &str,
    ) -> Result<ResolvedMod, OrbitError> {
        todo!()
    }

    async fn get_version_by_hash(&self, hash: &str) -> Result<Option<ResolvedMod>, OrbitError> {
        todo!()
    }

    async fn get_versions(
        &self,
        slug: &str,
        mc_version: Option<&str>,
        loader: Option<&str>,
    ) -> Result<Vec<ResolvedMod>, OrbitError> {
        todo!()
    }

    async fn get_categories(&self) -> Result<Vec<String>, OrbitError> {
        todo!()
    }
}

fn map_side(side: Option<&str>) -> Option<SideSupport> {
    match side {
        Some("required") => Some(SideSupport::Required),
        Some("optional") => Some(SideSupport::Optional),
        Some("unsupported") => Some(SideSupport::Unsupported),
        _ => None,
    }
}
```

**CurseForgeProvider 结构对称**，不同之处在于构造函数需要 API key（`CurseForgeProvider::new(api_key: &str)`），以及内部的 API 调用映射到 `curseforge-wrapper` 的端点。

---

## 9. 数据流全景

以当前 `orbit add sodium` 为例：

```text
orbit-cli/commands/add.rs
  → 解析命令参数，创建 provider 与确认回调
  → installer::install_to_instance
      → 打开 ManifestFile / Lockfile
      → outdated::download_candidates_with_fallback
          → 按 manifest 配置顺序查询 provider，采用首个有效候选集
          → 并发下载候选 JAR，解析真实 mod_id / version / dependencies
      → resolver::resolve_with_candidates
          → graph::build_solver_graph
          → retry::solve_with_fetch_retry
              → pubgrub::resolve_with_observer
              → 必要时补抓 lockfile 已知的缺失依赖
          → 返回选中的升级版本
      → 通过注入的回调让 CLI 确认计划
      → 下载最终选中 JAR、再次读取元数据并更新 manifest/lockfile
  → orbit-cli 格式化结果
```

这里描述的是现状：`--version` 绑定到候选 JAR 的真实 `mod_id` 并参与根约束；
`--platform` 限定 provider，`--env` 和 `--optional` 写入 manifest 完整依赖形式。
resolver/provider 编排已支持按配置顺序回退，但 CurseForge 的具体实现仍待完成。
高级依赖字段与剩余边界见
[orbit-resolver.md](orbit-resolver.md#8-已收口的规范与剩余边界)。

---

## 10. 历史迁移路线与剩余工作

下面三阶段最初用于从旧 CLI 架构迁移。Phase 1 和 CLI 中的 `adaptors/models` 清理已经完成；
保留清单用于说明历史，不再把它称为“当前状态”。

### Phase 1：创建 `orbit-core` 骨架

1. 创建 `orbit-core/Cargo.toml`，引入 `toml`、`serde` 等依赖
2. 将 `orbit-cli/src/models/` 中的数据结构迁移到 `orbit-core/src/manifest.rs` 和 `orbit-core/src/lockfile.rs`
3. 将 `orbit-cli/src/utils/jar.rs` 迁移到 `orbit-core/src/jar/` （哈希函数 + 元数据提取）
4. 定义 `orbit-core/src/providers/mod.rs` 中的 `ModProvider` trait
5. 在 `orbit-cli/Cargo.toml` 中添加 `orbit-core = { path = "../orbit-core" }` 依赖

### Phase 2：实现 Provider 层

1. 创建 `orbit-core/src/providers/modrinth.rs`，封装 `modrinth-wrapper::Client`
2. 将 `orbit-cli/src/adaptors/curseforge.rs` 的逻辑提取为 `curseforge-wrapper` crate
3. 创建 `orbit-core/src/providers/curseforge.rs`，封装 `curseforge-wrapper`
4. 删除 `orbit-cli/src/adaptors/` 目录

### Phase 3：实现业务逻辑 + 重构 CLI

1. 在 `orbit-core` 中实现 `resolver.rs`、`sync.rs`、`installer.rs`、`checker.rs`、`purge.rs`
2. 重写 `orbit-cli/src/commands/` 下每个命令文件，替换 `println!` 占位符为对 `orbit-core` 的实际调用
3. 删除 `orbit-cli` 中对 `modrinth-wrapper` 和 `curseforge-wrapper` 的直接依赖

---

> **关联文档**
> - `orbit-toml-spec.md` — 项目级 orbit.toml / orbit.lock 格式规格
> - `orbit-global-config.md` — 全局级 config.toml 规格与加载策略
> - `orbit-cli-commands.md` — 命令行为规格
> - `orbit-metadata.md` — 文件格式解析层（metadata/ + jar/）
> - `orbit-detection.md` — 实例环境检测层（init 命令编排）
> - `orbit-providers.md` — 平台 Provider 层（RateLimiter + ModProvider trait）
> - `orbit-versions.md` — 版本号解析（Fabric 等加载器语义）
> - `orbit-resolver.md` — PubGrub 依赖解析引擎设计
> - `orbit-status.md` — 项目完成度追踪
> - 本文档 — 项目结构、模块边界、核心抽象接口
