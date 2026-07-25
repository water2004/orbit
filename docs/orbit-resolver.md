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

Fabric、Quilt、Forge、NeoForge 不各自拥有 resolver。它们只在
`metadata/` 和 `versions/` 适配输入，之后统一进入：

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

probe 的 start/finish 事件带有结果。成功 probe 中的决定、传播和回溯成为当前候选的
真实路径；失败 probe 的 observer 状态回滚。因此最终诊断仍来自产生该 Pareto 解的
实际推导，不是事后反事实重跑。

Orbit 已核对过这项 API 与当前包语义的边界：`P` 可以直接使用 JAR 声明的 `mod_id`，
`V` 是求解器视为不透明值的复合候选，`same_version(V)` 与 `strictly_higher(V)` 均由
Orbit 定义。前者覆盖同一 JAR 声明版本的所有来源身份，后者只覆盖更高 JAR 内版本的
所有来源身份。因此同版本不同来源既不是升级，也不会仅因载体身份制造不同用户方案。
fork 会验证等价范围包含当前候选且不与严格更高范围重叠；无效回调直接返回
`InvalidVersionOrdering`，不能加入一个未排除当前投影的子句后原地重复。包/候选建模
与 Pareto 枚举均由 fork 原生抽象覆盖，不需要在 Orbit 中做第二次求解。

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

顶层 `mods/*.jar` 是包的具体候选，身份中的 `path` 为空且 `owner == mod_id`。同一
`mod_id` 的多个顶层 JAR 是同一个包的不同候选，即使它们声明了相同版本也不能互相
覆盖，因为依赖元数据或文件来源可能不同。最终解每个包只选择一个候选。

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

1. 用用户输入或 lockfile 中的 provider project locator 作为种子；
2. 对每个 project 枚举当前 Minecraft/loader 的全部可下载版本；
3. 只沿 provider project relation 递归，直到远端 project 闭包稳定；
4. 将完整 artifact 队列统一交给 content-addressed cache/下载器；
5. 每个 artifact 校验来源强哈希并解析真实 JAR metadata；
6. 把完整 `CandidateCatalog` 交给纯离线 resolver；
7. 建一次最终图并调用 fork 的 maximal-solution API；
8. 唯一解直接选择；多解才交给 CLI 选择。

这些边界同时是进度事件边界。project 递归发现报告当前 provider locator 和已发现
artifact 数；队列稳定后报告每个候选 JAR 的完成数；纯离线求解报告包/候选规模和
动态工作量。fork 对每个 enumeration continuation run 和 maximality probe 发出成对
start/finish 事件；UI 在 start 时扩大总量，在 finish 时推进，并额外显示 decision、
propagation、backtrack、conflict 与 retained solution 计数。probe 内部决策仍使用
noop observer，不能污染用于解释候选淘汰原因的成功路径。

provider 的 dependency relation 仅用于定位下一批 project，不携带可信的 required、
版本或 `mod_id` 语义。JAR dependency 也不会反向触发 provider 查询，因为 `mod_id`
不是 slug。若下载闭包中没有 JAR 声明某个 required identity，建图会把该引用注册为
空版本包，并由 PubGrub 产生可解释的无可行解。

`resolve_candidate_portfolio()` 不持有 provider、下载器或缓存，也不会动态联网。
这保证下载失败、JAR 解析和依赖求解是三个清楚的错误边界。

同一 provider locator 可能跨 artifact 声明多个 `mod_id`。catalog 对它们按包身份
分区；`add` 对每个真实身份独立求可行 portfolio 后选择身份，upgrade 则固定现有
lockfile 身份。这个选择发生在下载完成、纯离线求解开始之后，不把 slug 当包名。

多解的定义是标准 Pareto 极大：不存在另一个可行方案，使全部已选用户包版本等价或更高，
并且至少一个严格更高。候选来源不是“更高版本”的第二条坐标。这个定义会删除全面落后的
方案，但保留“某些包升级、另一些包必须降级”的真实权衡。交互界面列出每个方案的安装、
升级、降级、同版本替换和删除。共同动作只列一次，每个选项用 `◆` 标记与其他选项不同
的逻辑包动作；物理文件名不属于求解决策，不进入这张表。只有一个方案时不读取 stdin。
dry-run 仍会在多解时请求选择，因为它预览的必须是一个确定方案；`--yes` 才稳定选择
枚举顺序中的第一个。

fork 对每个保留点排除完整支配区域，因此独立包的低版本不会形成需要逐项检查的笛卡尔积。
Pareto front 或 co-Pareto front 本身仍可能很大；动态工作量说明求解器正在检查新区域，
但不是完成时间上界。

## 7. 本地、安装与恢复路径

`check_local_graph()` 不维护第二套手写解析器或检查器。它把 `IdentifiedMod` 转成同一
`OrbitLockfile` 结构，再调用 `build_solver_graph()`。`init` 与 `sync` 进一步通过
`package_reconciliation` 共享 `select_local_packages()`：本地同 ID 文件进入同一个
候选集合，而不是按扫描顺序覆盖。

`add`、非 locked `install`、`restore`、`upgrade`、`sync`、`init` 和 `outdated` 都消费
同一种 `ResolutionReport`。多个 Pareto 极大解统一选择；选择完成后统一生成包事务计划。
未选中的顶层包版本会列出精确 `mod_id`、版本和动作，实际写入或删除前必须确认；文件名
只供事务执行层定位载体。即使方案唯一也不能跳过破坏性计划确认。嵌套 JAR 从不作为
独立删除目标。

升级选择不会先丢弃“不含升级”的 Pareto 解再遗失其 observer 快照。统一的
`select_upgrade_resolution` 先保留所有解上的候选诊断，再筛选批量升级解或“指定包自身
升级”的单包解；若筛选为空，返回无变更但携带诊断的报告。若 provider 对当前
Minecraft/loader 没有返回任何 JAR 声明某个已锁 `mod_id`，outdated 另行产生
`NoCompatibleCandidate`，不能将“没有可分析候选”表述为“已是最新”。

`add` 的候选闭包不只包含新 locator 及其递归远端项目。因为加入新包的可行方案允许
现有包升级或降级，lockfile 中每个在线包的 provider project 也必须贡献当前
Minecraft/loader 下的全部 JAR 候选。所有 project 先完成版本枚举并进入同一下载队列，
统一下载、读取真实 JAR 元数据后才交给求解器；只给现有包的锁定版本会制造假冲突。

`sync` 只对本地 `mods/`、manifest 与 lockfile 对账，不联网下载候选来修复冲突；
`install` 才会通过完整远端 artifact 闭包重建候选并修复缺失或不兼容的包。

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
  `c334509daecf91611af2729b2db91af7eba6f076` 固定依赖，不跟随可移动分支头。
- 静态字节码判断：只给出必要条件，不宣称能完整证明模组运行时兼容。
