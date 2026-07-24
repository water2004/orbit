# Orbit 架构

## 1. workspace

```text
orbit-cli       参数、交互和展示
    ↓
orbit-core      领域模型、编排、JAR、求解、文件事务
    ├── modrinth-wrapper
    ├── CurseForge typed client（core provider 边界内）
    └── pubgrub-fork（本地 path dependency）
```

CLI 不实现业务规则。core 不打印 UI 文本，而是返回结构化报告或错误。平台 SDK、网络、
ZIP 和文件系统位于边界模块。

`pubgrub-fork` 是独立 Git 仓库，目前不加入根 workspace。它先保留本地有序历史；
用户提供 fork 远端后再接远端历史。

CurseForge 的 HTTP/JSON 位于 `providers/curseforge/{client,models}.rs`，平台映射位于
同目录 `mod.rs`。`providers/download.rs` 是所有平台共用的 artifact transport；
provider 只配置自己的运行时认证策略，不会复制安装器或 resolver。

## 2. core 分层

```text
metadata/     loader 文件 → 规范化逻辑元数据
jar/          ZIP、manifest、嵌套 JAR、Jar-in-Jar、class major
identification/
providers/    来源查询、统一下载与受限运行时认证
lockfile      可复现的 Fat Lockfile
versions/     Fabric predicate 与 Maven version range
resolver/
  graph       loader-neutral 建图
  constraints 依赖表达式 → PubGrub 子句
  ordering    顺序环与软依赖 warning
  retry       候选补抓
  diagnostics 同次求解的原因
installer/    事务、复制和恢复
init/sync/    实例扫描与对账
```

允许出现 loader 分支的位置：

- 元数据文件名与字段映射；
- loader 自身检测；
- 版本约束语义；
- loader 官方定义的嵌套格式。

不允许出现 loader 分支的位置：

- lockfile 的依赖数据模型；
- 本地/联网求解；
- 安装选择；
- 错误证明路径；
- sync/outdated 的图语义。

## 3. 端到端数据流

```text
命令
  → manifest / instance
  → provider 或本地 JAR
  → jar reader
  → loader adapter
  → normalized metadata
  → lock/candidate model
  → shared solver graph
  → PubGrub solution + diagnostics + warnings
  → transaction / report
```

一个物理 JAR 可以包含多个逻辑模组。顶层 `PackageEntry` 对应物理文件的主逻辑包，
其余逻辑模组递归位于 `bundled`。它们参与同一求解图，但不会生成不存在的独立文件。

## 4. 统一求解

所有入口最终调用 `build_solver_graph()` 或带 target 的变体：

- 联网候选升级；
- 本地扫描校验；
- install / restore 的选择；
- lockfile 校验；
- outdated。

依赖表达式在 `constraints.rs` 编译；加载顺序在 `ordering.rs`；平台、capability、
Jar-in-Jar 和物理包注册在 `graph.rs`。这种拆分按职责而不是按 loader 切开。

PubGrub fork 允许 provider 在选择包版本时注入带 reason 的自定义 incompatibility。
条件原因因此属于真正的传播/回溯路径。observer 只补充成功解中的候选淘汰原因，不承担
另一条证明路径。

## 5. loader 支持矩阵

| Loader | 元数据 | 版本 | 嵌套 | 求解 |
|---|---|---|---|---|
| Fabric | `fabric.mod.json` | Fabric predicate | `jars` | 完整统一路径 |
| Quilt | `quilt.mod.json` / Fabric fallback | Fabric predicate | `jars` | 完整统一路径 |
| Forge | `META-INF/mods.toml` | Maven | JarJar | 完整统一路径 |
| NeoForge | `META-INF/neoforge.mods.toml` / legacy name | Maven | JarJar | 完整统一路径 |

“支持”意味着 identity、依赖类别、环境、版本、provides、内嵌和求解都进入真实路径，
不是只识别文件名。

## 6. 可维护性规则

- 规范化类型表达语义，不用 tuple/字符串标志隐藏含义。
- 新字段先进入 metadata model，再向 candidate/lock/solver 传播。
- 不保留旧 lock schema 的兼容分支；项目尚无外部使用者，schema 直接收敛。
- parser 对身份和结构错误 fail fast。
- 测试断言公开行为、结构化 reason 和领域错误，不解析 debug 日志。
- 暂不支持的产品边界必须显式报错并给出恢复建议。

## 7. 静态兼容性边界

Orbit 能确定：

- loader 元数据声明的版本/端侧冲突；
- class major 要求的最低 Java；
- Maven/Fabric 版本范围；
- Jar-in-Jar artifact 冲突；
- 加载顺序环。

Orbit 不能仅凭字节码完整证明：

- Minecraft/loader API 调用一定存在；
- Mixin 目标和映射一定正确；
- 反射、native code、配置或其他模组交互一定安全。

因此静态扫描只产生可证明的必要条件，不把“没有发现问题”描述为“保证兼容”。

## 8. 当前外部边界

| 边界 | 状态 |
|---|---|
| Modrinth | 可用 |
| 本地 `file:` | 可用 |
| CurseForge | 可用；无 API Key 时 provider 无法创建，Core API 与 CDN 下载均认证 |
| PubGrub fork 远端 | 等用户提供 fork 历史后接入 |
