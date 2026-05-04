# Orbit 依赖解析引擎设计

> 本文档定义 `orbit-core/src/resolver/` 的 PubGrub 集成架构及 lockfile 查询 API。

---

## 目录

1. [模块结构](#1-模块结构)
2. [PubGrub 求解器集成](#2-pubgrub-求解器集成)
3. [Fetch-and-Retry 懒加载](#3-fetch-and-retry-懒加载)
4. [ProviderVersionResolver 版本比较](#4-providerversionresolver-版本比较)
5. [版本比较规则](#5-版本比较规则)
6. [本地检查（check_local_graph）](#6-本地检查check_local_graph)
7. [Lockfile 查询 API](#7-lockfile-查询-api)
8. [版本号系统](#8-版本号系统)

---

## 1. 模块结构

```
orbit-core/src/resolver/
├── mod.rs                 # resolve_manifest(), check_local_graph(), 查询 API
├── types.rs               # PackageId 类型别名
├── provider.rs            # OrbitDependencyProvider impl DependencyProvider
├── provider_version.rs    # ProviderVersionResolver trait + FallbackResolver
├── modrinth_version.rs    # ModrinthVersionResolver — date_published 排序
│                          # inject_lockfile() + dependents() / find_entry()
```

与 `versions/` 模块紧密协作——`Version` 枚举和 `parse_constraint()` 在 `versions/mod.rs` 中定义。

---

## 2. PubGrub 求解器集成

`resolve_manifest(manifest, lockfile, providers)` 对 `orbit.toml` 中声明的顶层依赖执行 PubGrub 求解：

- 输入：`OrbitManifest`（顶层依赖 + MC version + loader）+ `OrbitLockfile` + Provider 列表
- 通过 `inject_lockfile()` 将 lockfile 已有条目注入 PubGrub（条目不携带依赖，避免重解析已安装 mod 的内部链）
- 输出：`HashMap<PackageId, ResolvedMod>` —— 每个包被选中的版本（含下载 URL、SHA-512 等）
- 版本约束通过 `Version::parse_constraint()` 转换为 PubGrub `Range<Version>`
- 冲突时返回人类可读的冲突报告（PubGrub `DefaultStringReporter`）

---

## 3. Fetch-and-Retry 懒加载

PubGrub 是同步的，但 API 调用是异步的。`resolve_manifest()` 使用 Fetch-and-Retry 模式：

1. 先注册顶层依赖的版本列表（一次 `get_versions()` API 调用）
2. 调用 `pubgrub::solver::resolve()`
3. 若 PubGrub 返回 `FetchRetryError`（缓存未命中）→ 仅获取该包的版本列表和依赖 → 填充缓存 → 重试
4. 循环直到求解成功或冲突

这避免了全量预获取导致的请求海啸：Sodium 150 个版本 x 5 个前置 x 100 版本 = 650 次 → Fetch-and-Retry 只需 ~6 次。

---

## 4. ProviderVersionResolver 版本比较

Provider 版本解析器仅在 `resolve_manifest()` 的 Fetch-and-Retry 循环中使用，处理 API 返回的依赖版本排序和约束检查。与 `versions/` 的字符串比较不同，Provider resolver 使用 provider 特定逻辑。

### ProviderVersionResolver trait

```rust
/// Provider 版本比较与约束检查。
pub trait ProviderVersionResolver: Send + Sync {
    fn provider_name(&self) -> &str;

    /// 从最新到最旧排序（会修改传入的 slice）
    fn sort_newest_first(&self, versions: &mut [ResolvedMod]);

    /// 检查版本是否满足约束（provider 特定逻辑）
    fn satisfies(&self, version: &ResolvedMod, constraint: &str) -> bool;

    /// 从列表中选出满足约束的最新版本（默认实现：filter + sort + first）
    fn pick_best(&self, versions: &[ResolvedMod], constraint: &str) -> Option<ResolvedMod>;
}
```

### ModrinthVersionResolver

位于 `modrinth_version.rs`，使用 `date_published` 时间戳排序。

- **`sort_newest_first()`**: 按 `date_published` 降序排列。ISO 8601 格式天然可字符串排序，无需额外解析。
- **`satisfies()`**: 优先用 `modrinth.version_number` 做 SemVer 约束检查，若不存在或为空则回退到 `version` 字段。约束为 `*` 或空时始终返回 `true`。

```rust
pub struct ModrinthVersionResolver;

impl ProviderVersionResolver for ModrinthVersionResolver {
    fn provider_name(&self) -> &str { "modrinth" }

    fn sort_newest_first(&self, versions: &mut [ResolvedMod]) {
        versions.sort_by(|a, b| b.date_published.cmp(&a.date_published));
    }

    fn satisfies(&self, version: &ResolvedMod, constraint: &str) -> bool {
        if constraint == "*" || constraint.is_empty() { return true; }
        let ver_str = match &version.modrinth {
            Some(m) if !m.version_number.is_empty() => &m.version_number,
            _ => &version.version,
        };
        // fabric::SemanticVersion::parse + fabric::satisfies
        // 回退到字符串完全匹配
    }
}
```

### FallbackResolver

位于 `provider_version.rs`，默认回退实现——使用 `fabric::SemanticVersion` 字符串比较。

- **`sort_newest_first()`**: 解析 SemanticVersion 后降序排列。解析失败则回退到字符串比较（`Ord`）。
- **`satisfies()`**: 使用 `fabric::satisfies()` 做 SemVer 约束检查。解析失败则回退到字符串完全匹配。

```rust
pub struct FallbackResolver;

impl ProviderVersionResolver for FallbackResolver {
    fn provider_name(&self) -> &str { "fallback" }

    fn sort_newest_first(&self, versions: &mut [ResolvedMod]) {
        versions.sort_by(|a, b| {
            let va = fabric::SemanticVersion::parse(&a.version, true);
            let vb = fabric::SemanticVersion::parse(&b.version, true);
            match (va, vb) {
                (Ok(sva), Ok(svb)) => svb.cmp(&sva),
                _ => b.version.cmp(&a.version),
            }
        });
    }
}
```

### 在 Fetch-and-Retry 中的集成

`resolve_manifest()` 在 Fetch-and-Retry 循环中按 provider name 动态选择 resolver：

```rust
// 使用 provider 特定的版本排序（Modrinth → date_published，fallback → SemVer）
let pvr: &dyn ProviderVersionResolver = if p.name() == "modrinth" {
    &ModrinthVersionResolver
} else {
    &FallbackResolver
};
pvr.sort_newest_first(&mut versions);
```

排序后的版本按序注入 PubGrub 的 `OrbitDependencyProvider`，`choose_package_version` 选取第一个满足范围约束的版本。

---

## 5. 版本比较规则

| 场景 | 比较方式 | 说明 |
|------|----------|------|
| **add 同 provider** | `date_published` 排序 | Modrinth+Modrinth 依赖链使用 `ModrinthVersionResolver` |
| **add 跨 provider / fallback** | SemanticVersion 字符串 | 其他所有情况使用 `FallbackResolver` |
| **check_local_graph** | SemanticVersion 字符串 | 永远不调用 provider resolver，仅使用 `versions/` 的 `Version` 和 PubGrub 内建约束 |

`check_local_graph()` 不访问网络，无 Provider 参与，所有版本比较均通过 `Version::parse()` 和 `Version::parse_constraint()` 在 PubGrub 内部完成。

---

## 6. 本地检查（check_local_graph）

`check_local_graph(manifest, local_mods)` 在不联网的情况下，仅凭本地已安装模组验证依赖图是否可解：

- 将本地已识别模组的 `fabric.mod.json` 依赖注入 `OrbitDependencyProvider`
- 注入虚拟依赖（minecraft, fabricloader）
- 缺失依赖注册空版本列表，让 PubGrub 给出完整冲突报告
- `orbit check` / `orbit sync` 使用此函数

---

## 7. Lockfile 查询 API

查询函数直接从 `resolver/mod.rs` 导出：

| 函数 | 签名 | 说明 |
|------|------|------|
| `find_entry(slug, entries)` | `-> Option<&LockEntry>` | 按 slug 查 lockfile（匹配 name/mod_id/slug） |
| `dependents(slug, entries)` | `-> Vec<&str>` | 反查谁依赖了 slug |
| `check_version_conflict(slug, ver, entries)` | `-> Result<(), String>` | 版本冲突检查 |
| `resolve_manifest(manifest, providers)` | `-> Result<HashMap<...>, String>` | PubGrub 依赖求解 |
| `check_local_graph(manifest, local_mods)` | `-> Result<(), String>` | 本地依赖图验证 |

---

## 8. 版本号系统

`orbit-core/src/versions/` 提供 PubGrub 所需的版本抽象：

```rust
pub enum Version {
    Lowest,                      // PubGrub 要求的无限小版本
    Fabric(SemanticVersion),     // Fabric 1:1 复刻
    Generic(String),             // 未知 loader 回退
}
```

`Version::parse_constraint(raw, loader)` 将 orbit.toml 中的约束字符串转换为 PubGrub `Range<Version>`。Fabric loader 使用 `fabric::parse_constraint()` 解析 `>=0.5, <1.0` 等格式。

---

> **关联文档**
> - [orbit-versions.md](orbit-versions.md) — 版本号解析（Fabric SemanticVersion 1:1）
> - [orbit-architecture.md](orbit-architecture.md) — resolver 在项目中的位置
> - [orbit-cli-commands.md](orbit-cli-commands.md) — add/install/upgrade 的 resolver 调用方式
