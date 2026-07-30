# Orbit 架构

## 1. workspace

```text
orbit-cli       参数、交互和展示
  output        自适应表格、逻辑包事务、多方案差异高亮、audit 摘要
    ↓
orbit-core      领域模型、编排、JAR、求解、文件事务
    ├── modrinth-wrapper
    ├── curseforge-wrapper
    ├── orbit-bytecode-audit（只依赖已选择的实际 JAR 内容与运行时环境）
    └── water2004/pubgrub（固定 Git revision）
```

CLI 不实现业务规则。core 不打印 UI 文本，而是返回结构化报告或错误。CLI `output`
使用 `comfy-table` 渲染更新、诊断、事务、方案差异和 audit 文本报告；颜色只是增强，
`◆` 才是可重定向的差异语义。表格在 TTY 中服从终端宽度，无法探测宽度的重定向输出
以 120 列为上限。平台 SDK、网络、ZIP 和文件系统位于边界模块。

`orbit --output-format json` 与 `orbit-launcher --output-format json` 配合各自的
`--progress-format ndjson`，直接复用
`orbit-machine-protocol` 的 schema 2 成功、错误、进度和交互信封。原生 GUI 只是启动两个
CLI 进程：读取 stdout 最终结果与 stderr NDJSON，并向同一子进程 stdin 写回交互选择；
不存在 GUI 专用参数、旧 schema 别名、备用 JSON 路径或 core 直连。

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

查询模型还统一携带 provider 官方返回的 icon、RGB accent、project links 与 gallery，
供 CLI JSON/原生 GUI 展示。它们与 `RemoteArtifact` 严格分离：展示 URL 不得成为包身份、
远端 locator、候选版本或下载可信依据。Modrinth adapter 映射 project/search 字段；
CurseForge wrapper 先按官方 Core API 的 `ModLinks` / `ModAsset` DTO 解析，再由 core
adapter 映射，CLI/GUI 不直接理解 provider 专属响应。

## 2. core 分层

```text
metadata/     loader 文件 → 规范化逻辑元数据
jar/          ZIP、manifest、嵌套 JAR、Jar-in-Jar、class major
identification/
providers/    来源查询、统一下载与受限运行时认证
runtime       跨平台目录发现、显式路径覆盖与运行时服务注入
launcher      标准/HMCL/Prism/MultiMC/CurseForge/GDLauncher 游戏目录归一化
platform_detection
              仅供 init/sync 使用的 launcher 探测、JAR 定位和快照生成
platform      TOML 平台快照的精确路径解析、哈希与元数据校验
lockfile      可复现的 Fat Lockfile
versions/     Fabric predicate 与 Maven version range
resolver/
  graph       消费归一化 LoaderSemantics 的统一建图
  constraints 依赖表达式 → PubGrub 子句
  ordering    顺序环与软依赖 warning
  diagnostics 同次求解的原因
installer     精确 lock 物化与唯一修复事务
migration     面向已安装目标运行时的共享迁移规划与导出
init/sync     平台探测、本地事实扫描与清单对账
audit         复用 resolver 的 Loader-selected runtime；不包含字节码判定规则
    ↓
orbit-bytecode-audit
  backend/    Fabric/Quilt/Forge/NeoForge 的 ABI、namespace、注册和 transformer 策略
  classfile   第三方 parser 隔离 facade、稳定指令 ID
  jar         安全预算、活动嵌套 JAR/resource、MR-JAR、同名类多定义 Universe
  namespace   backend 调用的共享 runtime symbol alignment、Tiny 投影/readiness
  mixin_config Loader 注册、端侧/requiredMods/plugin 激活、config/refmap 作用域
  mixin       候选类合并；selector/slice → InjectionQuery；injector → Mutation
  transformer FML ServiceLoader 图 → ModLauncher ITransformer / NeoForge ClassProcessor → 统一效果
  conflict    独立风险原因、行为交互、query 重算、遮蔽后的硬引用风险
```

允许出现 loader 分支的位置：

- 元数据文件名与字段映射；
- loader 自身检测；
- 版本约束语义；
- loader 官方定义的嵌套格式；
- audit 的 ABI、namespace、Mixin 注册入口与 Transformer 能力。

