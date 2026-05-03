# Orbit 项目状态

> 最后更新: 2026-05-04 (sync with code)

---

## 架构概览

```
orbit-cli ──→ orbit-core ──→ modrinth-wrapper
                              (curseforge-wrapper 待创建)
```

三层 Monorepo，Workspace 成员: `orbit-cli`, `orbit-core`, `modrinth-wrapper`

---

## 各 Crate 完成度

### modrinth-wrapper — ✅ 完成

| 模块 | 状态 | 说明 |
|------|------|------|
| `client.rs` | ✅ | HTTP 客户端（30s 超时 + check_response 保留错误 body） |
| `api.rs` | ✅ | 所有端点 + SearchParams/ListVersionsParams builder |
| `models.rs` | ✅ | Project, Version, SearchHit（含 author_id/organization）, VersionFile（含 id） |
| `error.rs` | ✅ | ModrinthError 枚举 |
| 集成测试 | ✅ | 14/14 通过（对接真实 API） |

### orbit-core — 🚧 Phase 1 完成，Phase 2 推进中（35 单测）

| 模块 | 状态 | 说明 |
|------|------|------|
| `manifest.rs` | ✅ | orbit.toml serde + 3 单测 |
| `lockfile.rs` | ✅ | orbit.lock serde + 2 单测 |
| `error.rs` | ✅ | OrbitError 枚举 (thiserror) |
| `jar.rs` | ✅ | SHA-256/512 哈希计算（FabricModInfo 已删除，统一走 metadata/） |
| `config.rs` | ✅ | GlobalConfig (分层加载) + InstancesRegistry + 4 单测 |
| `metadata/mod.rs` | ✅ | MetadataParser trait + ModMetadata + Extractor (纯内存) |
| `metadata/fabric.rs` | ✅ | FabricParser — per-field fallback + 7 单测 |
| `metadata/mojang.rs` | ✅ | McVersion::from_json — version.json + 1 单测 |
| `metadata/version_profile.rs` | ✅ | VersionProfile — launcher JSON（libraries/mainClass）+ 3 单测 |
| `detection/mod.rs` | ✅ | LoaderDetector trait + LoaderDetectionService |
| `detection/fabric.rs` | ✅ | FabricDetector — 扫描 JSON libraries 匹配 fabric-loader → Certain + 版本号 |
| `init.rs` | ✅ | detect_mc_version (JAR → version.json) + scan_mods_dir + run_init |
| `providers/mod.rs` | ✅ | ModProvider trait + 统一类型 |
| `providers/rate_limiter.rs` | ✅ | RateLimiter — acquire() 返回 Result |
| `providers/modrinth.rs` | ✅ | ModrinthProvider（含批量 API + version_constraint 过滤 + slug 解析） |
| `identification.rs` | ✅ | 批量哈希反查（get_versions_by_hashes），避免 N+1 |
| `metadata/{forge,neoforge,quilt}.rs` | 🚧 | 占位 |
| `detection/{forge,neoforge,quilt}.rs` | 🚧 | 占位 |
| `providers/curseforge.rs` | 🚧 | 骨架（待 curseforge-wrapper） |
| `versions/mod.rs` | ✅ | pub mod fabric（VersionScheme trait 已删除） |
| `versions/fabric.rs` | ✅ | Fabric SemanticVersion 1:1 复刻 + 11 单测 |
| `resolver.rs` | ✅ | lock 构建 + 依赖图查询（find_entry / dependents / check_version_conflict） |
| `sync.rs` | 🚧 | 算法占位（todo! 已改为 Err） |
| `installer.rs` | ✅ | install_to_instance + remove_from_instance + 批量 provider fallback |
| `checker.rs` | 🚧 | 逻辑占位（todo! 已改为 Err） |
| `purge.rs` | 🚧 | 逻辑占位（todo! 已改为 Err） |

### orbit-cli — ✅ 极薄层，结构对齐架构

| 模块 | 状态 | 说明 |
|------|------|------|
| `cli/mod.rs` | ✅ | 完整 clap 命令定义（16 个命令 + 全局标志 + install 可接收 mod 参数） |
| `cli/commands/init.rs` | ✅ | 自动检测 MC 版本 + Fabric loader → 仅自动失败时才交互 |
| `cli/commands/search.rs` | ✅ | 完整实现：provider.search() → facets 过滤 + 格式化输出 + ✓ 兼容标记 + slug 展示 |
| `cli/commands/install.rs` | ✅ | 单模组安装：resolve → 依赖检查 → 下载 → Not Found 搜索回退 → 交互式选择 |
| `cli/commands/remove.rs` | ✅ | 按 slug 删除 + 反查依赖图阻断 + 找不到时列出候选交互式选择 |
| `cli/commands/*` | 🚧 | 其余 12 个 handler 全部 `eprintln! + exit(2)` |
| `adaptors/` | — | ❌ 已删除 |
| `models/` | — | ❌ 已删除 |
| Cargo.toml | ✅ | 依赖 `orbit-core` + `clap` + `tokio` + `anyhow`（toml 已删除） |

