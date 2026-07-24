# Orbit 架构

> 本文描述当前仓库边界，并把尚未落地的目标单独列出。历史迁移步骤不再混入当前模块图。

## 1. Workspace

```text
orbit-cli ───────────────→ orbit-core ───────────────→ modrinth-wrapper
                               │
                               └─────────────────────→ pubgrub-fork
```

Cargo workspace 当前包含：

| Crate | 职责 |
|-------|------|
| `orbit-cli` | clap 参数、实例上下文、交互和 stdout/stderr |
| `orbit-core` | manifest/lockfile、JAR、provider、求解、安装与文件业务逻辑 |
| `modrinth-wrapper` | Modrinth HTTP/JSON SDK，不包含 Orbit 领域逻辑 |

`pubgrub-fork` 位于仓库根目录，但暂时被 workspace 排除；`orbit-core` 通过本地 path
依赖使用它。该 fork 增加同次求解中的类型化 observer 事件，用于解释候选淘汰原因，
默认 `pubgrub::resolve()` 行为不变。

不存在 `curseforge-wrapper`。CurseForge 是保留的 provider 扩展方向，当前明确暂不
支持。

## 2. 依赖规则

```text
CLI/UI
  ↓
orbit-core 领域 API
  ↓
provider trait / jar / resolver / workspace I/O
  ↓
平台 SDK、ZIP、文件系统、PubGrub
```

边界规则：

- CLI 可以依赖 core；core 不依赖 CLI；
- CLI 不直接依赖或调用 Modrinth SDK；
- resolver 只依赖 `ModProvider` 和统一候选类型，不绑定具体 SDK；
- 元数据 parser 不做文件 I/O，JAR reader 负责 ZIP 与哈希；
- manifest/lockfile 的持久化通过 `ManifestFile` / `Lockfile`；
- core 返回类型化报告或错误，不打印用户界面文本；
- provider 专属字段放在专属子结构中，不污染公共 lockfile 字段。

## 3. `orbit-core` 模块

```text
orbit-core/src/
├── manifest.rs            orbit.toml serde 模型
├── lockfile.rs            Fat Lockfile serde 模型
├── workspace.rs           manifest/lockfile 原子业务封装
├── config.rs              全局配置与实例注册表
├── metadata/              纯字符串元数据 parser
├── jar/                   ZIP reader、下载、哈希、内嵌 JAR
├── detection/             Minecraft/loader 环境检测
├── versions/              Fabric 与 Maven 版本约束
├── providers/             provider trait、Modrinth、CF 拒绝边界
├── resolver/              PubGrub 构图、补抓、诊断、本地图校验
├── installer.rs           add/remove/upgrade/restore/list 编排
├── installer/local.rs     file: JAR 安装
├── init.rs                接管现有实例
├── sync.rs                磁盘/manifest/lockfile 对账
├── checker.rs             目标版本兼容性预检
├── outdated.rs            可升级候选查询和下载
├── purge.rs               config 候选发现与安全删除
├── archive.rs             TOML/ZIP/mrpack 导入导出
└── jar_cache.rs           缓存检查与清理
```

过去把 `sync`、`checker`、`purge`、Forge/NeoForge/Quilt parser 与 detector 标为
“占位”或 “future” 的目录树已经过时；这些模块现在都有实际实现和测试。

## 4. 数据流

### 初始化

```text
launcher profile / version.json
  → detection
mods/*.jar
  → jar reader
  → normalized metadata + hashes
  → provider hash identification
  → manifest + Fat Lockfile
  → local dependency graph check
```

无法识别平台来源的真实 JAR 仍可作为 `file` package 管理。内嵌模组记录在父 package
的 `implanted` 中，不提升为顶层 manifest 依赖。

### 在线添加与升级

```text
manifest constraint + provider candidates
  → download candidate JAR
  → parse real mod_id/version/dependencies
  → PubGrub solve_with_observer
  → structured diagnostics / selected graph
  → confirmed file replacement
  → lockfile update
```

平台的 slug 和 version number 不能代替 JAR 自声明 `mod_id` 和版本。求解图只使用
后者；来源 ID、下载 URL 和平台展示版本保存在 provider 专属字段。

### 还原

```text
manifest + lockfile
  → target/group/optional root selection
  → retain transitive closure
  → validate lock graph
  → cache / file: / provider materialization
  → checksum verification
```

`--locked`/`--frozen` 禁止重新解析缺失的来源元数据。旧 lockfile 没有
`download_url` 时，非 locked 模式可以向 Modrinth 重新查询；locked 模式只能使用缓存
或返回明确错误。

