# Orbit 依赖解析引擎

> 本文描述当前实现。依赖语义的来源是 JAR 内的 loader 元数据，所有 loader 都经过同一条求解路径。

## 1. 模块边界

```text
resolver/
├── mod.rs          公共 API 与求解编排
├── graph.rs        注册包候选、模块、provides、load condition 和 Jar-in-Jar
├── constraints.rs  将规范化 any/all/unless 表达式编译为 PubGrub 子句
├── ordering.rs     加载顺序环约束与软依赖告警
├── local.rs        把本地扫描结果转换为 lockfile 后复用统一建图
├── provider.rs     内存 DependencyProvider
└── diagnostics/    成功路径观察与不可解证明的领域化渲染
```

Fabric、Quilt、Forge、NeoForge 不各自拥有 resolver。TOML/CLI 的 loader 字符串在边界
解析为封闭 `LoaderKind`；`LoaderSemantics` 决定版本体系、规范平台包、capability 与
nested priority。各 loader 只在 `metadata/` 和 `versions/` 适配输入，之后统一进入：

```text
loader metadata
  → ModFileMetadata / ModMetadata
  → JarModMetadata
  → PackageEntry / CandidateVersion
  → build_solver_graph
  → PubGrub
```

## 2. 为什么使用 PubGrub fork

上游 `DependencyProvider::get_dependencies()` 只能表达“包版本依赖另一个包的版本
范围”，不能表达 Orbit 所需的 n 元互斥、条件依赖、Quilt 分组和加载顺序环。