---

## 命令完成度矩阵

| 命令 | CLI 入口 | Core 逻辑 | 说明 |
|------|:---:|:---:|------|
| `orbit init` | ✅ | ✅ init::run_init | Auto MC (JAR) + Fabric detect (JSON libraries) + mods/ scan |
| `orbit instances list` | ✅ | 🚧 config::InstancesRegistry | 需实现格式化输出 |
| `orbit instances default` | ✅ | 🚧 config | 需 UI |
| `orbit instances remove` | ✅ | 🚧 config | 需 UI |
| `orbit add` | ✅ | 🚧 resolver + installer | 占位，用 `orbit install <slug>` 替代 |
| `orbit install` | ✅ | ✅ installer::install_mod | 单模组安装：resolve → dep check → 下载 → JAR 解析 → toml/lock |
| `orbit remove` | ✅ | ✅ resolver::dependents | 反查依赖图 + 删除 JAR + 更新 toml/lock |
| `orbit purge` | ✅ | 🚧 purge + manifest | 需启发式搜索 |
| `orbit sync` | ✅ | 🚧 sync | **核心功能** |
| `orbit outdated` | ✅ | 🚧 resolver (只读) | 需版本比对 |
| `orbit upgrade` | ✅ | 🚧 resolver + installer | **核心功能** |
| `orbit search` | ✅ | ✅ provider::search | CLI handler + facets 过滤 + 格式化输出 |
| `orbit info` | ✅ | 🚧 provider::get_mod_info | 需格式化输出 |
| `orbit list` | ✅ | 🚧 lockfile | 需 --tree 算法 |
| `orbit import` | ✅ | 🚧 manifest | 需合并逻辑 |
| `orbit export` | ✅ | 🚧 lockfile + zip | 需打包逻辑 |
| `orbit check` | ✅ | 🚧 checker | 需 API 查询 |
| `orbit cache clean` | ✅ | 🚧 config | 需 UI |

---

## Phase 规划

### Phase 1 — ✅ 完成 (2026-05-01)

- [x] 创建 `orbit-core` crate + Monorepo 架构
- [x] `OrbitManifest` / `OrbitLockfile` serde 结构体 + 测试
- [x] `OrbitError` 统一错误类型
- [x] `GlobalConfig` 分层加载 + `InstancesRegistry`
- [x] `ModProvider` trait + `ModrinthProvider` 完整实现
- [x] `MetadataParser` trait + `FabricParser` (per-field fallback, 7 测试)
- [x] `McVersion` (mojang.rs) + `LoaderDetectionService` (detection/)
- [x] `init` 命令: 自动 MC + Fabric 检测 + mods/ 扫描 + orbit.toml 生成
- [x] 迁移 CLI 到新架构（16 命令 + 全局标志，移除 adaptors/models）

### Phase 2 — 🔜 进行中

- [x] Resolver 设计文档（`docs/orbit-resolver.md`）
- [x] `resolver.rs` 依赖图查询 API（find_entry / dependents / check_version_conflict）
- [x] `installer.rs` install_mod（resolve → dep 检查 → 下载 → JAR 解析 → toml/lock 写入）
- [x] `cli install <slug>` 单模组安装（含搜索回退 + 交互式选择）
- [x] `cli remove <mod>` 按 slug 删除（含反查依赖图 + 候选列表）
- [ ] 实现 `resolver.rs` PubGrub 求解器集成
- [ ] 实现 `sync.rs` 五态比对
- [ ] 实现 `checker.rs` 跨版本预检
- [ ] 实现 `purge.rs` 启发式搜索
- [ ] 将所有 `println!` 占位符替换为 core 调用
- [ ] 创建 `curseforge-wrapper` crate

### Phase 3 — 📋 未来

- [ ] Forge / NeoForge / Quilt parser + detector
- [ ] `orbit-core` 集成测试
- [ ] CLI 进度条与彩色输出
- [ ] 发布到 crates.io

---

## 文档索引

| 文档 | 定义 |
|------|------|
| [orbit-toml-spec.md](orbit-toml-spec.md) | 项目级 orbit.toml / orbit.lock 格式 |
| [orbit-global-config.md](orbit-global-config.md) | 全局 config.toml 规格 |
| [orbit-cli-commands.md](orbit-cli-commands.md) | 命令行为规格 |
| [orbit-metadata.md](orbit-metadata.md) | 文件元数据解析层 |
| [orbit-detection.md](orbit-detection.md) | 实例环境检测层 |
| [orbit-providers.md](orbit-providers.md) | 平台 Provider 层（RateLimiter + trait） |
| [orbit-versions.md](orbit-versions.md) | 版本号解析（Fabric SemanticVersion 1:1） |
| [orbit-resolver.md](orbit-resolver.md) | PubGrub 依赖解析引擎设计 |
| [orbit-architecture.md](orbit-architecture.md) | 项目结构、模块边界、核心接口 |
| [orbit-status.md](orbit-status.md) | 本文档 — 当前完成度追踪 |
