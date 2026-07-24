# Orbit 实现状态

> 更新日期：2026-07-24。本文区分“正确规范曾未被代码执行”和“文档本身已经过时”。

## 1. 当前结论

除 CurseForge 外，仓库现有命令已接入 core 逻辑。Fabric、Quilt、Forge、NeoForge
都从真实 JAR 元数据进入同一个规范化模型、lockfile 和 PubGrub 图。

| 能力 | 状态 | 说明 |
|---|---|---|
| 初始化与检测 | ✅ | Minecraft 与四种 loader/version |
| JAR 元数据 | ✅ | 四种 loader、多逻辑 mod、嵌套 JAR、JarJar |
| 版本语义 | ✅ | Fabric predicate；Maven ComparableVersion/range |
| 依赖求解 | ✅ | any/all/unless、六类关系、环境、provides、ordering、Java、JarJar |
| 原因 | ✅ | 自定义 reason 参与原始推导；成功候选用同次 observer |
| 本地校验 | ✅ | 转 Fat Lockfile 后复用统一建图 |
| 安装/恢复/升级 | ✅ | 由求解结果选择物理 JAR |
| Modrinth / `file:` | ✅ | 查询、下载、识别、锁定 |
| CurseForge | ⏸ | 明确暂不支持，不静默回退 |
| fork 远端 | ⏳ | 本地提交完成后等待用户远端历史 |

## 2. 保留的正确规范

下列旧文档原则是正确的，问题曾经是代码没有遵守；本轮按规范修复，而不是删除规范：

- 所有 loader 共享同一解析后数据流和 resolver；
- 本地校验与联网候选不能走两套规则；
- 依赖原因必须来自实际推导路径；
- 不允许第二次反事实求解或日志解析充当证明；
- JAR 解析只能通过 `jar` 层；
- provider 专属数据位于专属子结构；
- CLI 不承载业务逻辑；
- CurseForge 未实现时必须明确报错。

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

不提供旧 lockfile schema 的兼容读取层；目前没有外部 Orbit 用户需要承担这种迁移债。

## 4. 当前命令状态

| 命令 | 状态 |
|---|---|
| `init` | 扫描并验证真实实例 |
| `add` | Modrinth、搜索名和本地 JAR；`cf:` 明确失败 |
| `install` / `restore` | 共享求解图，按 target 选择 |
| `remove` / `upgrade` / `outdated` | 使用 Fat Lockfile 和结构化报告 |
| `sync` | 重新扫描、识别、对账 |
| `check` | 实例目标兼容性预检 |
| `list` / `info` / `why` | 展示逻辑依赖和 bundled |
| `export` / `import` | Orbit archive 与 Modrinth pack |
| `cache` / `config` / `instance` / `purge` | 已接 core |

## 5. 已知边界

- CurseForge 下载、搜索、识别和升级均不支持。
- 字节码扫描只能证明 class major 下限，不能证明 API/Mixin/反射兼容。
- PubGrub fork 当前是本地 path dependency；接远端要保留用户 fork 的历史。
- 补抓传递候选依赖已有 lockfile 来源信息；不会凭别名猜远端项目。

## 6. 文档索引

- [orbit-architecture.md](orbit-architecture.md)
- [orbit-metadata.md](orbit-metadata.md)
- [orbit-resolver.md](orbit-resolver.md)
- [orbit-versions.md](orbit-versions.md)
- [orbit-toml-spec.md](orbit-toml-spec.md)
- [orbit-detection.md](orbit-detection.md)
- [orbit-cli-commands.md](orbit-cli-commands.md)
- [orbit-providers.md](orbit-providers.md)