不允许出现 loader 分支的位置：

- lockfile 的依赖数据模型；
- 本地/联网求解；
- 安装选择；
- 错误证明路径；
- sync/outdated 的图语义；
- audit 的 JAR/ClassFile 扫描、统一效果模型和冲突合成。

“统一”指四个 loader 在边界适配后消费同一个领域模型，不表示把不一致的运行时规则
塞进公共分支。内部 loader 身份是封闭的 `LoaderKind`；TOML、CLI 和 provider 参数只在
边界转换为字符串。新增 loader 不允许走 unknown/generic fallback。

## 3. 端到端数据流

```text
init / sync
  → launcher layout / platform_detection
  → 完整 platform snapshot

联网求解命令（add/fix/constraint set/upgrade/migrate）
  → manifest / exact platform snapshot validation
  → package remotes 的 provider project 闭包发现（联网命令）
  → 完整 artifact 队列
  → content-addressed cache（命中即 touch）/ 网络
  → jar reader
  → loader adapter
  → normalized metadata
  → lock/candidate model
  → shared solver graph
  → PubGrub solution + diagnostics + warnings
  → transaction / report
  → 命令结束合并 LRU 索引并执行容量淘汰
```

联网求解分为三个不可反向调用的阶段：

1. manifest、lock 与本次输入中的全部 package remotes 同时作为种子，provider 只按
   project relation 递归枚举当前 Minecraft/loader 的 artifact；
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
的多个候选，最终每包只选一个。候选以本地计算的内容哈希保持唯一；完全相同的字节跨
provider 合并为一个候选并累积来源，同版本不同字节仍是不同候选。哈希、文件名、slug
和 project ID 都不能变成求解包，也不能作为正常交互中的包名。

一个顶层包 JAR 可以包含多个同文件模块、嵌套模组 JAR 和普通库；并不是所有内嵌 JAR
都是包。含 loader 元数据的 contained 模块用 owner/source/path 绑定所选顶层候选，
普通库随 owner 一起移动而不单独求解。用户和事务计划操作的最小单元始终是逻辑包；
执行层只为这个包物化或移除对应的顶层 artifact，绝不把包内部的单个 JAR 当删除目标。

## 4. 统一求解

所有需要判定可行包集合的入口最终调用 `build_solver_graph()` 或带 target 的变体：

- 联网候选升级；
- 本地扫描校验；
- fix 与 migrate 的选择；
- lockfile 校验；
- outdated。

依赖表达式在 `constraints.rs` 编译；加载顺序在 `ordering.rs`；平台、mod_id 候选、
`provides`、load condition 和 Jar-in-Jar 在 `graph.rs` 注册。这种拆分按职责而不是
按 loader 复制 resolver。`LoaderSemantics` 在建图前给出版本体系、规范平台包、平台
capability 与 nested priority；graph 内不比较 loader 字符串。

launcher profile 指向的实际 loader library JAR 也通过公共 JAR reader 进入平台图。
loader 自身仍是平台包，但其声明的 contained 模块使用与普通顶层包相同的
owner/source/path 绑定规则参与求解；它们不成为磁盘事务目标。

`orbit.toml [platform]` 是完整、强制的运行时快照：Minecraft JAR、Loader JAR、
其余 launcher runtime JAR、物理端，以及每个文件的 SHA-256。`platform_detection`
封装 launcher profile、Prism/MultiMC component、Maven 坐标和目录候选等不稳定规则，
生产代码中只有 `init`、`sync` 和面向另一个真实游戏目录的 migration planner 可以引用
它。migration 的探测只用于读取用户明确指定的目标实例，不会刷新源实例。
`platform` 则是无发现能力的严格消费者。

`sync` 每次忽略旧快照，从当前 launcher 状态重建并整体替换快照，因此允许 launcher
改名、移动、替换或升级 JAR。install/outdated/upgrade/archive export/audit 不 fresh scan、
不修改 `[platform]`、不寻找替代文件：路径、哈希或 JAR 元数据与快照不符就要求先
`orbit sync`。同步后的 loader 版本变化仍是求解事实，不先验等同于不兼容。

