# Orbit 实现状态

> 更新日期：2026-07-25。本文区分“正确规范曾未被代码执行”和“文档本身已经过时”。

## 1. 当前结论

仓库现有命令已接入 core 逻辑。Fabric、Quilt、Forge、NeoForge 都从真实 JAR 元数据
进入同一个规范化模型、lockfile 和 PubGrub 图；Modrinth 与 CurseForge 也共享安装、
识别、恢复、检查和升级编排。

| 能力 | 状态 | 说明 |
|---|---|---|
| 初始化与检测 | ✅ | 合法游戏目录校验；标准/HMCL/Prism/MultiMC/CurseForge/GDLauncher；Minecraft 与四种 loader/version/JAR |
| 平台工件同步 | ✅ | fresh scan 后刷新 Minecraft/loader JAR 路径、SHA-256 和版本；旧 TOML 路径不是发现入口 |
| JAR 元数据 | ✅ | 四种 loader、多逻辑 mod、嵌套 JAR、JarJar |
| 版本语义 | ✅ | Fabric predicate；Maven ComparableVersion/range |
| 依赖求解 | ✅ | 强类型 occurrence 图、完整 Pareto front、any/all/unless、环境、provides、ordering、Java、JarJar |
| 原因 | ✅ | 自定义 reason 参与原始推导；成功候选用同次 observer |
| 本地校验 | ✅ | 转 Fat Lockfile 后复用统一建图 |
| 安装/恢复/升级 | ✅ | 由求解结果选择顶层包候选并生成统一事务计划 |
| Modrinth / CurseForge / `file:` | ✅ | 查询、下载、识别、锁定；CurseForge 无 API Key 时拒绝创建 |
| PubGrub fork 远端 | ✅ | 功能分支已发布，Orbit 固定到完整 commit SHA |
| 多解选择 | ✅ | fork 原生枚举 Pareto 极大解；唯一解自动选择，多解交互 |
| 本地重复包 | ✅ | init/sync 按 mod_id 合并为候选；确认后删除未选中的顶层包版本 |
| 远端身份边界 | ✅ | provider 只给下载 locator；一个 locator 的多种真实 mod_id 按 JAR 身份分区并选择 |
| Provider 分层 | ✅ | Modrinth / CurseForge HTTP 与 DTO 各在独立 wrapper，core 只做领域适配 |
| 跨平台全局路径 | ✅ | RuntimeEnvironment + 显式路径；system/executable 布局 |
| Windows MSI | ✅ | x64 per-machine 向导、可选系统 PATH、同版本重建升级、维护模式、可选清理默认 AppData；发布产物仍需项目证书签名 |
| 长事务进度 | ✅ | 包操作与 audit 均使用 core 强类型事件；候选/审计工件精确计数，求解工作总量随实际 run/probe 动态增长 |

## 2. 保留的正确规范

下列旧文档原则是正确的，问题曾经是代码没有遵守；本轮按规范修复，而不是删除规范：

- 所有 loader 共享同一解析后数据流和 resolver；
- launcher 实际 loader JAR 的依赖和内嵌模块必须进入同一平台图；
- 本地校验与联网候选不能走两套规则；
- 依赖原因必须来自实际推导路径；
- 不允许第二次反事实求解或日志解析充当证明；
- JAR 解析只能通过 `jar` 层；
- provider 专属数据位于专属子结构；
- CLI 不承载业务逻辑；
- 平台不可用、缺认证或文件禁止 API 下载时必须明确报错。

## 3. 已迁移的过时文档

下列描述本身已经过时，现已从文档和 schema 中删除：

- `implanted` / `implanted_mods` / `[[package.implanted]]`：改为递归 `bundled`；
- `(mod_id, version, required)` 依赖 tuple：改为
  `DependencyExpression` + `ModDependency`；
- “只取 Forge 第一个 `[[mods]]`”：现在保留全部逻辑模组；
- “Java 依赖统一忽略”：现在注册 Minecraft 推导 Java，并加入 class major/feature 约束；
- “fork 只增加 observer”：现在还支持 provider-defined incompatibility clauses；
- “Forge/NeoForge/Quilt 属于 future phase”：四者已经进入完整路径；
- “optional/env 不影响传递图”：现在按真实语义和 target 建图。
- “Jar-in-Jar 是全局无依赖叶子”：现在 artifact 版本由 owner-bound occurrence
  提供，未选中候选不能提供内容。
- “resolver 动态补抓候选”：下载层先按远端 project relation 构造完整 artifact
  队列并统一解析，resolver 此后严格离线。
