# Orbit 依赖解析引擎设计

> 本文档定义 `orbit-core/src/resolver/` 的 PubGrub 集成架构、
> 版本号归一化策略、`OrbitDependencyProvider` 实现及编排流程。

---

## 目录

1. [架构原则](#1-架构原则)
2. [模块结构](#2-模块结构)
3. [核心类型映射](#3-核心类型映射)
   - [PackageId / Version / Constraint](#packageid--version--constraint)
   - [MC 版本号归一化](#mc-版本号归一化)
4. [OrbitDependencyProvider](#4-orbitdependencyprovider)
5. [求解器封装](#5-求解器封装)
6. [编排流程（CLI → Resolver → Installer）](#6-编排流程cli--resolver--installer)
7. [冲突报告](#7-冲突报告)
8. [Rust 实现参考](#8-rust-实现参考)

---

## 1. 架构原则

**Resolver 是纯函数**。不发起网络请求、不读写文件。所有数据在进入求解器之前预加载到内存。

```
                    ┌──────────────────────┐
                    │     CLI / 编排层       │
                    │  1. 读取 orbit.toml    │
                    │  2. 调用 Provider API  │  ← 网络 I/O 发生在这里
                    │  3. 填充 Provider 缓存 │
                    │  4. 调用 Resolver      │
                    │  5. 交给 Installer     │  ← 文件 I/O 发生在这里
                    └──────────┬───────────┘
                               │ 调用
                    ┌──────────▼───────────┐
                    │       Resolver        │
                    │  • 纯 CPU 计算        │  ← 零副作用
                    │  • 可单元测试          │
                    │  • PubGrub 算法        │
                    └──────────────────────┘
```

**控制反转**：PubGrub 求解器决定"问什么"，`OrbitDependencyProvider` 回答"有什么"。求解器不关心数据从哪来。

---

## 2. 模块结构

当前 Phase 1 为单文件实现，Phase 2 将拆分为目录结构：

```
orbit-core/src/
├── resolver.rs          # Phase 1：单文件，基于 lockfile 的依赖图查询
│                        # Phase 2 规划：
│   ├── mod.rs           #   compute_resolution_graph() 入口
│   ├── types.rs         #   PackageId, NormalizedVersion, VersionConstraint
│   ├── version.rs       #   版本号归一化
│   └── provider.rs      #   OrbitDependencyProvider impl
```

---

## 2b. Phase 1 实现：lockfile 依赖图查询

当前 resolver 以 `orbit.lock` 为依赖图的唯一数据源，提供以下查询 API：

### `find_entry(slug, entries) -> Option<&LockEntry>`

按 slug 在 lockfile 中查找条目，同时匹配 `entry.name`、`entry.mod_id` 和 `entry.slug`。

### `dependents(slug, entries) -> Vec<&str>`

从 lockfile 的 `[[lock.dependencies]]` 反向查询：返回所有依赖了 `slug` 的模组名称列表。供 `orbit remove` 使用——被依赖的模组不可删除。

### `check_version_conflict(slug, new_version, entries) -> Result<(), String>`

检查新版本与 lockfile 中已有版本是否冲突。若 lock 中已存在同名条目且版本不同，返回错误描述。

### `build_lock_entries(identified, scanned, embedded, loader, mc, loader_ver) -> (Vec<LockEntry>, Vec<String>)`

从已识别的模组列表构建 lock 条目，解析 JAR 声明的依赖、注入环境依赖（minecraft / fabricloader）、检测版本不匹配并生成警告。Entry name 优先使用 `mod_id`（slug）而非 human-readable name。供 `orbit init` 使用。

**注意**：内嵌子模组 dedup 已修复——多父模组共享同名内嵌 JAR 时，按 `(filename, parent)` 精确归入。

从已识别的模组列表构建 lock 条目，解析 JAR 声明的依赖、注入环境依赖（minecraft / fabricloader）、检测版本不匹配并生成警告。供 `orbit init` 使用。

**调用关系**：

```
orbit remove ──→ resolver::dependents()
orbit install ──→ resolver::find_entry()
                  resolver::check_version_conflict()
orbit init    ──→ resolver::build_lock_entries()
```

---

## 3. 核心类型映射

### PackageId / Version / Constraint

```rust
/// 包标识符：`"sodium"`, `"fabric-api"`
pub type PackageId = String;

/// 归一化版本号，基于分词法比较（见 §3 版本号归一化）
pub struct NormalizedVersion {
    /// 原始版本字符串（如 "0.8.7+mc1.21.11"）
    pub raw: String,
    /// 分词后的 Token 序列，用于比较
    tokens: Vec<VersionToken>,
}

/// 版本约束
pub enum VersionConstraint {
    /// 任意版本
    Any,
    /// 精确匹配
    Exact(String),
    /// 范围约束（>=X, <Y）
    Range { lower: NormalizedVersion, upper: NormalizedVersion },
}
```

> **为什么不用 PubGrub 内置的 `SemanticVersion`**：MC 模组版本号不遵循 semver（如 `0.8.7+mc1.21.11`、`1.20.1-0.5.8`、`v12.0.0.1`）。我们需要自己的 `NormalizedVersion`。
>
> PubGrub 的 `Version` trait 是泛型的——我们可以为 `NormalizedVersion` 实现 `pubgrub::version::Version`，完全适配。

### MC 版本号归一化

**为什么不用字典序后缀**：`"0.5.10"` 的字典序小于 `"0.5.8"`（字符 `'1'` < `'8'`），会导致 Orbit 永远把 `0.5.10` 排在 `0.5.8` 前面。这是灾难性的版本倒退。

**策略：分词比较法 (Tokenization)**

将原始版本字符串按分隔符（`.` `-` `+` `_`）切分为 Token 序列，每个 Token 要么是数字（按数值比较）、要么是字母（按字典序，且字母 < 数字）。

```
输入                          分词结果
────────────────────────────────────────────────────────
"0.5.8"                → [Num(0),  Num(5),  Num(8)]
"0.5.10"               → [Num(0),  Num(5),  Num(10)]    ✅ 10 > 8
"0.8.7+mc1.21.11"      → [Num(0),  Num(8),  Num(7),  Alpha("mc"), Num(1), Num(21), Num(11)]
"1.20.1-0.5.8"         → [Num(1),  Num(20), Num(1),  Num(0),  Num(5),  Num(8)]
"v12.0.0.1"            → [Alpha("v12"), Num(0),  Num(0),  Num(1)]
"2024.1.0"             → [Num(2024), Num(1), Num(0)]
"HD_U_G5"              → [Alpha("HD"), Alpha("U"), Alpha("G5")]   ← 全是字母
```

**比较规则**（逐 Token 比较，短路返回）：
1. 如果两个 Token 都是数字 → 按数值大小比较（`10 > 8`）
2. 如果两个 Token 都是字母 → 按字典序比较
3. 如果一个是数字、一个是字母 → **字母 < 数字**（`1.0-alpha` < `1.0`，即预发布版本 < 正式版）
4. 如果一方 Token 耗尽 → 短序列 < 长序列（`"0.5"` < `"0.5.1"`）

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
enum VersionToken {
    Num(u64),
    Alpha(String),
}

impl PartialOrd for VersionToken {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for VersionToken {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (VersionToken::Num(a), VersionToken::Num(b)) => a.cmp(b),   // 数值比较：10 > 8 ✅
            (VersionToken::Alpha(a), VersionToken::Alpha(b)) => a.cmp(b), // 字典序
            (VersionToken::Alpha(_), VersionToken::Num(_)) => Ordering::Less,    // 字母 < 数字
            (VersionToken::Num(_), VersionToken::Alpha(_)) => Ordering::Greater, // 数字 > 字母
        }
    }
}

/// 将版本字符串切分为 Token 序列
fn tokenize(raw: &str) -> Vec<VersionToken> {
    let mut tokens = vec![];
    let mut current = String::new();
    let mut is_numeric = None;

    for ch in raw.chars() {
        let ch_is_digit = ch.is_ascii_digit();
        match is_numeric {
            Some(was_digit) if was_digit == ch_is_digit => current.push(ch),
            _ => {
                if !current.is_empty() {
                    tokens.push(if is_numeric.unwrap_or(false) {
                        VersionToken::Num(current.parse().unwrap_or(0))
                    } else {
                        VersionToken::Alpha(current.clone())
                    });
                }
                current.clear();
                current.push(ch);
                is_numeric = Some(ch_is_digit);
            }
        }
        if ch == '.' || ch == '-' || ch == '+' || ch == '_' {
            // 分隔符被消费为 token 边界，不进入 token
            if !current.is_empty() {
                tokens.push(if is_numeric.unwrap_or(false) {
                    VersionToken::Num(current[..current.len()-1].parse().unwrap_or(0))
                } else {
                    VersionToken::Alpha(current[..current.len()-1].to_string())
                });
            }
            current.clear();
            is_numeric = None;
        }
    }
    if !current.is_empty() {
        tokens.push(if is_numeric.unwrap_or(false) {
            VersionToken::Num(current.parse().unwrap_or(0))
        } else {
            VersionToken::Alpha(current)
        });
    }
    tokens
}

#[derive(Debug, Clone)]
pub struct NormalizedVersion {
    pub raw: String,
    tokens: Vec<VersionToken>,
}

impl Ord for NormalizedVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        for (a, b) in self.tokens.iter().zip(other.tokens.iter()) {
            let ord = a.cmp(b);
            if ord != Ordering::Equal { return ord; }
        }
        self.tokens.len().cmp(&other.tokens.len())
    }
}
```

**验证**：

| 比较 | 结果 | 原因 |
|------|:---:|------|
| `0.5.10` vs `0.5.8` | `>` | Num(10) > Num(8) |
| `0.5.8-hotfix` vs `0.5.8` | `<` | 多了 Alpha 后缀 → 更长 → 更大（补丁版 > 正式版） |
| `1.0-alpha` vs `1.0` | `<` | Alpha < Num（预发布 < 正式） |
| `mc1.21.11` vs `mc1.20.1` | `>` | Num(21) > Num(20) |

---

## 4. OrbitDependencyProvider

**纯内存数据结构**。所有数据在进入求解器之前预加载。

```rust
/// PubGrub 的数据源——一个只读的内存视图
pub struct OrbitDependencyProvider {
    /// package → 已知可用版本列表（从新到旧排序）
    versions: HashMap<PackageId, Vec<NormalizedVersion>>,
    /// (package, version) → 前置依赖列表
    dependencies: HashMap<(PackageId, NormalizedVersion), Vec<(PackageId, VersionConstraint)>>,
}

impl OrbitDependencyProvider {
    /// 向缓存中添加一个包及其版本信息（由编排层在求解前调用）
    pub fn add_package(
        &mut self,
        package_id: PackageId,
        versions: Vec<NormalizedVersion>,       // 来自 Provider API
        dependencies: HashMap<NormalizedVersion, Vec<(PackageId, VersionConstraint)>>,
    ) { ... }
}
```

**`DependencyProvider` trait 实现**：

```rust
impl DependencyProvider<PackageId, NormalizedVersion> for OrbitDependencyProvider {
    fn choose_package_version(
        &self,
        package: &PackageId,
        range: &Range<NormalizedVersion>,
    ) -> Result<Option<NormalizedVersion>, ...> {
        // 从 self.versions 中找满足 range 约束的最新版本
    }

    fn get_dependencies(
        &self,
        package: &PackageId,
        version: &NormalizedVersion,
    ) -> Result<Dependencies<PackageId, NormalizedVersion>, ...> {
        // 从 self.dependencies 中查这个 (package, version) 的前置依赖
    }
}
```

> **关键**：`OrbitDependencyProvider` 内部没有 `reqwest::Client`、没有 `tokio::fs`。它只是一个 `HashMap` 的包装。

---

## 5. 求解器封装

```rust
// resolver/mod.rs

use pubgrub::solver::resolve;

/// 求解依赖图。纯函数，无副作用。
///
/// # 参数
/// - `root_deps`: orbit.toml 中的顶层依赖（包名 → 版本约束）
/// - `provider`: 预填充好的数据源
///
/// # 返回
/// - `Ok(HashMap<PackageId, NormalizedVersion>)` — 每个包被选中的版本
/// - `Err(String)` — 人类可读的冲突报告
pub fn compute_resolution_graph(
    root_deps: HashMap<PackageId, VersionConstraint>,
    provider: &OrbitDependencyProvider,
) -> Result<HashMap<PackageId, NormalizedVersion>, String> {
    let root_pkg = "___orbit_root___".to_string();
    let root_version = NormalizedVersion::zero();

    // 将 root_deps 注册进 provider（或通过单独的 RootPackage 机制）
    let mut solver = pubgrub::solver::Solver::new(provider);

    // 添加根依赖
    for (pkg, constraint) in &root_deps {
        solver.add_dependency(&root_pkg, constraint.into(), pkg.clone());
    }

    match solver.resolve(&root_pkg, root_version) {
        Ok(mut solution) => {
            solution.remove(&root_pkg);
            Ok(solution)
        }
        Err(e) => {
            use pubgrub::report::{DefaultStringReporter, Reporter};
            Err(DefaultStringReporter::report(&e))
        }
    }
}
```

---

## 6. 编排流程（Fetch-and-Retry 懒加载）

### 为什么不能全量预获取

PubGrub 是同步的（`DependencyProvider` trait 方法不能 `await`），但网络请求是异步的。如果在上层把所有版本的依赖都预加载，会造成 N+1 网络海啸——Sodium 有 150 个历史版本，每个版本又有 5 个前置依赖，前置依赖又各有 100 个版本……几分钟内向 Modrinth 发起数千次请求，直接触发 `429 Too Many Requests`。

### 解法：外层 Fetch-and-Retry 状态机

PubGrub 只会在需要时询问两个问题。当它遇到缓存未命中时，返回 `UnknownDependencies` 错误。外层捕获这个错误，**仅获取 PubGrub 确切请求的那个版本的依赖**，然后重试。

```
┌─ resolver/mod.rs ─────────────────────────────────────────────────┐
│                                                                     │
│  pub async fn resolve_manifest(                                     │
│      manifest: &OrbitManifest,                                      │
│      providers: &[Box<dyn ModProvider>],                           │
│  ) -> Result<HashMap<PackageId, NormalizedVersion>, String> {       │
│                                                                     │
│      let mut provider = OrbitDependencyProvider::new();             │
│      let root_pkg = "___root___".to_string();                       │
│                                                                     │
│      // 1. 先注册顶层依赖的版本列表（仅列表，不拉详细依赖）          │
│      for (name, spec) in &manifest.dependencies {                   │
│          let versions = query_versions(name, spec, providers).await?;│
│          provider.add_package_versions(name, versions);             │
│      }                                                              │
│                                                                     │
│      // 2. Fetch-and-Retry 循环                                     │
│      loop {                                                         │
│          match pubgrub::solver::resolve(&provider, &root_pkg, ...) {│
│              Ok(solution) => return Ok(solution),  // ✅ 秒解       │
│              Err(PubGrubError::UnknownDependencies(pkg, ver)) => {  │
│                  // 🔄 仅获取这个确切版本的依赖                      │
│                  let deps = fetch_dependencies(pkg, ver, providers) │
│                      .await?;                                       │
│                  provider.add_package_deps(pkg, ver, deps);         │
│                  // 循环继续，PubGrub 重跑瞬间到断点                 │
│              }                                                       │
│              Err(e) => return Err(report(e)), // ❌ 真正的冲突       │
│          }                                                           │
│      }                                                               │
│  }                                                                   │
│                                                                     │
└────────────────────────────────────────────────────────────────────┘
```

**请求数量对比**：

| 策略 | Sodium (150 版本) + 5 前置 (各 100 版本) | 实际 HTTP 请求 |
|------|------------------------------------------|:---:|
| 全量预获取 | 150 + 5×100 = 650 次 | **650** |
| Fetch-and-Retry | Sodium 最新 1 个版本 + 5 个前置各 1 个版本 | **~6** |

### 完整编排流程（orbit add sodium）

```
┌─ CLI ──────────────────────────────────────────────────────────────┐
│ 1. 读取 orbit.toml                                                  │
│ 2. 调用 resolver::resolve_manifest(&manifest, &providers).await    │
└───────────────────────┬────────────────────────────────────────────┘
                        │
┌─ resolver ───────────▼────────────────────────────────────────────┐
│ 3. 预填充顶层包版本列表（一次 API 调用获得版本号列表）              │
│ 4. Fetch-and-Retry 循环（仅在 PubGrub 需要时拉取依赖详情）          │
│ 5. 返回 selected: HashMap<PackageId, NormalizedVersion>             │
└───────────────────────┬────────────────────────────────────────────┘
                        │
┌─ installer ──────────▼────────────────────────────────────────────┐
│ 6. 并发下载 (JoinSet, 默认 8 并发)                                 │
│ 7. SHA-256 校验                                                    │
│ 8. 写入 mods/                                                      │
└───────────────────────┬────────────────────────────────────────────┘
                        │
┌─ CLI ────────────────▼────────────────────────────────────────────┐
│ 9. 更新 orbit.toml + orbit.lock                                    │
│ 10. 输出: Added sodium 0.5.11 (modrinth)                           │
└────────────────────────────────────────────────────────────────────┘
```

---

## 7. 冲突报告

PubGrub 原生支持详细冲突解释。当依赖不可解时，用户看到的是：

```
Error: Dependency conflict:

  sodium 0.5.8 depends on fabric-api >= 0.92.0
  jei 12.0.0 depends on fabric-api < 0.90.0
  ── fabric-api 0.86.0 was chosen because...

Resolution failed: no version of fabric-api satisfies both
  >=0.92.0 (required by sodium) and <0.90.0 (required by jei)

Suggestion: add an [overrides] entry for fabric-api to force a specific version.
```

> 这比冰冷的 `Panic` 或 `NoSuchElement` 强 100 倍。

---

## 8. Rust 实现参考

### 依赖

```toml
# orbit-core/Cargo.toml
pubgrub = "0.5"
```

### version.rs

```rust
/// 从原始版本字符串归一化（分词法）
pub fn normalize_version(raw: &str) -> NormalizedVersion {
    NormalizedVersion {
        raw: raw.to_string(),
        tokens: tokenize(raw),
    }
}
// tokenize() 实现见 §3 版本号归一化
```

### provider.rs

```rust
/// PubGrub 的内存数据源
pub struct OrbitDependencyProvider {
    /// package → 已知版本列表（从新到旧）
    versions: HashMap<PackageId, Vec<NormalizedVersion>>,
    /// (package, version) → 前置依赖
    dependencies: HashMap<(PackageId, NormalizedVersion), Vec<(PackageId, VersionConstraint)>>,
}

impl OrbitDependencyProvider {
    pub fn add_package_versions(&mut self, pkg: PackageId, versions: Vec<NormalizedVersion>) {
        self.versions.insert(pkg, versions);
    }

    /// 仅在 PubGrub 返回 UnknownDependencies 时调用
    pub fn add_package_deps(
        &mut self,
        pkg: PackageId,
        version: NormalizedVersion,
        deps: Vec<(PackageId, VersionConstraint)>,
    ) {
        self.dependencies.insert((pkg, version), deps);
    }
}

impl DependencyProvider<PackageId, NormalizedVersion> for OrbitDependencyProvider {
    fn choose_package_version(&self, pkg: &PackageId, range: &Range<NormalizedVersion>) -> ... {
        self.versions.get(pkg)
            .and_then(|vs| vs.iter().find(|v| range.contains(v)).cloned())
    }

    fn get_dependencies(&self, pkg: &PackageId, ver: &NormalizedVersion) -> ... {
        match self.dependencies.get(&(pkg.clone(), ver.clone())) {
            Some(deps) => Dependencies::Known(deps.clone()),
            None => Dependencies::Unknown,  // ← 触发外层 fetch-and-retry
        }
    }
}
```

### mod.rs — Fetch-and-Retry 入口

```rust
pub async fn resolve_manifest(
    manifest: &OrbitManifest,
    providers: &[Box<dyn ModProvider>],
) -> Result<HashMap<PackageId, NormalizedVersion>, String> {
    let mut provider = OrbitDependencyProvider::new();
    // 1. 预填充版本列表（仅列表，一次 API 调用）
    // 2. Fetch-and-Retry 循环
    // (见 §6 编排流程)
}
```

### 单元测试设计

Resolver 是纯函数 → 测试无需任何网络：

```rust
#[test]
fn test_simple_resolution() {
    let mut provider = OrbitDependencyProvider::new();
    provider.add_package("sodium".into(), vec![
        normalize_version("0.5.8"),
        normalize_version("0.5.3"),
    ], HashMap::new());

    let root = vec![("sodium".into(), VersionConstraint::Any)];
    let result = compute_resolution_graph(root, &provider).unwrap();
    assert_eq!(result.get("sodium").unwrap().raw, "0.5.8");
}

#[test]
fn test_conflict_detection() {
    // sodium 0.5 要求 fabric-api >= 0.92
    // jei 12.0 要求 fabric-api < 0.90
    // → 不可解
    let mut provider = OrbitDependencyProvider::new();
    // ... 填充数据 ...
    let result = compute_resolution_graph(root, &provider);
    assert!(result.is_err());
}
```

---

> **关联文档**
> - [orbit-architecture.md](orbit-architecture.md) — resolver 在项目中的位置
> - [orbit-metadata.md](orbit-metadata.md) — 元数据解析（为 resolver 提供模组信息）
> - [orbit-toml-spec.md](orbit-toml-spec.md) — orbit.toml 中的版本约束语法
> - [orbit-cli-commands.md](orbit-cli-commands.md) — add/install/upgrade 的 resolver 调用方式