[`water2004/pubgrub`](https://github.com/water2004/pubgrub/tree/codex/solver-observer)
增加：

```rust
fn get_incompatibilities(
    &self,
    package: &Self::P,
    version: &Self::V,
) -> Result<Vec<IncompatibilityConstraint<...>>, Self::Err>;
```

每个 provider 子句都带类型化正/负 term 和领域 `reason`。求解器在选择该包版本时，
把声明者 term 与这些 term 一起加入当前 `State`。因此原因参与原始传播、冲突学习和
`DerivationTree`，不是求解后再猜测。

这项定制是必要的：如果推导和解释分别运行，第一次的选择/回溯路径可能与第二次不同，
就会出现“结果正确，但解释对应另一条路径”的问题。Orbit 不运行反事实第二次求解，
也不解析 debug 日志。

fork 仍保留 observer。observer 只解释一次成功求解里“某个候选为何未被选择”：

- `ExcludedByPropagation`
- `Backtracked`

不可解原因则直接来自同次求解的 `DerivationTree`，其中也包含 Orbit 自定义子句的
`reason`。

fork 还提供 `resolve_maximal_solutions_with_observer()`。它在同一个 solver session
中枚举完整 Pareto front：先用 probe 寻找一个可行方案，使所有已选投影包保持等价或
变高且至少一个严格变高；成功时继续从该方案提升，直到不存在支配它的方案。保留一个
Pareto 点后，fork 一次排除它支配的整个区域，而不是逐个阻塞低版本组合。投影之外的
内部包允许变化，也不会制造重复的用户解。这些 API 仍然只理解通用
package/version/constraint，不包含 Jar-in-Jar、loader 或 Orbit 类型。

`resolve_minimal_change_solutions_with_observer()` 则接受一组独立的包状态偏好，并枚举
未满足偏好集合按包含关系 Pareto 极小的全部解。若一个解未满足的偏好集合是另一解的真
超集，它会被排除；互不包含的集合都会保留。因此这是标准集合 Pareto 极小，不是最小基数、
加权打分或按枚举顺序挑一个。偏好集合固定后，fork 再用上述版本序枚举该变更集合内的
版本 Pareto 极大 front。偏好 probe 与 maximality probe 都是 fork 的原生求解阶段，
Orbit 不循环调用黑盒可行性测试。

probe 的 start/finish 事件带有结果。成功 probe 中的决定、传播和回溯成为当前候选的
真实路径；失败 probe 的 observer 状态回滚。因此最终诊断仍来自产生该 Pareto 解的
实际推导，不是事后反事实重跑。

Orbit 已核对过这项 API 与当前包语义的边界：`P` 直接使用 JAR 声明的 `mod_id`，`V` 是
求解器视为不透明值的复合候选。Orbit 分别提供 `same_version(V)`、
`same_precedence(V)` 与 `strictly_higher(V)`：第一项覆盖同一物理内容实现；第二项覆盖
数字核心相同的所有完整版本表示与内容候选；第三项只覆盖数字核心更高的候选。因此同一版本或
同一数值核心的不同实现不是升级，却仍保留为不同用户方案。fork 会验证等价范围包含当前
候选且不与严格更高范围重叠；无效回调直接返回
`InvalidVersionOrdering`，不能加入一个未排除当前投影的子句后原地重复。包/候选建模
与 Pareto 枚举均由 fork 原生抽象覆盖，不需要在 Orbit 中做第二次求解。

已安装内容在建图时用 `lock:sha512:<digest>`（或 SHA-256）保持选择优先级，下载目录中的
同一字节内容使用不带 `lock:` 的身份。两者必须落入同一个 `same_version` 等价类；它们只是
同一 JAR 的两个图内表示，不能产生“保留/重新安装同一内容”的用户方案。只有内容哈希不同的
JAR 才是不同实现，即使它们声明了相同版本。

`upgrade` 另有一个操作层条件：相对当前安装集合，方案中至少存在一个
`PackageChangeKind::Upgrade`。这只是对 fork 一次性返回的 Pareto 解做分类；方案中的其他包
允许降级、替换或删除。它不改变可行性定义，也不产生另一条证明路径。

## 3. 规范化依赖语义

`constraints.rs` 处理 loader-neutral 的 `DependencyExpression`：

- `Only`：单一关系；
- `Any`：至少一个分支满足；
- `All`：全部分支满足；
- `unless`：条件成立时禁用当前关系。

关系类别：

| kind | 求解行为 |
|---|---|
| `required` | 必须存在且版本匹配 |
| `optional` | 不主动安装；存在时必须版本匹配 |
| `recommended` | 不阻止解；缺失或版本不符产生 warning |
| `suggested` | 仅保留元数据，不影响解 |
| `incompatible` | 匹配时形成硬冲突 |
| `discouraged` | 不阻止解；匹配时产生 warning |

环境在建图时按 `client`、`server` 或保守的 `both` 目标计算。Forge/NeoForge 的
`BEFORE`/`AFTER` 会变为版本精确的有向图；选中组合构成环时，环本身是 PubGrub
自定义不相容关系，错误会显示完整路线。

## 4. 包、候选与 `provides`

求解器中的模组包只有一个身份轴：JAR loader 元数据声明的 `mod_id`。物理文件名、
provider slug、project ID、下载 URL 和嵌套路径都不能成为 `SolverPackage`。

一个包可以有多个候选版本。私有 `SolverVersion` 同时保存：

- loader 语义版本；
- `CandidateIdentity { owner, source, path, location, installed }`。

远端顶层候选的 `source` 是 Orbit 对实际字节计算的内容哈希；已安装候选也优先使用
内容哈希。哈希只保证两个具体候选不被错误覆盖，不参与版本高低比较，也不进入正常
CLI 文本。顶层 `mods/*.jar` / `mods/*.jar.disabled` 是包的具体候选，身份中的 `path` 为空且
`owner == mod_id`。同一 `mod_id` 的多个顶层 JAR 是同一个包的不同候选，即使声明了
相同版本也不能互相覆盖，因为依赖元数据或内容可能不同。最终解每个包只选择一个候选。

一个顶层包 JAR 可以包含多个同文件模块或嵌套 JAR；并不是每个嵌套 JAR 都是包。
只有含 loader 模组元数据的模块才进入 `Mod(mod_id)` 候选，普通库只作为包内容随 owner
移动。contained 候选带 owner/source/path，并精确依赖 owner 候选，所以不能脱离外层包
单独安装或删除：

- 同一元数据文件声明的模块与 owner 原子选择；
- Fabric 嵌套模组按 `if_possible` 优先加载，无兼容候选时可以省略；
- Quilt 使用 `always` / `if_possible` / `if_required`；
- Forge-family JarJar 先按 Maven `group:artifact` 区间选 artifact；若 artifact 本身
  声明模组，再把相应模块绑定到同一个 owner。

`provides` 也不创建“物理包”。它在被提供的 `mod_id` 下注册一个代理候选，并精确依赖
实际 owner 候选。因此多个 provider 可以正常竞争，未选中的顶层包不能凭空提供能力。

## 5. 平台与运行时约束

建图先注册以下内置包：

- `minecraft`
- 当前 loader 及其官方别名
- Forge family 的 `javafml` / `lowcodefml`
- `java`

当前 loader 不是只凭 manifest 版本注册成无依赖叶子。Orbit 从 launcher version
profile 找到实际 Maven library JAR，再通过同一个 JAR reader 读取 loader 自身的
依赖、provides 与内嵌模块。loader 根包使用平台版本身份，内嵌的 MixinExtras 等真实
模块则以 owner-bound contained 候选进入普通 `Mod(mod_id)` 图，可以满足其它模组的
正常依赖，同时不会出现在安装、删除或 lockfile 事务中。找不到实际 library JAR 时
才退化为只注册 loader 版本；不会硬编码某个 loader 附带的具体模块。
`minecraft` 与 canonical loader 是 root 的必选依赖，而不是等到某个 Mod 声明依赖时
才进入解；因此 Loader 自己的依赖、端侧条件和嵌套 load condition 始终参与同一次求解，
audit 也能直接消费该解中的实际 Loader archive chain。

Java 版本由目标 Minecraft 版本确定；JAR 根目录 class 文件的最高 class major
又会产生模组到 `java` 的最低版本依赖。因此声明式 Java feature 和实际字节码下限都
走正常依赖边。多版本 JAR 的 `META-INF/versions/` 变体不被误当作基础运行下限。

这只能发现确定的字节码级不兼容，不能证明 API、Mixin 目标、反射或运行时行为一定
兼容。

Forge-family Jar-in-Jar 的 Maven 坐标是逻辑 artifact 包。每个内嵌 artifact 是其
外层包候选提供的一个版本；artifact 候选精确依赖这个 owner/version。
父模组依赖声明 range，所以两个父 JAR 对同一坐标要求不相交时冲突由 PubGrub 证明，
而候选 `a@1` 绝不能借用未选中 `a@2` 里的 artifact。

## 6. 下载闭包与纯离线求解

联网编排在调用 `resolve_candidate_portfolio()` 之前完成：

1. 用用户输入、manifest 全部受管包和 lockfile 中的全部确切 `remotes` 作为种子；
2. 按 provider 批量读取 project 级变更标记；每个精确 Minecraft/loader 作用域拥有独立
   `remote.sqlite`，未变化的 project 直接复用快照，变化的 project 才用精确游戏版本和
   loader 过滤枚举可下载版本，绝不请求全部游戏版本；
3. 只沿 provider project relation 递归，直到远端 project 闭包稳定；
4. 按 SHA-512/SHA-1 对完整 artifact 队列去重后统一处理；当前作用域独立的
   `jars.sqlite` 先按内容哈希复用 Loader 分析，未命中才访问全局 LRU JAR cache 或下载；
5. 每个新内容校验来源强哈希并解析真实 JAR metadata，再以真实 `mod_id` 建候选；
6. 对实际字节自行计算 SHA-512：相同内容跨 provider 合并来源，不同内容即使
   `mod_id + version` 相同也保持独立；
7. 把完整 `CandidateCatalog` 交给纯离线 resolver；
8. 建一次最终图，并按命令调用 fork 的 minimal-change 或 maximal-solution API；
9. 唯一解直接选择；多解才交给 CLI 选择。

发现阶段报告的 artifact/JAR 数量是尚未建立依赖图的原始候选数，不是 Pareto 方案数。
不能只按版本号提前删除看似更旧或相等的候选：同版本不同内容可能声明不同依赖，较新版本也
可能因约束不可行。完成 JAR 解析和统一建图以后，求解器才按完整方案的支配关系裁剪；同一内容
在 lock 与下载目录中的重复表示则由 `same_version` 等价类在枚举阶段一次排除。

这些边界同时是进度事件边界。版本库先报告批量检查、刷新/复用 project 与动态增长的
project 总量；队列稳定后按去重内容报告候选 JAR 完成数；纯离线求解报告包/候选规模和
动态工作量。fork 对每个 enumeration continuation run、preference probe 和 maximality probe 发出成对
start/finish 事件；UI 在 start 时扩大总量，在 finish 时推进，并额外显示 decision、
propagation、backtrack、conflict 与 retained solution 计数。probe 内部决策仍使用
noop observer，不能污染用于解释候选淘汰原因的成功路径。

provider 的 dependency relation 仅用于定位下一批 project，不携带可信的 required、
版本或 `mod_id` 语义。JAR dependency 也不会反向触发 provider 查询，因为 `mod_id`
不是 slug。若下载闭包中没有 JAR 声明某个 required identity，建图会把该引用注册为
空版本包，并由 PubGrub 产生可解释的无可行解。

`resolve_candidate_portfolio()` 不持有 provider、下载器或缓存，也不会动态联网。
这保证下载失败、JAR 解析和依赖求解是三个清楚的错误边界。

JAR 分析库没有 provider project ID，远端库也不产生 solver package。project ID 只用于
刷新下载定位快照；从数据库进入 `CandidateCatalog` 时仍必须读取 JAR 分析中的真实
`mod_id`、版本和依赖。版本库由所有候选命令内部维护，不提供另一条 `fetch` 工作流。

同一 provider locator 可能跨 artifact 声明多个 `mod_id`。catalog 对它们按包身份
分区；`add` 对每个真实身份独立求可行 portfolio 后选择身份，upgrade 则固定现有
lockfile 身份。这个选择发生在下载完成、纯离线求解开始之后，不把 slug 当包名。

`upgrade` / `outdated` 多解的定义是标准版本 Pareto 极大：不存在另一个可行方案，使全部
已选用户包版本等价或更高，并且至少一个严格更高。候选来源不是“更高版本”的第二条坐标。
这个定义会删除全面落后的方案，但保留“某些包升级、另一些包必须降级”的真实权衡。

`add` / `fix` / 结构化 `constraint set` 使用标准 Pareto 极小变更集合。Orbit 给 fork 的
偏好坐标由逻辑包构成：

- TOML 与 lock 都有的包偏好保留 lock 中的精确候选身份；
- TOML 有而 lock 没有的包是必须实现的意图，不设“保持缺失”偏好；
- 不在 TOML 中的候选包偏好保持不存在。

方案的变更坐标就是未满足的这些偏好。如果 A 的变更集合是 B 的真子集，B 被支配；
`{change A}` 与 `{change B}` 则都保留。固定一个极小集合后，再以版本 Pareto 极大作为
次级目标，所以不可避免的新包或已确定要变化的包不会留下全面更旧的组合。

交互界面列出每个方案的安装、
升级、降级、同版本替换和删除。共同动作只列一次，每个选项用 `◆` 标记与其他选项不同
的逻辑包动作。同版本不同内容的选项用 provider project/release 与 JAR-declared
依赖差异描述；物理文件名和哈希不属于用户决策，不进入这张表。只有一个方案时不读取 stdin。
dry-run 仍会在多解时请求选择，因为它预览的必须是一个确定方案；`--yes` 只跳过最终
写入确认，不能替用户挑选真实包身份或 Pareto 解。stdin 关闭、取消或无效机器响应都会
终止选择，不能回退到枚举顺序中的第一个。

fork 对每个保留点排除完整支配区域，因此独立包的低版本不会形成需要逐项检查的笛卡尔积。
Pareto front 或 co-Pareto front 本身仍可能很大；动态工作量说明求解器正在检查新区域，
但不是完成时间上界。

## 7. 本地、安装与恢复路径

`check_local_graph()` 不维护第二套手写解析器或检查器。它把 `IdentifiedMod` 转成同一
`OrbitLockfile` 结构，再调用 `build_solver_graph()`。它只校验一份已经确定的本地
选择，不替 `init`/`sync` 选择重复实现。

`add`、`fix`、结构化 `constraint set`、`upgrade`、`outdated` 与 `migrate` 消费同一种
`ResolutionReport`。
不论是多个 Pareto 极小变更解还是多个版本 Pareto 极大解，都进入同一选择协议；选择完成
后统一生成包事务计划。未选中的顶层包版本会列出
精确 `mod_id`、版本和动作，实际写入或删除前必须确认；文件名只供事务执行层定位载体。
即使方案唯一也不能跳过破坏性计划确认。嵌套 JAR 从不作为独立删除目标。

`constraint set` 不先写 TOML 再调用另一个修复命令。core 在内存中的 manifest 副本上
应用 Any、单边界或有界区间策略，建立与 fix 相同的完整候选闭包并选择 Pareto 极小方案；
确认后由同一事务一起提交 JAR、lock 与 manifest。若当前选中 JAR 已满足策略，则只原子
持久化 manifest；无解、用户取消、dry-run 或物化失败都不得留下新策略。GUI 只从
`versions` 的真实 JAR 候选选取边界并生成结构化 CLI 动作，不解析或拼接 Loader 约束文本。

`install` 不进入候选求解：它严格校验现有 lock 并只恢复其中记录的内容哈希。
`init`/`sync` 也不进入候选求解：它们扫描当前磁盘事实；同一 `mod_id` 出现多个实现时
保留所有顶层文件与 TOML source，并要求 `fix` 选择。这样事实采集、精确物化与修复没有
隐含的第二条包选择路径。

升级选择不会先丢弃“不含升级”的 Pareto 解再遗失其 observer 快照。统一的
`select_upgrade_resolution` 先保留所有解上的候选诊断，再筛选批量升级解或“指定包自身
升级”的单包解；若筛选为空，返回无变更但携带诊断的报告。若 provider 对当前
Minecraft/loader 没有返回任何 JAR 声明某个已锁 `mod_id`，outdated 另行产生
`NoCompatibleCandidate`，不能将“没有可分析候选”表述为“已是最新”。

`add` 的候选闭包不只包含新 locator 及其递归远端项目。因为加入新包的可行方案允许
现有包升级或降级，lockfile 中每个在线包的 provider project 也必须贡献当前
Minecraft/loader 下的全部 JAR 候选。所有 project 先完成版本枚举并进入同一下载队列，
统一下载、读取真实 JAR 元数据后才交给求解器；只给现有包的锁定版本会制造假冲突。

`sync` 对本地 `mods/`、manifest 与 lockfile 对账，并批量调用可用 provider 的哈希识别
接口恢复 remote 与精确 artifact source；它不下载候选 JAR，也不构造远端候选闭包。
仅所有 provider 都未匹配的内容才记录为本地持久来源。`fix` 才会通过完整远端 artifact
闭包重建候选并修复缺失或不兼容的包。

`fix` 成功提交时以所选逻辑包集合同时收敛三个状态：删除 `mods/` 中未选顶层实现，
从 `orbit.lock` 删除未选包/候选，并让 `orbit.toml` 的完整包集合、group 引用和有效
remote 与选择收敛；最后清理不再被 TOML/lock 引用的 `.orbit/sources`。不能出现
“磁盘删了但 lock/TOML 仍记录”的半套删除语义。

迁移 planner 使用目标实例真实平台和空的目标安装状态构图。默认首先把全部源 manifest 包
作为 root 硬要求；严格图无解时，先把真实推导原因交给调用方，只有用户同意才进入软解。
软解仍使用同一候选图：Minecraft、Loader 和包自身 TOML 版本范围保持硬约束，源包是否被
选中改为 fork 的 `PackagePreference::selected` 坐标。fork 对未满足的包状态集合做标准
Pareto 极小枚举，再在同一保留集合内做版本 Pareto 极大化。因此它求的是“没有任何一个被
移除包能在不牺牲其他已保留包时恢复”的完整 front，不是最小数量启发式，也不是逐包删除后
重试。约束不允许的候选在建图时已经从该逻辑包定义域移除，不能经传递依赖绕过 manifest。

`migrate check` 与 `migrate export` 直接复用这个严格优先规划器。GUI 首次 check 若接受软解，
会根据已审阅计划中的删除动作给 export 传入 `--allow-removals`，避免重复确认；export 仍重建
同一完整候选闭包并执行同一目标函数，不存在 GUI 专用求解。export 产生可在目标由 `install`
精确物化的 lock。便携源快照为离线恢复添加的 `file` 载体不属于目标版本候选：只要同一包
存在 Modrinth/CurseForge project，迁移发现层就丢弃该恢复载体并按目标 Minecraft/Loader
重新枚举 project；真正没有在线 project 的本地包才保留其文件来源。无论候选来自哪一层，
其 JAR 声明的 Minecraft/Loader 约束仍作为 PubGrub 图中的硬边，目标不兼容候选只能被求解
排除，不能进入方案。成功方案只报告实际被软迁移删除的包诊断，不把被排除的源版本或内置
依赖候选渲染成迁移失败。入选的 file-only 内容在 export 时转存到目标的内容寻址 source
store；`mods/` 仍只由后续 `install` 从最终 lock 物化。

## 8. 可读错误的约束

错误文本只呈现领域事实：

- 哪个模组需要/排斥哪个版本；
- 哪个 `any`/`all` 组无法满足；
- 哪条加载顺序形成环；
- 哪个 Jar-in-Jar 坐标区间冲突；
- 哪个环境或 Java 下限不兼容。

内部根、候选身份、load preference 和 artifact 绑定边按类型隐藏；代码中不再存在
包名前缀或字符串反解析。测试断言结构化 reason 或领域文本，不断言 PubGrub debug
输出。

## 9. 产品边界

- CurseForge：需要用户 API Key；API 没有可用下载 URL 时返回可恢复的明确错误，不
  猜测 CDN 地址。
- 远端 fork：功能分支已发布；Orbit 通过完整 commit SHA
  `914cf645982ba790090652bf3a09d934de857408` 固定依赖，不跟随可移动分支头。
- 静态字节码判断：只给出必要条件，不宣称能完整证明模组运行时兼容。
