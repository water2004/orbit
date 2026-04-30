# Orbit 全局配置文件规格

> 本文档定义 `config.toml`（全局用户级配置）的完整 schema 及加载策略。
> 与之对应的项目级配置 `orbit.toml` 见 [orbit-toml-spec.md](orbit-toml-spec.md)。

---

## 目录

1. [概述](#1-概述)
2. [文件位置](#2-文件位置)
3. [完整 Schema](#3-完整-schema)
   - [[core] — 核心设置](#core--核心设置)
   - [[network] — 网络设置](#network--网络设置)
   - [[auth] — 平台认证](#auth--平台认证)
   - [[cache] — 缓存设置](#cache--缓存设置)
   - [[ui] — 终端界面](#ui--终端界面)
4. [配置加载优先级](#4-配置加载优先级)
5. [Rust 实现参考](#5-rust-实现参考)
6. [安全注意事项](#6-安全注意事项)

---

## 1. 概述

Orbit 有两级配置文件：

| 文件 | 级别 | 内容 |
|------|------|------|
| `orbit.toml` | 项目级 | 该 Minecraft 实例装了什么模组 |
| `config.toml` | 全局级 | Orbit 这个工具本身该如何运行 |

`config.toml` **不关心你装了什么模组**，只控制 Orbit CLI 的运行时行为——代理、缓存、并发数、认证等。

---

## 2. 文件位置

```
平台    路径
─────────────────────────────────────────────
Windows  %APPDATA%\orbit\config.toml
Linux    ~/.orbit/config.toml
macOS    ~/Library/Application Support/orbit/config.toml (预留)
```

> **设计决策**：采用统一的 `orbit/` 数据目录，`instances.toml` 与 `config.toml` 同目录存放。
> 不拆分 XDG `~/.config` vs `~/.local/share`——Orbit 的配置和状态数据都是轻量 TOML 文件，没必要分两个目录。

---

## 3. 完整 Schema

```toml
# ==========================================
# Orbit 全局用户配置文件
# ==========================================

# ── 核心设置 ──────────────────────────────
[core]
# 默认全局实例名称（当你在非项目目录下执行命令时默认操作它）
# default_instance = "my-survival"
# 并发下载的最大线程数（默认 8）
max_concurrent_downloads = 8
# 语言偏好: "en" | "zh-CN"（默认 "en"）
language = "zh-CN"

# ── 网络设置 ──────────────────────────────
[network]
# HTTP 请求超时时间（秒）
timeout = 30
# 遇到网络错误时的最大重试次数
max_retries = 3
# 可选的 HTTP 代理（对国内玩家拉取 CurseForge 至关重要）
# proxy = "http://127.0.0.1:7890"

# ── 平台认证 ──────────────────────────────
[auth]
# CurseForge API Key（使用第三方客户端时必须提供）
# curseforge_token = "cf_YOUR_API_KEY_HERE"
# Modrinth Token（可选，用于操作私有项目）
# modrinth_token = "mrp_YOUR_TOKEN_HERE"

# ── 缓存设置 ──────────────────────────────
[cache]
# 是否开启全局 JAR 包缓存（多实例共享模组秒装）
enable = true
# 自定义缓存目录（可选）。留空则使用默认路径。
# Windows 示例: dir = "D:/Games/OrbitCache"
# Linux 示例:   dir = "/mnt/data/orbit-cache"
# 缓存清理策略: "size" | "time" | "none"
eviction_policy = "size"
# 最大缓存占用（GB），eviction_policy = "size" 时生效
max_size_gb = 5.0

# ── 终端界面 ──────────────────────────────
[ui]
# 终端颜色: "auto" | "always" | "never"
color = "auto"
# 进度条样式: "modern" | "classic" | "none"
progress_bar = "modern"
```

### [core] — 核心设置

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `default_instance` | `Option<String>` | `None` | 全局默认实例名。等价于 `orbit instances default <name>` |
| `max_concurrent_downloads` | `usize` | `8` | 并发下载上限。网络差时可调低 |
| `language` | `String` | `"en"` | 终端输出语言 + 平台 API 搜索结果偏好 |

### [network] — 网络设置

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `timeout` | `u64` | `30` | HTTP 请求超时（秒） |
| `max_retries` | `u32` | `3` | 网络错误自动重试次数 |
| `proxy` | `Option<String>` | `None` | HTTP 代理 URL。设置后所有 Orbit 发起的请求都走代理 |

### [auth] — 平台认证

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `curseforge_token` | `Option<String>` | `None` | CurseForge API Key |
| `modrinth_token` | `Option<String>` | `None` | Modrinth API Token |

> **安全警告**：API Key 以明文存储在 `config.toml` 中。请勿将此文件纳入 Git 或分享给他人。
> 未来版本计划使用 OS 凭据管理器（Windows Credential Manager / Linux Secret Service）存储敏感信息。

### [cache] — 缓存设置

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `enable` | `bool` | `true` | 是否开启全局 JAR 缓存 |
| `dir` | `Option<String>` | `None` | 自定义缓存目录。留空则使用 `{数据目录}/cache`。适合系统盘空间紧张时指向其他硬盘 |
| `eviction_policy` | `String` | `"size"` | 清理策略：`"size"` 按大小，`"time"` 按时间过期，`"none"` 不自动清理 |
| `max_size_gb` | `f64` | `5.0` | 最大缓存占用（GB），`eviction_policy = "size"` 时生效 |

> **全局缓存的收益**：玩家可能有 3 个 1.20.1 整合包都装了 Sodium。开启缓存后 Orbit 只下载一次，在各实例间使用硬链接（或 copy）部署，节省时间和磁盘。

### [ui] — 终端界面

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `color` | `String` | `"auto"` | 终端颜色输出：`"auto"`（自动探测 TTY）、`"always"`、`"never"` |
| `progress_bar` | `String` | `"modern"` | 进度条样式：`"modern"`（Cargo 风格）、`"classic"`（ASCII）、`"none"` |

---

## 4. 配置加载优先级

Orbit 按以下优先级合并配置（高优先级覆盖低优先级）：

```
1. 命令行参数          --proxy http://127.0.0.1:1080
2. 环境变量            ORBIT_PROXY=http://127.0.0.1:1080
3. config.toml         ~/.orbit/config.toml 中的 [network] proxy
4. 代码默认值           NetworkConfig::default()
```

**环境变量映射**：

| 环境变量 | 覆盖字段 |
|----------|---------|
| `ORBIT_PROXY` | `network.proxy` |
| `ORBIT_TIMEOUT` | `network.timeout` |
| `ORBIT_RETRIES` | `network.max_retries` |
| `ORBIT_LANGUAGE` | `core.language` |
| `ORBIT_CURSEFORGE_TOKEN` | `auth.curseforge_token` |
| `ORBIT_MODRINTH_TOKEN` | `auth.modrinth_token` |

> **设计原则**：不为每个字段都做 CLI 参数——只对 `--proxy` 这种临时需要覆盖的字段提供命令行开关。
> 大部分配置建议用户写在 `config.toml` 里，一次设置永久生效。

---

## 5. Rust 实现参考

```rust
// orbit-core/src/config.rs

use serde::{Deserialize, Serialize};

/// config.toml 的完整 Rust 表示
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrbitGlobalConfig {
    #[serde(default)]
    pub core: CoreConfig,
    #[serde(default)]
    pub network: NetworkConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub cache: CacheConfig,
    #[serde(default)]
    pub ui: UiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreConfig {
    pub default_instance: Option<String>,
    #[serde(default = "default_max_downloads")]
    pub max_concurrent_downloads: usize,
    #[serde(default = "default_language")]
    pub language: String,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            default_instance: None,
            max_concurrent_downloads: 8,
            language: "en".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    #[serde(default = "default_timeout")]
    pub timeout: u64,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    pub proxy: Option<String>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self { timeout: 30, max_retries: 3, proxy: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthConfig {
    pub curseforge_token: Option<String>,
    pub modrinth_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    #[serde(default = "default_true")]
    pub enable: bool,
    pub dir: Option<String>,
    #[serde(default = "default_eviction_policy")]
    pub eviction_policy: String,
    #[serde(default = "default_max_size")]
    pub max_size_gb: f64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self { enable: true, dir: None, eviction_policy: "size".into(), max_size_gb: 5.0 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default = "default_progress_bar")]
    pub progress_bar: String,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self { color: "auto".into(), progress_bar: "modern".into() }
    }
}

// 辅助默认值函数
fn default_max_downloads() -> usize { 8 }
fn default_language() -> String { "en".into() }
fn default_timeout() -> u64 { 30 }
fn default_max_retries() -> u32 { 3 }
fn default_true() -> bool { true }
fn default_eviction_policy() -> String { "size".into() }
fn default_max_size() -> f64 { 5.0 }
fn default_color() -> String { "auto".into() }
fn default_progress_bar() -> String { "modern".into() }
```

**加载流程**：

```rust
impl OrbitGlobalConfig {
    /// 分层加载：config.toml → 环境变量覆盖 → 返回
    pub fn load() -> Result<Self, OrbitError> {
        let path = orbit_data_dir().join("config.toml");

        // Layer 1: 文件（如果存在）
        let mut config = if path.exists() {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| OrbitError::Other(anyhow::anyhow!("failed to read config.toml: {e}")))?;
            toml::from_str(&content)
                .map_err(|e| OrbitError::Other(anyhow::anyhow!("failed to parse config.toml: {e}")))?
        } else {
            Self::default()
        };

        // Layer 2: 环境变量覆盖
        if let Ok(proxy) = std::env::var("ORBIT_PROXY") {
            config.network.proxy = Some(proxy);
        }
        if let Ok(lang) = std::env::var("ORBIT_LANGUAGE") {
            config.core.language = lang;
        }
        if let Ok(cf) = std::env::var("ORBIT_CURSEFORGE_TOKEN") {
            config.auth.curseforge_token = Some(cf);
        }
        if let Ok(mr) = std::env::var("ORBIT_MODRINTH_TOKEN") {
            config.auth.modrinth_token = Some(mr);
        }

        Ok(config)
    }
}

impl Default for OrbitGlobalConfig {
    fn default() -> Self {
        Self {
            core: CoreConfig::default(),
            network: NetworkConfig::default(),
            auth: AuthConfig::default(),
            cache: CacheConfig::default(),
            ui: UiConfig::default(),
        }
    }
}
```

---

## 6. 安全注意事项

1. **`config.toml` 应设为 `0600` 权限**（仅所有者可读写）——其中 `[auth]` 块包含 API Token。
2. **不要将 `config.toml` 纳入 Git 版本控制**——Orbit 会将其放在 `orbit/` 数据目录下，而非项目目录。
3. **环境变量 `ORBIT_CURSEFORGE_TOKEN`** 提供的值优先级高于 `config.toml`，适合 CI/CD 或临时使用。
4. 未来计划使用 `keyring` crate 将 Token 存入操作系统凭据管理器，届时 `config.toml` 中的 `[auth]` 将只作为回退方案。

---

> **关联文档**
> - [orbit-toml-spec.md](orbit-toml-spec.md) — 项目级 `orbit.toml` 规格
> - [orbit-architecture.md](orbit-architecture.md) — 项目结构中的配置模块
> - [orbit-cli-commands.md](orbit-cli-commands.md) — 命令行为与全局标志