- “每个物理 JAR 是独立求解包”：求解包现为 JAR 声明的 `mod_id`，文件与嵌套路径
  只区分候选；顶层文件才是安装/删除单元。
- “同一 ID 的嵌套版本必须全部满足”：现在按 Fabric/Quilt load condition 选择一个
  loader 可加载候选，Forge-family JarJar 按 artifact range 选择。

不提供旧 lockfile schema 的兼容读取层；目前没有外部 Orbit 用户需要承担这种迁移债。

## 4. 当前命令状态

| 命令 | 状态 |
|---|---|
| `init` | 拒绝空/任意目录，定位真实平台 JAR，扫描实例并确认重复包清理 |
| `add` | Modrinth、CurseForge、搜索名和本地 JAR |
| `install` / `restore` | fresh platform scan；Minecraft 变化拒绝，loader 变化由共享图判定 |
| `remove` / `upgrade` / `outdated` | 使用 Fat Lockfile、保留受阻候选原因、自适应表格与多解差异高亮 |
| `sync` | 重新探测平台并扫描 mods，刷新工件快照，按包选择候选并确认移除未选版本 |
| `check` | 实例目标兼容性预检 |
| `audit` | 复用 Loader-selected runtime，解析实际 Mixin/Transformer 注册与 ClassFile；分类摘要 + schema 3 JSON/显式完整 report |
| `list` / `info` | 展示包信息、逻辑依赖和 bundled |
| `export` / `import` | Orbit archive 与 Modrinth pack |
| `cache` / `instances` / `purge` | 已接 core |

## 5. 已知边界

- CurseForge Core API 和 CDN 下载需要用户申请的 Key；provider 不提供匿名降级。
  Key 仅在运行时下载客户端中存在，并限定为 HTTPS `forgecdn.net` 域名。仓库自动测试
  使用 mock server，不读取开发者私人 Key。live smoke test 需要显式提供
  `ORBIT_CURSEFORGE_API_KEY`。
- CurseForge API 未公开本地 fingerprint 算法；实现明确依据 Prism Launcher 的公开
  源码并用 golden vectors 固定行为。
- 普通安装扫描只能证明 class major 下限；`orbit audit` 另行分析 Mixin、
  ModLauncher transformer 和二进制形状风险，但仍只报告潜在风险，不能证明兼容，
  也不覆盖资源、配置、注册表、网络协议、反射目标或游戏业务逻辑。
- PubGrub fork 已发布到 `water2004/pubgrub` 的 `codex/solver-observer` 分支；
  Orbit 固定到 `c334509daecf91611af2729b2db91af7eba6f076`。
- 当前 fork 原生支持 `P = mod_id`、不透明复合候选版本、调用方定义
  `same_version` / `strictly_higher` 和完整 Pareto front 枚举；同声明版本的载体身份不会
  扩成多个用户解，每个保留点会一次排除完整支配区域，无效版本序回调会在产生错误排除
  前失败。upgrade 的“至少一个包变新、其他包可降级”是对同批 Pareto 解的操作分类。
- 远端 project relation 会递归构造下载闭包；JAR `mod_id` 从不作为 slug/project
  查询。闭包缺少实际 required identity 时由 resolver 正常证明无解。
- `sync` 保持本地对账且不下载修复；`install` 才构造远端候选闭包修复依赖图。
- 共享游戏根目录若同时暴露多个 Minecraft/loader 候选，没有通用办法从目录本身判断
  launcher 下一次会启动哪一个；Orbit 明确报歧义，要求使用隔离实例或在 init 显式选择，
  不按目录顺序猜测。
- JAR 缓存按本地 SHA-512 寻址，SHA-1 只作别名；provider 文件名不作为缓存键。
- project 闭包的总工作量事前未知，因此显示当前 locator、已发现 artifact 与耗时。
  Pareto 枚举的总量随 continuation run/maximality probe 的实际发现而增长，完成数同步
  推进；它不构成剩余耗时上界，Pareto 或 co-Pareto front 本身仍可能很大。候选 JAR
  下载/校验/解析及最终物化使用预先稳定的精确总数。

## 6. 文档索引

- [orbit-architecture.md](orbit-architecture.md)
- [orbit-metadata.md](orbit-metadata.md)
- [orbit-resolver.md](orbit-resolver.md)
- [orbit-versions.md](orbit-versions.md)
- [orbit-toml-spec.md](orbit-toml-spec.md)
- [orbit-detection.md](orbit-detection.md)
- [orbit-cli-commands.md](orbit-cli-commands.md)
- [orbit-providers.md](orbit-providers.md)
- [orbit-bytecode-audit.md](orbit-bytecode-audit.md)
