# Orbit 依赖解析引擎设计

> 本文档定义 `orbit-core/src/resolver/` 的 PubGrub 集成架构及 lockfile 查询 API。

---

## 目录

1. [模块结构](#1-模块结构)
2. [PubGrub 求解器集成](#2-pubgrub-求解器集成)
3. [Fetch-and-Retry 懒加载](#3-fetch-and-retry-懒加载)
4. [本地检查（check_local_graph）](#4-本地检查check_local_graph)
5. [Lockfile 查询 API](#5-lockfile-查询-api)
6. [版本号系统](#6-版本号系统)

---

## 1. 模块结构

```
orbit-core/src/resolver/
├── mod.rs              # resolve_manifest(), check_local_graph(), 查询 API
├── types.rs            # PackageId 类型别名
└── provider.rs         # OrbitDependencyProvider impl DependencyProvider
```

与 `versions/` 模块紧密协作——`Version` 枚举和 `parse_constraint()` 在 `versions/mod.rs` 中定义。

---

## 2. PubGrub 求解器集成

`resolve_manifest(manifest, providers)` 对 `orbit.toml` 中声明的顶层依赖执行 PubGrub 求解：

- 输入：`OrbitManifest`（顶层依赖 + MC version + loader）+ Provider 列表
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

这避免了全量预获取导致的请求海啸：Sodium 150 个版本 × 5 个前置 × 100 版本 = 650 次 → Fetch-and-Retry 只需 ~6 次。

---

## 4. 本地检查（check_local_graph）

`check_local_graph(manifest, local_mods)` 在不联网的情况下，仅凭本地已安装模组验证依赖图是否可解：

- 将本地已识别模组的 `fabric.mod.json` 依赖注入 `OrbitDependencyProvider`
- 注入虚拟依赖（minecraft, fabricloader）
- 缺失依赖注册空版本列表，让 PubGrub 给出完整冲突报告
- `orbit check` / `orbit sync` 使用此函数

---

## 5. Lockfile 查询 API

查询函数直接从 `resolver/mod.rs` 导出：

| 函数 | 签名 | 说明 |
|------|------|------|
| `find_entry(slug, entries)` | `-> Option<&LockEntry>` | 按 slug 查 lockfile（匹配 name/mod_id/slug） |
| `dependents(slug, entries)` | `-> Vec<&str>` | 反查谁依赖了 slug |
| `check_version_conflict(slug, ver, entries)` | `-> Result<(), String>` | 版本冲突检查 |
| `resolve_manifest(manifest, providers)` | `-> Result<HashMap<...>, String>` | PubGrub 依赖求解 |
| `check_local_graph(manifest, local_mods)` | `-> Result<(), String>` | 本地依赖图验证 |

---

## 6. 版本号系统

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