PubGrub fork 允许 provider 在选择包版本时注入带 reason 的自定义 incompatibility。
条件原因因此属于真正的传播/回溯路径。observer 只补充成功解中的候选淘汰原因，不承担
另一条证明路径。fork 的最大解枚举只接收通用投影包；Orbit 直接把 `mod_id` 作为投影
包，把语义版本与来源身份组成私有候选版本。`same_version` 把同一 loader 语义版本的
全部来源身份映射为一个等价类，`strictly_higher` 只覆盖更高语义版本；枚举和 probe
因此按包版本而非载体身份区分方案。fork 验证两个范围的基本序关系，避免无效排除导致
同一投影重复。该抽象由 fork 原生支持，不需要领域特判。

同一语义版本的不同内容哈希天然是不同的私有 `SolverVersion`，因此仍可携带不同
依赖约束参与一次求解；语义投影又保证它们不被误当成版本升级。Orbit 只需为 CLI
提供 project/release 与依赖差异描述，不需要再次修改 fork 或把哈希显示给用户。

manifest 包策略由 Loader 对应的数字核心范围与完整原始版本字符串规则组成。数字规则的
操作数只能是任意段点分无符号整数；字符串规则看到 JAR 声明的全部文本，不拆前缀或后缀，
也不解释 `beta`、`snapshot` 或 Loader 名。规则从 all/none 集合开始，逐项执行交、并、原子
取反和整体取补；建图时对 provider 已注册的有限候选同时应用两部分规则，并把允许候选的
singleton 并集作为根约束。它不是求解后的校验路径。Fabric/Quilt 的 Loader-valid 不透明
版本只旁路无法适用的数字规则，完整字符串规则照常生效；Forge/NeoForge 在 JAR 元数据入口
按 Loader 的 `^\d+.*` 规则拒绝无效声明。

Jar-in-Jar artifact 使用独立的 Maven 坐标包并精确绑定 owner 候选；`provides` 使用
同一 mod_id 包下的代理候选。公共 loader `Version` 不包含来源编号，诊断也按强类型
折叠内部边，不解析名称前缀。

所有会形成新包集合的入口先得到同一种 `ResolutionReport`，再形成事务计划。当前包括
`add`、`fix`、结构化 `constraint set`、`upgrade` 和迁移规划，不包括只记录事实的
`init`/`sync`，也不包括只按 lock 物化的 `install`。优化目标由入口显式传入统一 resolver：
`add` / `fix` / `constraint set` 以当前 lock 为基线枚举标准 Pareto 极小逻辑包变更集合；
`upgrade` / `outdated` 枚举标准版本 Pareto 极大 front。迁移先要求全部源包；严格无解且
用户许可后，以源 manifest 包的保留状态枚举标准 Pareto 极小删除集合。极小变更或删除集合
固定后仍以版本极大作为次级目标。fork 对每个保留点
一次排除完整支配区域。唯一解自动选择，多解由调用方选择；任何降级、替换或删除都在写盘
前展示并确认。upgrade 方案只要求至少一个包相对当前版本变新，允许其他包降级。

`sync` 只调用可用 provider 的批量哈希识别接口，把本地精确内容恢复为
project/release 远端，并按磁盘事实重建 lock；它不枚举版本、不下载候选 JAR、不求解，
也不删除重复实现。发现同一 `mod_id` 的多个本地实现时，sync 保留全部文件和 TOML
来源并要求运行 `fix`。`install` 只物化现有 lock 的精确内容，既不求解也不修改
TOML/lock。联网候选闭包发现、可行解选择和未选包删除由 `add`、`fix`、`constraint set`、`upgrade` 与
迁移等明确求解操作执行；这些写操作共享同一个 reconciliation，使 TOML 完整包集合与
所选 lock 同步收敛。

