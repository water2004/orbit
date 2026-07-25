# Orbit 架构

## 1. workspace

```text
orbit-cli       参数、交互和展示
    ↓
orbit-core      领域模型、编排、JAR、求解、文件事务
    ├── modrinth-wrapper
    ├── curseforge-wrapper
    ├── orbit-bytecode-audit（只依赖实际 ClassFile/refmap）
    └── water2004/pubgrub（固定 Git revision）
```

CLI 不实现业务规则。core 不打印 UI 文本，而是返回结构化报告或错误。平台 SDK、网络、
ZIP 和文件系统位于边界模块。

PubGrub fork 位于
[`water2004/pubgrub`](https://github.com/water2004/pubgrub/tree/codex/solver-observer) 的
`codex/solver-observer` 分支。Orbit 使用完整 commit SHA 固定 Git dependency，避免
分支后续移动导致构建结果变化。仓库内的 `pubgrub-fork` 仍是独立 checkout，不加入根
workspace，仅供继续开发和向 fork 推送。

`modrinth-wrapper` 与 `curseforge-wrapper` 分别拥有平台的 HTTP client、请求参数、
响应 DTO、分页和传输错误。`orbit-core/src/providers/{modrinth,curseforge}` 只把
wrapper 输出适配成统一的 `RemoteArtifact` / 查询模型。
`providers/download.rs` 是所有平台共用的 artifact transport；provider 只配置自己的
运行时认证策略，不会复制安装器或 resolver。

## 2. core 分层

```text
metadata/     loader 文件 → 规范化逻辑元数据
jar/          ZIP、manifest、嵌套 JAR、Jar-in-Jar、class major
identification/
providers/    来源查询、统一下载与受限运行时认证
runtime       跨平台目录发现、显式路径覆盖与运行时服务注入
launcher      标准/HMCL/Prism/MultiMC/CurseForge/GDLauncher 游戏目录归一化
platform      fresh discovery、Minecraft/loader JAR 定位、哈希与运行时事实
lockfile      可复现的 Fat Lockfile
versions/     Fabric predicate 与 Maven version range
resolver/
  graph       loader-neutral 建图
  constraints 依赖表达式 → PubGrub 子句
  ordering    顺序环与软依赖 warning
  diagnostics 同次求解的原因
installer/    事务、复制和恢复
package_reconciliation
              init/sync 共用的本地包候选选择与清理计划
init/sync/    实例扫描与对账
audit         实际运行时 classpath 组装；不包含字节码判定规则
    ↓
orbit-bytecode-audit
  classfile   第三方 parser 隔离 facade、稳定指令 ID
  jar         安全预算、嵌套 JAR/refmap、同名类多定义 Universe
  mixin       Mixin/MixinExtras → 统一效果
  transformer ModLauncher/Java transformer → 统一效果
  conflict    写写、写形状和普通二进制形状风险
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
  → launcher layout / fresh platform discovery
  → manifest / instance
  → provider project 闭包发现
  → 完整 artifact 队列
  → content-addressed cache / 网络
  → jar reader
  → loader adapter
  → normalized metadata
  → lock/candidate model
  → shared solver graph
  → PubGrub solution + diagnostics + warnings
  → transaction / report
```

在线安装分为三个不可反向调用的阶段：

1. provider 只按 project relation 递归枚举当前 Minecraft/loader 的 artifact；
2. 队列稳定后统一查缓存或下载，并把每个 JAR 解析为候选；
3. resolver 纯离线消费 JAR 候选，缺少实际依赖时产生正常的无解证明。

JAR `mod_id` 不会被拿去猜 provider slug，resolver 也没有联网补抓入口。
一个远端 locator 可以跨版本映射到多个真实 `mod_id`；下载后按 JAR 身份分区。新包
添加先比较各身份的可行 portfolio，已有包升级则保持 lockfile 身份，不把项目改名
伪装成普通版本升级。

长事务通过 core 的强类型 `ProgressEvent` 暴露进度，core 不写 stdout/stderr。CLI
把同一事件流渲染为交互式 spinner/进度条，非终端环境退化为逐项文本。事件边界与上述
数据流一致：project 闭包发现、候选 JAR 下载/校验/解析、离线求解、选中包物化。
并发下载任务只上报结构化完成计数，不各自操作终端。求解进度直接来自 fork observer：
enumeration continuation 与 maximality probe 的 start/finish 动态扩展并完成工作总量；
probe 内部路径不进入成功解原因轨迹。

求解包的身份恒为 JAR 声明的 `mod_id`。同一 ID 的多个顶层 `mods/*.jar` 是同一个包
的多个候选，最终每包只选一个。文件名、slug 和 project ID 只是候选来源事实，不能
变成求解包。

一个顶层包 JAR 可以包含多个同文件模块、嵌套模组 JAR 和普通库；并不是所有内嵌 JAR
都是包。含 loader 元数据的 contained 模块用 owner/source/path 绑定所选顶层候选，
普通库随 owner 一起移动而不单独求解。事务只安装或删除顶层包文件，绝不删除包内部的
单个 JAR。

## 4. 统一求解

所有入口最终调用 `build_solver_graph()` 或带 target 的变体：

- 联网候选升级；
- 本地扫描校验；
- install / restore 的选择；
- lockfile 校验；
- outdated。

依赖表达式在 `constraints.rs` 编译；加载顺序在 `ordering.rs`；平台、mod_id 候选、
`provides`、load condition 和 Jar-in-Jar 在 `graph.rs` 注册。这种拆分按职责而不是
按 loader 切开。

launcher profile 指向的实际 loader library JAR 也通过公共 JAR reader 进入平台图。
loader 自身仍是平台包，但其声明的 contained 模块使用与普通顶层包相同的
owner/source/path 绑定规则参与求解；它们不成为磁盘事务目标。

`orbit.toml [platform]` 是上次探测的路径/哈希快照，不是发现索引。`sync` 每次从 launcher
profile、Prism/MultiMC component 和当前 libraries 重新建候选集，因此允许 launcher
改名、移动或替换 JAR。`install`同样 fresh scan：Minecraft 变化是需要先 sync 的硬
边界；loader 版本变化是求解事实，不先验等同于不兼容。

PubGrub fork 允许 provider 在选择包版本时注入带 reason 的自定义 incompatibility。
条件原因因此属于真正的传播/回溯路径。observer 只补充成功解中的候选淘汰原因，不承担
另一条证明路径。fork 的最大解枚举只接收通用投影包；Orbit 直接把 `mod_id` 作为投影
包，把语义版本与来源身份组成私有候选版本。`strictly_higher` 只比较 loader 语义版本，
所以同版本不同 JAR 不是升级。该抽象由 fork 原生支持，不需要领域特判。

Jar-in-Jar artifact 使用独立的 Maven 坐标包并精确绑定 owner 候选；`provides` 使用
同一 mod_id 包下的代理候选。公共 loader `Version` 不包含来源编号，诊断也按强类型
折叠内部边，不解析名称前缀。

所有会形成新包集合的入口先得到同一种 `ResolutionReport`，再形成事务计划。唯一解
自动选择，多解由调用方选择；任何降级、替换或删除都在写盘前展示并确认。upgrade
方案只要求至少一个包相对当前版本变新，允许其他包降级。

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
| Modrinth | `modrinth-wrapper` + core adapter，可用 |
| 本地 `file:` | 可用 |
| CurseForge | `curseforge-wrapper` + core adapter，可用；无 API Key 时 provider 无法创建，Core API 与 CDN 下载均认证 |
| PubGrub fork | 已发布并固定到 `0c260ff2528a6c09c683cc7270b3b97c2ea114f3` |
| 多个极大解 | fork 原生完整枚举；唯一解自动选择，多解交给调用方选择 |

## 9. 跨平台运行环境

`RuntimeEnvironment` 是唯一允许读取宿主平台目录的 trait。Windows、Linux 和 macOS
实现分别使用 AppData、XDG/HOME 和 Library 目录；公共层只接收 `RuntimePaths`。
`RuntimeContext` 加载显式 `config.toml`、实例注册表路径和 content-addressed JAR
缓存，随后注入 CLI 调用的 core API。

调用方可传精确配置/缓存路径，也可选择 `system` 或 `executable` 布局。Cargo
`portable` feature 只把编译默认值改成 executable 布局，不取消运行时显式覆盖。
