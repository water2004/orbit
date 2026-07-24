# Orbit 项目状态

> 最后更新：2026-07-24
>
> 本文是实现快照。行为规范仍由 `orbit-cli-commands.md`、`orbit-toml-spec.md`
> 和各专题设计文档定义；快照与规范冲突时，需要判断是历史描述过时，还是代码尚未满足规范。

## 当前结论

除 CurseForge 外，仓库内已有 CLI 命令都已接入实际 core 逻辑，不再保留
`exit(2)`、`todo!()` 或“返回未实现错误”的命令占位。

```
orbit-cli ──→ orbit-core ──→ modrinth-wrapper
                  │
                  └──→ pubgrub-fork（本地定制依赖）
```

Workspace 成员为 `orbit-cli`、`orbit-core`、`modrinth-wrapper`。
`pubgrub-fork` 暂时排除在 workspace 外，等待用户提供 fork 远端历史后再接入发布来源。

## 已实现能力

| 范围 | 状态 | 当前实现 |
|------|:---:|----------|
| 实例管理 | ✅ | 注册、列出、设置默认实例、移除追踪 |
| 初始化与检测 | ✅ | 自动检测 MC；检测 Fabric、Forge、NeoForge、Quilt 及 loader 版本 |
| JAR 元数据 | ✅ | Fabric JSON、Forge/NeoForge TOML、Quilt JSON、内嵌 JAR/JarJar |
| 版本约束 | ✅ | Fabric/Quilt 语义约束；Forge/NeoForge Maven 版本区间 |
| 依赖求解 | ✅ | 定制 PubGrub observer、统一构图、补抓、override/exclude、结构化诊断 |
| 安装与恢复 | ✅ | add、`file:`、Fat Lockfile、target/group/optional、locked/frozen、校验与缓存 |
| 本地一致性 | ✅ | sync 四类差异报告、check、remove、purge |
| 查询与更新 | ✅ | search、info、list/tree/target、outdated、单包/全量 upgrade |
| 导入导出 | ✅ | TOML 合并、安全 ZIP；mrpack index 下载/双哈希校验与 overrides 导入导出 |
| 缓存 | ✅ | 检查、确认、dry-run、安全清理 |
| CurseForge | ⏸ | **明确暂不支持**；不会静默创建不可用 provider |

`orbit-core` 当前有 96 个单元测试，`orbit-cli` 有 1 个上下文安全测试；
`modrinth-wrapper` 有 14 个默认离线的 API 契约测试，另有 2 个 doctest。全工作区通过：

```text
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## 命令矩阵

| 命令 | Core 入口 | 状态与边界 |
|------|-----------|------------|
| `orbit init` | `init::run_init` | 四种 loader 检测与现有 mods 扫描 |
| `orbit instances *` | `config::InstancesRegistry` | 完整 |
| `orbit add` | `installer` + `resolver` | Modrinth 与 `file:`；`cf:` 返回明确暂不支持错误 |
| `orbit install` | `installer::restore_instance` | target/group/no-optional/locked/frozen 完整 |
| `orbit remove` | `installer::remove_from_instance` | 含被依赖检查 |
| `orbit purge` | `purge` + `installer` | 配置候选逐项确认，限制在 config 根目录内 |
| `orbit sync` | `sync::sync_instance` | 不下载 JAR；哈希识别可能查询 provider |
| `orbit outdated` | `outdated::check_all_outdated` | 只读；支持 mod_id/slug 校验 |
| `orbit upgrade` | `installer` + `outdated` | 单包必须已安装；本地文件没有在线升级源 |
| `orbit search` / `info` | `ModProvider` | 当前可用 provider 为 Modrinth |
| `orbit list` | `installer::list_installed*` | 展示 provider/env/optional，target 保留传递闭包 |
| `orbit import` / `export` | `archive` + `archive::mrpack` | ZIP 路径防护；mrpack URL 白名单、大小/双哈希校验与 overrides |
| `orbit check` | `checker::check_compatibility` | 在线包预检；本地文件无平台兼容信息 |
| `orbit cache clean` | `jar_cache` | 安全根校验、确认、dry-run |

## 文档差异分类

### 已经过时、应删除的历史描述

以下描述曾经正确，但现在只会误导维护者：

- `install`、`sync`、`check`、`purge`、`import`、`export`、实例管理或缓存命令仍是 stub；
- Forge、NeoForge、Quilt parser/detector 属于 future phase；
- `list --target` 被忽略，或 `file:` 只存在于 CLI 语法而没有实现；
- core 只有 62 个单元测试；
- `init` 只能检测 Fabric，或为未知 loader 写入假的 `0.0.0` 版本；
- mrpack 导出只是“普通 ZIP 加一个 index”，或导入忽略 index 下载。

这些属于状态快照过时，不代表原行为规范错误。

### 规范仍正确、实现仍有明确边界

| 优先级 | 规范/目标 | 当前边界 |
|:---:|-----------|----------|
| P1 | 配置的平台应当可实际使用 | CurseForge wrapper/provider 尚未实现；新清单默认只启用 Modrinth，显式配置 CurseForge 会直接报错 |
| P2 | Java 约束应校验实际运行时 | 候选图和本地图目前一致忽略 Java，避免伪造版本；尚未探测实例实际 Java |
| P2 | 大规模恢复应并发下载 | 候选验证并发，最终文件恢复目前仍按确定顺序逐个物化 |
| P2 | core 可独立发布 | 仍使用本地 `pubgrub-fork` path 依赖，需等 fork 远端接入 |
| P2 | 全局配置应控制运行时 | schema 与环境变量覆盖已实现；代理、重试、认证、语言/UI 和下载并发尚未全部接入 |
| P3 | CLI 全局输出约定 | `--quiet` / `--verbose` 和用户取消退出码尚未统一到结构化输出层 |

这几项不能因为尚未完成就从规范里删除；它们应作为显式边界保留。

## CurseForge 策略

CurseForge 当前保持暂不支持：

- 不创建 `curseforge-wrapper`；
- 不把空实现注册成可用 provider；
- `[resolver].platforms` 默认值仅为 `["modrinth"]`；
- `cf:` 或显式 `curseforge` 配置返回可读错误；
- 文档示例不得宣称已支持 CurseForge 下载、查询或升级。

## 文档索引

| 文档 | 定义 |
|------|------|
| [orbit-toml-spec.md](orbit-toml-spec.md) | orbit.toml / orbit.lock 数据格式 |
| [orbit-global-config.md](orbit-global-config.md) | 全局配置 |
| [orbit-cli-commands.md](orbit-cli-commands.md) | CLI 行为 |
| [orbit-metadata.md](orbit-metadata.md) | JAR 元数据解析 |
| [orbit-detection.md](orbit-detection.md) | 实例与 loader 检测 |
| [orbit-providers.md](orbit-providers.md) | Provider 抽象 |
| [orbit-versions.md](orbit-versions.md) | 版本与约束 |
| [orbit-resolver.md](orbit-resolver.md) | PubGrub 求解和诊断 |
| [orbit-architecture.md](orbit-architecture.md) | 模块边界 |