### 同步

```text
mods/ scan + hashes
  ↔ manifest
  ↔ lockfile
```

`sync` 报告 added/changed/missing/unlocked，并写回可确认的本地事实。它不下载 JAR；
为识别手动拖入的文件，哈希反查可能访问 provider。

## 5. 元数据和 loader

| Loader | 模组元数据 | 版本约束 | 环境检测 |
|--------|------------|----------|----------|
| Fabric | `fabric.mod.json` | Fabric predicate | launcher Maven 坐标 |
| Quilt | `quilt.mod.json`，兼容 Fabric JAR | Fabric predicate | launcher Maven 坐标 |
| Forge | `META-INF/mods.toml` + JarJar | Maven range | launcher Maven 坐标 |
| NeoForge | `META-INF/neoforge.mods.toml`，兼容旧文件名 | Maven range | launcher Maven 坐标 |

新增 loader 必须同时考虑 parser、JAR reader、版本模型和 detector，不能只在一个 switch
中添加字符串。

## 6. Provider

`ModProvider` 提供 search、info、resolve、hash lookup、version list、batch version 和
dependency 查询。统一类型包含 `ResolvedMod`、`SearchResultItem`、`ModInfo` 和
`ResolvedDependency`。

当前只有 `ModrinthProvider` 可创建。每个 provider 自己持有并发限制；调用方不需要
知道 SDK 客户端。配置出现未知 provider 或 `curseforge` 时立即报错，避免“配置成功
但运行时所有调用失败”的黑盒状态。

`providers/curseforge.rs` 仅定义一致的拒绝错误和未来接口形状，不注册为可用 provider。
在真正实现 SDK、哈希算法、文件选择、依赖映射和 lockfile 来源字段之前，不能把它加入
默认平台。

## 7. Resolver 和诊断

resolver 分为四个职责：

1. `graph` 将 manifest、lockfile 与候选 JAR 变为 PubGrub 输入；
2. `retry` 在不可解时补抓已知来源的缺失候选；
3. `diagnostics` 消费 fork 的类型化事件和 `DerivationTree`；
4. `local` 验证本地 JAR 图，公共入口另提供 lockfile 图校验。

成功求解中的候选淘汰原因必须来自同一次推导路径：

- 传播前排除；
- decision 后因冲突回溯；
- 仍允许但 provider 顺序选择其它版本。

这里不能退回反事实二次求解、debug 字符串解析或只看最终证明；这些做法会把“选择路径”
和“不可解证明路径”混在一起。

## 8. 并发与文件安全

已经实现的安全边界：

- provider 内部使用 semaphore 控制 HTTP 并发；
- 升级候选的下载与 JAR 验证并发执行；
- 下载先写临时文件，校验后再替换；
- ZIP/mrpack 导入拒绝路径穿越；
- cache/purge 先验证目标位于允许根目录；
- dry-run 返回计划，不写 manifest、lockfile 或目标文件。

仍有效但尚未完成的性能目标：

- restore 最终物化当前按确定顺序逐包执行；大型实例应增加有界并发，同时保持校验失败
  时不会留下半写状态；
- 全局配置中的并发数、代理、重试和 UI 选项目前主要完成了持久化模型，尚未全部接到
  HTTP/CLI 执行路径。

第二项是代码尚未满足有效配置规范，不能通过删掉配置字段来假装完成。

## 9. 当前发布边界

| 边界 | 当前策略 |
|------|----------|
| CurseForge | 明确返回暂不支持；默认仅 Modrinth |
| Java 依赖 | 所有求解路径一致忽略；不伪造运行时版本 |
| PubGrub fork | 本地 path 依赖，等待远端历史后再接发布来源 |
| restore 并发 | 正确但顺序执行，后续做有界并发 |
| 全局运行时配置 | schema/环境变量覆盖已实现，部分消费者未接入 |

## 10. 扩展检查表

新增 provider：

1. 独立 SDK 或清晰 HTTP 边界；
2. 实现全部必要 `ModProvider` 方法；
3. 定义 provider 专属 resolved/lockfile 子结构；
4. 实现真实平台哈希、主文件选择和依赖映射；
5. 添加离线契约测试和错误上下文；
6. 最后才注册到 `create_providers()` 与默认配置。

新增业务命令：

1. core 先定义输入、报告与错误；
2. I/O 通过 workspace/JAR/cache 安全边界；
3. CLI 只做参数、交互和展示；
4. dry-run 与确认语义必须在 core 写入前生效；
5. 添加成功、无变化、错误和部分状态测试；
6. 同步更新命令规范与状态快照。