迁移先通过普通 archive exporter 冻结一个校验通过的便携源实例；该步骤成功后 Launcher
才创建真实目标实例。随后同一个 `migration::plan_migration()` 从便携源读取包与配置事实、
从目标读取 Minecraft/Loader JAR，枚举目标版本候选并选择 Pareto 解。`migrate check` 只
展示这份计划；`migrate export` 复用同一计划写入目标 `orbit.toml`、`orbit.lock` 和
配置文件，拒绝覆盖已有目标状态。导出不把模组 JAR 安装到 `mods/`；入选的 file-only 内容
会按哈希保存到目标 `.orbit/sources`，在线内容仍由随后执行的 `orbit install` 按新 lock 精确
物化，因而预检和导出不会走两条推导路径。便携源包不会替代真实目标平台
探测；它只消除源实例在目标创建期间发生变化的竞态。便携包为精确恢复注入的源实例
`file` 载体也不会污染迁移候选：有 Provider project 的逻辑包只枚举目标版本远端，只有
file-only 包保留本地 JAR 并由同一个求解图检查其真实 Loader/Minecraft 依赖。

严格迁移和允许删包迁移不是两个下载/解析管线。候选闭包只构建一次；软解只改变求解图中
manifest 包的 root 角色，并把每个包的可选中状态交给 fork 的原生偏好 front。平台包与
manifest 版本范围始终是硬约束。默认 CLI 在严格无解后通过同一进程的结构化交互请求许可，
GUI 不持有策略状态或调用 core。求解器对未选候选产生的传播诊断是内部决策依据；迁移成功
结果只保留实际删除顶层包的解释，不把已被排除的源版本或 bundled 候选展示成错误。

受管包环境具有两层正交语义：`orbit.toml` 的可选 `env` 是用户 target 过滤，lock 的
`environment` 是精确候选从 JAR 解析出的事实。TOML 缺失设置时，locked 路径直接使用
选中候选的环境；无 lock 的候选路径使用候选集合的真实 Loader 声明。Loader 没有包级
声明时 adapter 产生 `both`。init/sync 不把推导结果反写成用户策略，`orbit env` 只修改
manifest 过滤。

## 5. loader 支持矩阵

| Loader | 元数据适配 | 版本语义 | 嵌套选择 | audit 后端 | 规范化求解 |
|---|---|---|---|---|---|
| Fabric | `fabric.mod.json` | Fabric predicate | `jars` + parent priority | Fabric | 统一 graph |
| Quilt | `quilt.mod.json` / Fabric fallback | Fabric predicate | Quilt `jars` 条件 | Quilt | 统一 graph |
| Forge | `META-INF/mods.toml` | Maven | JarJar | Forge/ModLauncher | 统一 graph |
| NeoForge | `META-INF/neoforge.mods.toml` / legacy name | Maven | JarJar | NeoForge/ClassProcessor（旧运行时按实际 ABI 走 ModLauncher） | 统一 graph |

“支持”意味着 identity、依赖类别、环境、版本、provides、内嵌和求解都进入真实路径，
不是只识别文件名。

## 6. 可维护性规则

- 规范化类型表达语义，不用 tuple/字符串标志隐藏含义。
- loader 差异必须进入 `LoaderSemantics`、metadata adapter 或 audit backend；共享流水线
  不允许重新从字符串推断 loader，也不允许 generic fallback。
- 新字段先进入 metadata model，再向 candidate/lock/solver 传播。
- 不保留旧 lock schema 的兼容分支；项目尚无外部使用者，schema 直接收敛。
- parser 对身份和结构错误 fail fast。
- JAR 内 loader-owned JSON 只兼容字符串内未转义控制字符；适配集中在
  `orbit-loader-json`，网络、launcher、lock 和 cache JSON 保持严格。
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
| PubGrub fork | 已发布并固定到 `914cf645982ba790090652bf3a09d934de857408` |
| 多个 Pareto 解 | fork 原生完整枚举变更极小或版本极大 front；唯一解自动选择，多解交给调用方选择 |

## 9. 跨平台运行环境

`RuntimeEnvironment` 是唯一允许读取宿主平台目录的 trait。Windows、Linux 和 macOS
实现分别使用 AppData、XDG/HOME 和 Library 目录；公共层只接收 `RuntimePaths`。
`RuntimeContext` 加载显式 `config.toml`、实例注册表路径和 content-addressed JAR
缓存，随后注入 CLI 调用的 core API。

调用方可传精确配置/缓存路径，也可选择 `system` 或 `executable` 布局。Cargo
`portable` feature 只把编译默认值改成 executable 布局，不取消运行时显式覆盖。
