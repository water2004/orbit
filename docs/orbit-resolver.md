# Orbit 依赖解析引擎

> 本文描述当前实现。依赖语义的来源是 JAR 内的 loader 元数据，所有 loader 都经过同一条求解路径。

## 1. 模块边界

```text
resolver/
├── mod.rs          公共 API 与求解编排
├── graph.rs        注册平台、物理模组、能力、内嵌模组和 Jar-in-Jar
├── constraints.rs  将规范化 any/all/unless 表达式编译为 PubGrub 子句
├── ordering.rs     加载顺序环约束与软依赖告警
├── retry.rs        同一路径上的缺失候选补抓
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
- `ProviderPreferred`

不可解原因则直接来自同次求解的 `DerivationTree`，其中也包含 Orbit 自定义子句的
`reason`。

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

依赖指向的是逻辑能力，不直接绑死某个物理 JAR。Orbit 为每个普通 mod ID 建立内部
capability 包；真实同名模组和 loader 元数据中的 `provides` 都能提供它。

同一能力、同一版本若有多个 provider，会建立 provider-choice 包。PubGrub 选择其中
一个具体 provider，而不会由后注册者覆盖前一个。物理模组、能力和选择包相互约束，
所以 incompatible、optional、warning 和加载顺序看到的是同一“存在性”。

内部包名不会写入 lockfile，也不会出现在安装列表；诊断渲染时还原为逻辑 mod ID。

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

Forge-family Jar-in-Jar 每个 Maven 坐标也是内部包。父模组依赖其声明 range，
内嵌 artifact version 是候选版本；两个父 JAR 对同一坐标要求不相交时，冲突直接由
PubGrub 证明。

## 6. 求解与补抓

`resolve_with_candidates_report()`：

1. 从 manifest、lockfile 和候选 JAR 建图；
2. 在一次 PubGrub 运行中收集候选事件；
3. 若 `NoSolution` 暴露尚未加载、且 lockfile 已知来源的 required 依赖，则下载并
   解析真实 JAR；
4. 通过同一个 `register_candidate_versions()` 增量注册后重新求解；
5. 无法继续补抓时渲染最终证明；
6. 成功时返回升级结果、候选诊断和软依赖 warnings。

未知依赖预先注册为空版本列表，因此正常表现为可解释的 `NoVersions`，而不是 provider
缓存异常。补抓读取已有 lockfile 的原始 provider 与 project ID；Modrinth 和
CurseForge 使用同一重试/注册路径，不跨平台猜别名。

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

内部根包、capability、provider-choice 和 Jar-in-Jar 前缀会被隐藏。测试断言
结构化 reason 或领域文本，不断言 PubGrub 内部编号和 debug 输出。

## 9. 产品边界

- CurseForge：需要用户 API Key；API 没有可用下载 URL 时返回可恢复的明确错误，不
  猜测 CDN 地址。
- 远端 fork：功能分支已发布；Orbit 通过完整 commit SHA
  `c3c4326a7e7ced4077e831285c4408c60c52ea32` 固定依赖，不跟随可移动分支头。
- 静态字节码判断：只给出必要条件，不宣称能完整证明模组运行时兼容。
