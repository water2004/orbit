# Orbit 依赖解析引擎

> 本文描述当前实现。依赖语义的来源是 JAR 内的 loader 元数据，所有 loader 都经过同一条求解路径。

## 1. 模块边界

```text
resolver/
├── mod.rs          公共 API 与求解编排
├── graph.rs        注册平台、物理模组、能力、内嵌模组和 Jar-in-Jar
├── constraints.rs  将规范化 any/all/unless 表达式编译为 PubGrub 子句
├── ordering.rs     加载顺序环约束与软依赖告警
├── catalog.rs      求解前闭合候选元数据图
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
中用投影排除子句枚举所有“单包不可再升级”的解；每个候选解再通过独立可行性 probe
判断是否仍能只提升一个投影包。投影之外的内部包允许变化，也不会制造重复的用户解。
这些 API 仍然只理解通用 package/version/constraint，不包含 Jar-in-Jar、loader 或
Orbit 类型。

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

## 4. `provides` 与多 provider

依赖指向的是逻辑能力，不直接绑死某个物理 JAR。求解图使用强类型
`SolverPackage`，明确区分：

- 顶层物理 mod；
- 带 owner/version/path 的 bundled occurrence；
- 逻辑 capability；
- Jar-in-Jar artifact；
- 平台包；
- provider witness。

每个逻辑 capability/version 依赖一个 provider witness；每个 witness 版本又精确
依赖一个真实物理 occurrence。因此同一能力有多个 provider 时不会被后注册者覆盖，
未选中的外层 JAR 也不能凭空提供 bundled mod 或 Jar-in-Jar。witness 的版本属于私有
`SolverVersion`，不会污染 loader `Version`、lockfile 或公共 API。

诊断观察逻辑 capability 的候选版本，而不是内部 witness。渲染器按强类型过滤
capability-to-witness 和 witness-to-occurrence 基础设施边，只呈现领域依赖事实。

## 5. 平台与运行时约束

建图先注册以下内置包：

- `minecraft`
- 当前 loader 及其官方别名
- Forge family 的 `javafml` / `lowcodefml`
- `java`

Java 版本由目标 Minecraft 版本确定；JAR 根目录 class 文件的最高 class major
又会产生模组到 `java` 的最低版本依赖。因此声明式 Java feature 和实际字节码下限都
走正常依赖边。多版本 JAR 的 `META-INF/versions/` 变体不被误当作基础运行下限。

这只能发现确定的字节码级不兼容，不能证明 API、Mixin 目标、反射或运行时行为一定
兼容。

Forge-family Jar-in-Jar 的 Maven 坐标是逻辑 artifact 包。每个内嵌 artifact 是其
外层 mod occurrence 提供的一个版本；artifact witness 精确依赖这个 owner/version。
父模组依赖声明 range，所以两个父 JAR 对同一坐标要求不相交时冲突由 PubGrub 证明，
而候选 `a@1` 绝不能借用未选中 `a@2` 里的 artifact。

## 6. 求解与补抓

`resolve_candidate_portfolio()`：

1. 从已知候选元数据收集 required 边；
2. 对 lockfile 中有明确 provider/project ID 的缺失包下载全部候选并解析真实 JAR；
3. 重复上一步直到候选目录闭合；
4. 只建一次最终图，并调用 fork 的 maximal-solution API；
5. 为每个保留解从同次 observer snapshot 生成升级、诊断和 warning 报告；
6. 唯一解直接选择；多解才交给 CLI 选择。

先闭合目录是完整枚举的前提；不能一边发现解一边补候选，否则前面排除的并不是最终
搜索空间。补抓读取已有 lockfile 的原始 provider 与 project ID；Modrinth 和
CurseForge 使用同一 catalog/注册路径，不跨平台猜别名。未知依赖仍注册为空版本列表，
表现为可解释的 `NoVersions`，而不是 provider 缓存异常。

多解的定义是：在保持其他投影包版本不变时，不存在任何一个包还能单独升级。交互界面
列出每个方案的实际升级集合；只有一个方案时不读取 stdin。`--yes` 和 dry-run 不
交互，稳定选择枚举顺序中的第一个方案。

## 7. 本地、安装与恢复路径

`check_local_graph()` 不维护第二套手写解析器或检查器。它把 `IdentifiedMod` 转成同一
`OrbitLockfile` 结构，再调用 `build_solver_graph()`。

`install`、`restore`、`sync`、`outdated` 使用相同的依赖表达式和目标环境。安装选择
也由求解结果过滤，不再用手写传递闭包推测该复制哪些 JAR。

## 8. 可读错误的约束

错误文本只呈现领域事实：

- 哪个模组需要/排斥哪个版本；
- 哪个 `any`/`all` 组无法满足；
- 哪条加载顺序形成环；
- 哪个 Jar-in-Jar 坐标区间冲突；
- 哪个环境或 Java 下限不兼容。

内部根、capability、provider witness 和物理 occurrence 边按类型隐藏；代码中不再
存在包名前缀或字符串反解析。测试断言结构化 reason 或领域文本，并明确拒绝 witness
编号、`capability` 等内部术语，不断言 PubGrub debug 输出。

## 9. 产品边界

- CurseForge：需要用户 API Key；API 没有可用下载 URL 时返回可恢复的明确错误，不
  猜测 CDN 地址。
- 远端 fork：功能分支已发布；Orbit 通过完整 commit SHA
  `0c260ff2528a6c09c683cc7270b3b97c2ea114f3` 固定依赖，不跟随可移动分支头。
- 静态字节码判断：只给出必要条件，不宣称能完整证明模组运行时兼容。
