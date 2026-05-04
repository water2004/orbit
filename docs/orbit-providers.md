# Orbit 平台 Provider 层设计

> 本文档定义 `orbit-core/src/providers/` 中 `RateLimiter` 任务队列和 `ModProvider` trait 的完整规格。

---

## 目录

1. [设计原则](#1-设计原则)
2. [模块结构](#2-模块结构)
3. [RateLimiter — 并发控制](#3-ratelimiter--并发控制)
4. [ModProvider trait 集成](#4-modprovider-trait-集成)
5. [统一数据类型](#5-统一数据类型)
6. [Provider 工厂函数](#6-provider-工厂函数)
7. [调用方视角](#7-调用方视角)
8. [Rust 实现参考](#8-rust-实现参考)

---

## 1. 设计原则

| 原则 | 说明 |
|------|------|
| **并发归 Provider** | 速率限制是平台 API 的实现细节，完全封装在 `ModProvider` impl 内部 |
| **对外透明** | 调用方只需 `spawn` 并发任务，队列和排队由 Provider 内部 Semaphore 自动处理 |
| **独立控制** | 每个平台持有自己的 `RateLimiter`，Modrinth 限 3 和 CurseForge 限 3 互不干扰 |

```
调用方: spawn 50 个任务并发调用 provider.get_version_by_hash()
    │
    ▼
Provider 内部:
    RateLimiter(Semaphore(3))
    ┌────┐ ┌────┐ ┌────┐
    │ T1 │ │ T2 │ │ T3 │  ← 同时最多 3 个
    └────┘ └────┘ └────┘
    ┌────┐ ┌────┐ ...    ← T4-T50 在 Semaphore 上排队阻塞
    │ T4 │ │ T5 │
    └────┘ └────┘
```

---

## 2. 模块结构

```
orbit-core/src/providers/
├── mod.rs           # ModProvider trait + create_providers() + 统一数据类型
├── rate_limiter.rs  # RateLimiter — Semaphore 封装
├── modrinth.rs      # ModrinthProvider（持有 RateLimiter）
└── curseforge.rs    # CurseForgeProvider（持有 RateLimiter）
```

---

## 3. RateLimiter — 并发控制

### 核心实现

```rust
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// 平台 API 并发控制工具。
///
/// 基于 tokio Semaphore 实现：
/// - `acquire()` 获取一个 permit，所有槽位被占时自动阻塞等待
/// - permit drop 时自动释放槽位
/// - 内部持有一个 permit 意味着"正在发起一个 HTTP 请求"
pub struct RateLimiter {
    semaphore: Arc<Semaphore>,
}

impl RateLimiter {
    /// `max_concurrency` — 最大并发请求数
    ///
    /// 建议值：
    ///   Modrinth:  3-4  (API 无官方速率限制文档，实测 4 并发安全)
    ///   CurseForge: 2-3 (需要 API Key，限制较严)
    pub fn new(max_concurrency: usize) -> Self {
        Self { semaphore: Arc::new(Semaphore::new(max_concurrency)) }
    }

    /// 获取一个并发槽位。所有槽位被占时自动 await 等待。
    /// Semaphore 关闭时返回错误（正常运行时不会发生）。
    pub async fn acquire(&self) -> Result<OwnedSemaphorePermit, OrbitError> {
        self.semaphore.clone().acquire_owned().await
            .map_err(|_| OrbitError::Other(anyhow!("RateLimiter semaphore unexpectedly closed")))
    }
}
```

### 为什么不用 crate

- `governor` / `ratelimit` 等 crate 侧重时间窗口限流（如 60 次/分钟），实现的是"令牌桶"
- Orbit 的需求是**控制并发连接数**，不是时间窗口阈值
- `tokio::sync::Semaphore` 是标准库级别的并发原语，零额外依赖，恰好满足需求

---

## 4. ModProvider trait 集成

### 当前 trait

```rust
#[async_trait]
pub trait ModProvider: Send + Sync {
    fn name(&self) -> &'static str;
    async fn search(...) -> Result<Vec<SearchResultItem>, OrbitError>;
    async fn get_mod_info(&self, slug: &str) -> Result<ModInfo, OrbitError>;
    async fn resolve(...) -> Result<ResolvedMod, OrbitError>;
    async fn get_version_by_hash(&self, hash: &str) -> Result<Option<ResolvedMod>, OrbitError>;
    async fn get_versions_by_hashes(&self, hashes: &[String]) -> Result<Vec<ResolvedMod>, OrbitError>;
    async fn get_versions(...) -> Result<Vec<ResolvedMod>, OrbitError>;
    async fn get_categories(&self) -> Result<Vec<String>, OrbitError>;
    async fn fetch_dependencies(&self, project_id: &str) -> Result<Vec<ResolvedDependency>, OrbitError>;
}
```

### Provider 实现模式（以 Modrinth 为例）

```rust
pub struct ModrinthProvider {
    client: Client,
    rate_limiter: RateLimiter,
}

impl ModrinthProvider {
    pub fn new(user_agent: &str, max_concurrency: usize) -> Result<Self, OrbitError> {
        Ok(Self {
            client: Client::new(user_agent).map_err(...)?,
            rate_limiter: RateLimiter::new(max_concurrency),
        })
    }
}

#[async_trait]
impl ModProvider for ModrinthProvider {
    fn name(&self) -> &'static str { "modrinth" }

    async fn get_version_by_hash(&self, hash: &str) -> Result<Option<ResolvedMod>, OrbitError> {
        let _permit = self.rate_limiter.acquire().await?;  // ← 排队
        match self.client.get_version_from_hash(hash, Some("sha512"), None).await {
            Ok(v) => Ok(Some(...)),
            Err(_) => Ok(None),
        }
    }

    async fn get_versions(&self, slug: &str, mc: Option<&str>, loader: Option<&str>) -> ... {
        let _permit = self.rate_limiter.acquire().await?;  // ← 排队
        // ...
    }

    // 所有 async fn 都在第一行 acquire，Result 用 ? 传播
}
```

**CurseForgeProvider 同理**，仅在构造时指定不同的 `max_concurrency`。

---

## 5. 统一数据类型

### ResolvedMod

平台解析后的统一模组信息。

```rust
#[derive(Debug, Clone)]
pub struct ResolvedMod {
    /// fabric.mod.json 的 `id`（即 mod_id，PubGrub 用此作为 PackageId）
    pub mod_id: String,
    /// fabric.mod.json 的 `version`
    pub version: String,
    /// SHA-1 哈希
    pub sha1: String,
    /// SHA-512 哈希（Modrinth 原生提供，用于下载校验）
    pub sha512: String,
    /// Modrinth 专属: project_id
    pub project_id: String,
    /// Modrinth 专属: version_id
    pub version_id: String,
    /// Modrinth 专属: version_number（如 "mc26.1.2-0.8.10-fabric"）
    pub modrinth_version: String,
    /// Modrinth 专属: slug
    pub slug: String,
    /// 发布时间（ISO 8601），provider 版本排序用
    pub date_published: String,
    /// 下载 URL
    pub download_url: String,
    /// jar 文件名
    pub filename: String,
    /// 前置依赖
    pub dependencies: Vec<ResolvedDependency>,
    /// 平台元数据声明的 client_side
    pub client_side: Option<SideSupport>,
    /// 平台元数据声明的 server_side
    pub server_side: Option<SideSupport>,
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `mod_id` | `String` | fabric.mod.json 的 `id`，PubGrub 的 PackageId |
| `version` | `String` | fabric.mod.json 的 `version`（语义版本号） |
| `sha1` | `String` | SHA-1 哈希值 |
| `sha512` | `String` | SHA-512 哈希值（Modrinth 原生，下载校验） |
| `project_id` | `String` | Modrinth project_id |
| `version_id` | `String` | Modrinth version_id |
| `modrinth_version` | `String` | Modrinth version_number（完整版本字符串） |
| `slug` | `String` | Modrinth slug |
| `date_published` | `String` | ISO 8601 发布时间，provider resolver 排序依据 |
| `download_url` | `String` | 可直接下载的 URL |
| `filename` | `String` | 下载文件名 |
| `dependencies` | `Vec<ResolvedDependency>` | 前置依赖列表 |
| `client_side` | `Option<SideSupport>` | Required / Optional / Unsupported |
| `server_side` | `Option<SideSupport>` | Required / Optional / Unsupported |

### ResolvedDependency

```rust
#[derive(Debug, Clone)]
pub struct ResolvedDependency {
    /// 依赖的 slug（Modrinth 解析后），或 mod_id
    pub slug: Option<String>,
    pub required: bool,
}
```

仅包含 `slug` 和 `required` 两个字段。`slug` 为 `Option<String>`——当 API 返回 `project_id` 时通过 `lookup_project_slugs()` 批量解析填充；无法解析时为 `None`，此时调用方回退到 `mod_id`。

### SideSupport

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum SideSupport {
    Required,
    Optional,
    Unsupported,
}
```

---

## 6. Provider 工厂函数

定义在 `providers/mod.rs` 中。

```rust
/// 根据配置创建 provider 列表，按 `resolver.platforms` 顺序。
pub fn create_providers(platforms: &[String]) -> Result<Vec<Box<dyn ModProvider>>, OrbitError> {
    let ua = format!("orbit/{}", env!("CARGO_PKG_VERSION"));
    // 按 platforms 顺序构造，modrinth → ModrinthProvider::new(&ua, 3)
    // curseforge → 暂未实现，输出 warning 并跳过
    // 未知平台 → 输出 warning 并跳过
    // 若结果为空则返回 Err
}

/// 默认仅 Modrinth 的 provider 列表。
pub fn create_providers_default() -> Result<Vec<Box<dyn ModProvider>>, OrbitError> {
    create_providers(&["modrinth".into()])
}
```

- `create_providers()` 接收平台名称列表，返回 `Vec<Box<dyn ModProvider>>`
- 通过 trait object 擦除具体类型，`resolver` 只依赖 `ModProvider` trait
- `create_providers_default()` 提供仅 Modrinth 的便捷构造

---

## 7. 调用方视角

```rust
// 调用方（如 identification.rs）使用工厂创建 provider：
let providers = create_providers(&["modrinth".into()])?;

// 批量 API 避免 N+1：
let found = providers[0].get_versions_by_hashes(&hashes).await?;
```

**推荐**：`identification.rs` 使用 `get_versions_by_hashes()` 批量端点，将 30 个 mod 的识别从 60+ 次请求压缩到 1 次。

**多平台并行**：ModrinthProvider(Semaphore(3)) + CurseForgeProvider(Semaphore(2)) = 两个独立 Semaphore，共 5 个并发请求同时进行，互不阻塞。

---

## 8. Rust 实现参考

### 单元测试

```rust
#[tokio::test]
async fn rate_limiter_serializes_requests() {
    let limiter = RateLimiter::new(1);
    let counter = Arc::new(AtomicU32::new(0));
    let mut handles = vec![];
    for _ in 0..10 {
        let limiter = &limiter;
        let counter = &counter;
        handles.push(tokio::spawn(async move {
            let _permit = limiter.acquire().await?;
            let prev = counter.fetch_add(1, Ordering::SeqCst);
            assert_eq!(prev, 0);
            tokio::time::sleep(Duration::from_millis(1)).await;
            counter.fetch_sub(1, Ordering::SeqCst);
        }));
    }
    for h in handles { h.await.unwrap(); }
}
```

---

> **关联文档**
> - [orbit-architecture.md](orbit-architecture.md) — providers/ 在项目中的位置
> - [orbit-toml-spec.md](orbit-toml-spec.md) — DependencySpec 的 platform 格式
