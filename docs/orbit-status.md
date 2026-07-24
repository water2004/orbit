# Orbit 实现状态

> 更新日期：2026-07-25。本文区分“正确规范曾未被代码执行”和“文档本身已经过时”。

## 1. 当前结论

仓库现有命令已接入 core 逻辑。Fabric、Quilt、Forge、NeoForge 都从真实 JAR 元数据
进入同一个规范化模型、lockfile 和 PubGrub 图；Modrinth 与 CurseForge 也共享安装、
识别、恢复、检查和升级编排。

| 能力 | 状态 | 说明 |
|---|---|---|
| 初始化与检测 | ✅ | Minecraft 与四种 loader/version |
| JAR 元数据 | ✅ | 四种 loader、多逻辑 mod、嵌套 JAR、JarJar |
| 版本语义 | ✅ | Fabric predicate；Maven ComparableVersion/range |
| 依赖求解 | ✅ | 强类型 occurrence 图、完整极大解枚举、any/all/unless、环境、provides、ordering、Java、JarJar |
| 原因 | ✅ | 自定义 reason 参与原始推导；成功候选用同次 observer |
| 本地校验 | ✅ | 转 Fat Lockfile 后复用统一建图 |
| 安装/恢复/升级 | ✅ | 由求解结果选择物理 JAR |
| Modrinth / CurseForge / `file:` | ✅ | 查询、下载、识别、锁定；CurseForge 无 API Key 时拒绝创建 |
| PubGrub fork 远端 | ✅ | 功能分支已发布，Orbit 固定到完整 commit SHA |
| 多解选择 | ✅ | 唯一解自动选择；多个单包不可升级解才交互 |
| 远端身份边界 | ✅ | provider 只给下载 locator；所有包元数据来自实际 JAR |
| Provider 分层 | ✅ | Modrinth / CurseForge HTTP 与 DTO 各在独立 wrapper，core 只做领域适配 |
| 跨平台全局路径 | ✅ | RuntimeEnvironment + 显式路径；system/executable 布局 |

## 2. 保留的正确规范

下列旧文档原则是正确的，问题曾经是代码没有遵守；本轮按规范修复，而不是删除规范：

- 所有 loader 共享同一解析后数据流和 resolver；
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

不提供旧 lockfile schema 的兼容读取层；目前没有外部 Orbit 用户需要承担这种迁移债。

## 4. 当前命令状态

| 命令 | 状态 |
|---|---|
| `init` | 扫描并验证真实实例 |
| `add` | Modrinth、CurseForge、搜索名和本地 JAR |
| `install` / `restore` | 共享求解图，按 target 选择 |
| `remove` / `upgrade` / `outdated` | 使用 Fat Lockfile 和结构化报告 |
| `sync` | 重新扫描、识别、对账 |
| `check` | 实例目标兼容性预检 |
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
- 字节码扫描只能证明 class major 下限，不能证明 API/Mixin/反射兼容。
- PubGrub fork 已发布到 `water2004/pubgrub` 的 `codex/solver-observer` 分支；
  Orbit 固定到 `0c260ff2528a6c09c683cc7270b3b97c2ea114f3`。
- 远端 project relation 会递归构造下载闭包；JAR `mod_id` 从不作为 slug/project
  查询。闭包缺少实际 required identity 时由 resolver 正常证明无解。
- JAR 缓存按本地 SHA-512 寻址，SHA-1 只作别名；provider 文件名不作为缓存键。

## 6. 文档索引

- [orbit-architecture.md](orbit-architecture.md)
- [orbit-metadata.md](orbit-metadata.md)
- [orbit-resolver.md](orbit-resolver.md)
- [orbit-versions.md](orbit-versions.md)
- [orbit-toml-spec.md](orbit-toml-spec.md)
- [orbit-detection.md](orbit-detection.md)
- [orbit-cli-commands.md](orbit-cli-commands.md)
- [orbit-providers.md](orbit-providers.md)
