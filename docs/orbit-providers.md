# Orbit 平台 Provider 层设计

> 本文档定义 `orbit-core/src/providers/` 中 `RateLimiter` 任务队列和 `ModProvider` trait 的完整规格。

---

## 目录

1. [设计原则](#1-设计原则)
2. [模块结构](#2-模块结构)
3. [RateLimiter — 并发控制](#3-ratelimiter--并发控制)
4. [ModProvider trait 集成](#4-modprovider-trait-集成)
5. [调用方视角](#5-调用方视角)
6. [Rust 实现参考](#6-rust-实现参考)

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
├── mod.rs           # ModProvider trait
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
    pub async fn acquire(&self) -> OwnedSemaphorePermit {
        self.semaphore.clone().acquire_owned().await
            .expect("RateLimiter semaphore closed")
    }
}
```

### 为什么不用 crate

- `governor` / `ratelimit` 等 crate 侧重时间窗口限流（如 60 次/分钟），实现的是"令牌桶"
- Orbit 的需求是**控制并发连接数**，不是时间窗口阈值
- `tokio::sync::Semaphore` 是标准库级别的并发原语，零额外依赖，恰好满足需求

---

## 4. ModProvider trait 集成

### 当前 trait（不变）

```rust
#[async_trait]
pub trait ModProvider: Send + Sync {
    fn name(&self) -> &'static str;
    async fn search(...) -> Result<Vec<SearchResultItem>, OrbitError>;
    async fn get_mod_info(&self, slug: &str) -> Result<ModInfo, OrbitError>;
    async fn resolve(...) -> Result<ResolvedMod, OrbitError>;
    async fn get_version_by_hash(&self, hash: &str) -> Result<Option<ResolvedMod>, OrbitError>;
    async fn get_versions(...) -> Result<Vec<ResolvedMod>, OrbitError>;
    async fn get_categories(&self) -> Result<Vec<String>, OrbitError>;
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
        let _permit = self.rate_limiter.acquire().await;  // ← 排队
        match self.client.get_version_from_hash(hash).await {
            Ok(v) => Ok(Some(...)),
            Err(_) => Ok(None),
        }
    }

    async fn get_versions(&self, slug: &str, mc: Option<&str>, loader: Option<&str>) -> ... {
        let _permit = self.rate_limiter.acquire().await;  // ← 排队
        // ...
    }

    // 所有 async fn 都在第一行 acquire
}
```

**CurseForgeProvider 同理**，仅在构造时指定不同的 `max_concurrency`。

---

## 5. 调用方视角

```rust
// 调用方（如 identification.rs）完全看不到 RateLimiter：
let providers: Vec<Box<dyn ModProvider>> = vec![
    Box::new(ModrinthProvider::new("orbit", 3)?),
    Box::new(CurseForgeProvider::new("key", 2)),   // future
];

// 并发发起 50 个识别请求 —— Provider 内部自动排队
let handles: Vec<_> = scanned_mods.iter().map(|m| {
    let providers = &providers;
    tokio::spawn(async move {
        for p in providers {
            if let Ok(Some(v)) = p.get_version_by_hash(&m.sha256).await {
                return Some((p.name(), v));
            }
        }
        None
    })
}).collect();
```

**多平台并行**：ModrinthProvider(Semaphore(3)) + CurseForgeProvider(Semaphore(2)) = 两个独立 Semaphore，共 5 个并发请求同时进行，互不阻塞。

---

## 6. 下载原子性与进度回调

### 6.1 原子写入

```rust
impl ModrinthProvider {
    /// 下载模组到指定目录。
    /// 先写入 .tmp 文件 → SHA-256 校验 → 校验通过后原子 rename 为正式文件名。
    async fn download_to(
        &self,
        url: &str,
        expected_sha256: &str,
        dest_dir: &Path,
        filename: &str,
    ) -> Result<PathBuf, OrbitError> {
        let _permit = self.rate_limiter.acquire().await;

        let tmp_path = dest_dir.join(format!(".{filename}.tmp"));
        let final_path = dest_dir.join(filename);

        // 1. 下载到临时文件
        let bytes = reqwest::get(url).await?.bytes().await?;
        tokio::fs::write(&tmp_path, &bytes).await?;

        // 2. SHA-256 校验
        let actual = sha256_digest(&bytes);
        if actual != expected_sha256 {
            tokio::fs::remove_file(&tmp_path).await.ok();
            return Err(OrbitError::ChecksumMismatch { ... });
        }

        // 3. 原子重命名
        tokio::fs::rename(&tmp_path, &final_path).await?;
        Ok(final_path)
    }
}
```

### 6.2 进度回调解耦

`orbit-core` 不依赖 `indicatif` 等终端 UI 库。通过回调将进度通知交给调用方：

```rust
/// 下载进度回调：`(bytes_downloaded, total_bytes)`
pub type ProgressCallback = Box<dyn Fn(u64, u64) + Send + Sync>;

impl ModrinthProvider {
    async fn download_to(
        &self,
        url: &str,
        expected_sha256: &str,
        dest_dir: &Path,
        filename: &str,
        on_progress: Option<&ProgressCallback>,
    ) -> Result<PathBuf, OrbitError> {
        let _permit = self.rate_limiter.acquire().await;
        // ... 流式下载，每 8KB 调用 on_progress(bytes_so_far, total)
    }
}
```

CLI 层注入自己的回调：

```rust
// orbit-cli
let pb = ProgressBar::new(total_size);
provider.download_to(url, sha256, dir, name, Some(&Box::new(move |done, total| {
    pb.set_length(total);
    pb.set_position(done);
}))).await?;
```

### 6.3 为什么不用 trait 对象？

回调用 `Box<dyn Fn>` 而非泛型 `<F: Fn(u64, u64)>`，原因：
- 泛型会污染整个调用链签名（`ModProvider` trait 的 async fn 当前不支持泛型参数）
- `Box<dyn Fn>` 允许调用方传 `None` 跳过进度报告
- 性能开销可忽略（下载 I/O 远大于一次动态分发）

---

## 7. Rust 实现参考

### Cargo.toml

无需新增依赖。`tokio::sync::Semaphore` 已在 workspace 依赖中。

### 单元测试

```rust
#[tokio::test]
async fn rate_limiter_serializes_requests() {
    let limiter = RateLimiter::new(1); // 单槽位 = 完全串行
    let counter = Arc::new(AtomicU32::new(0));

    let mut handles = vec![];
    for _ in 0..10 {
        let limiter = &limiter;
        let counter = &counter;
        handles.push(tokio::spawn(async move {
            let _permit = limiter.acquire().await;
            let prev = counter.fetch_add(1, Ordering::SeqCst);
            assert_eq!(prev, 0); // 同时最多 1 个在执行
            tokio::time::sleep(Duration::from_millis(1)).await;
            counter.fetch_sub(1, Ordering::SeqCst);
        }));
    }
    for h in handles { h.await.unwrap(); }
}

#[tokio::test]
async fn download_atomic_on_checksum_fail() {
    // SHA-256 不匹配时 .tmp 文件应被删除，最终路径不应存在
}
```

---

> **关联文档**
> - [orbit-architecture.md](orbit-architecture.md) — providers/ 在项目中的位置
> - [orbit-toml-spec.md](orbit-toml-spec.md) — DependencySpec 的 platform 格式
