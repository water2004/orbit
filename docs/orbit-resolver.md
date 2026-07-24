# Orbit 依赖解析引擎

> 本文档同时记录 `orbit-core/src/resolver/` 的当前实现和仍然有效的设计约束。
> “当前实现”用于解释代码；“规范未满足”不能通过修改文档来合理化，必须在代码中修复。

---

## 1. 模块结构

```text
orbit-core/src/resolver/
├── mod.rs                 # 公共 API 与顶层编排
├── graph.rs               # 构建 PubGrub 输入图、注册候选与内嵌模组
├── retry.rs               # 求解循环与缺失依赖补抓
├── local.rs               # 不联网的本地安装图校验
├── provider.rs            # OrbitDependencyProvider + ProviderError
├── types.rs               # 候选输入 + 结构化 ResolutionReport / CandidateDiagnostic
└── diagnostics/
    ├── mod.rs             # 从类型化 SolverEvent 采集候选淘汰原因
    ├── render.rs          # 将 DerivationTree 渲染为 Orbit 文案
    └── tests.rs           # 三种实际求解路径的契约测试
```

`mod.rs` 不构造依赖图，也不实现重试细节。它只暴露查询 API、调用各阶段并将最终解转换成升级结果。

Orbit 当前使用仓库根目录下的 `pubgrub-fork`，其基础版本是 PubGrub `0.4.0`。该 fork 增加了不改变默认 `resolve()` 行为的 `resolve_with_observer()`、`SolverObserver` 和类型化 `SolverEvent`。

---

## 2. `resolve_with_candidates`

```rust
resolve_with_candidates(manifest, lockfile, candidates, providers)
```

输入包括：

- `manifest`：项目、加载器和顶层依赖声明；
- `lockfile`：当前已安装版本、真实 JAR 依赖和内嵌模组；
- `candidates`：从候选 JAR 解析出的真实 `mod_id`、版本、依赖和内嵌模组；
- `providers`：缺失候选需要联网补抓时使用的平台 provider。

处理阶段如下：

```text
build_solver_graph
  → solve_with_fetch_retry
    → collect_upgrades
```

候选 JAR 的初次发现和批量下载目前发生在
`outdated::download_candidates_with_fallback()`（内部复用
`download_candidates_bfs()`）；resolver 本身只会在求解失败后补抓候选所引用、且
lockfile 已知的缺失依赖。因此“离线求解”只描述单次 PubGrub 运行，不代表整个 API
绝不访问网络。

---

## 3. 求解图构建

`graph.rs` 按以下顺序构建 `OrbitDependencyProvider`：

1. 注册平台内置包：`minecraft`、实际 loader 和 Fabric loader 别名；
2. 注册 lockfile 中的包、真实依赖和内嵌模组；
3. 注册候选版本和候选内嵌模组；
4. 构造内部根包 `___orbit_root___`；
5. 将所有被引用但尚无已知版本的包注册为空版本列表。

最后一步很重要：未知依赖应成为 PubGrub 的正常 `NoVersions` 推导，而不是 `DependencyProvider` 的缓存错误。

同一个包的版本顺序为：

```text
候选版本（调用方给定顺序，去重） → lockfile 版本
```

`OrbitDependencyProvider::choose_version()` 选择该顺序中第一个满足当前范围的版本。

当前根约束行为：

- manifest 中的包始终使用 manifest 版本约束，无论该包是否有候选；
- 不在 manifest 中的候选只有被已选包依赖时才进入解，不会被提升为顶级依赖；
- `[overrides]` 替换已有根边或传递边的版本范围，但不会创建新的依赖边；
- `exclude` 只移除声明该规则的包到指定传递依赖的边；其他包或 manifest
  显式声明仍可把该依赖带入解。

`orbit add` 会先把 provider 查询标识映射到候选 JAR 自声明的 `mod_id`，再把命令行
constraint 临时加入求解 manifest。安装成功后，该 constraint 写入真实 `mod_id` 对应的
manifest 条目；`--optional` 和 `--env` 同时保存在完整依赖形式中。传递依赖只进入
lockfile，不会被自动提升成 manifest 顶级声明；后续升级只更新 lockfile 版本，不覆盖原约束。

`java` 和 `mixinextras` 当前被明确视为运行时提供的依赖，并在联网候选图、本地图和
缺失候选发现中一致忽略。Orbit 尚未探测实际 Java 运行时版本，因此不会伪造 `0.0.0`
参与版本比较。

---

## 4. 求解与缺失依赖补抓

`retry.rs` 每次尝试都会：

1. 为每个包的首个候选建立 `ResolutionTrace`；
2. 调用 `pubgrub::resolve_with_observer()`；
3. 成功时返回解和本次实际求解路径的 trace；
4. `NoSolution` 时检查候选及其内嵌模组声明的 required dependencies；
5. 对尚无候选、不是内嵌模组、且能在 lockfile 找到 Modrinth 元数据的包，通过名称选择
   Modrinth provider，获取版本、下载 JAR、解析元数据并注册；
6. 有新增候选则重新求解，否则把不可解证明渲染为领域依赖事实。

补抓得到的候选走 `graph::register_candidate_versions()`，与初始候选共享同一套版本去重、依赖注册和未知传递依赖注册逻辑。

这不是旧文档描述的“PubGrub 返回 `FetchRetryError` 后按缓存缺口抓取”。`ProviderError` 只表示图构造漏掉了包版本或版本依赖，是内部错误；正常的未知依赖已注册为空版本列表，并表现为 `NoSolution`。

初始候选发现通过 `download_candidates_with_fallback()` 按
`[resolver].platforms` 顺序选择第一个有有效候选的平台。补抓已有 lockfile 条目时不做
跨平台猜测，而是按条目的来源元数据选择对应 provider；当前 lockfile 只实现了
Modrinth 专属元数据。

---

## 5. 成功求解中的候选淘汰原因

不可解时的 `DerivationTree` 证明“没有解”。它不能回答“这次成功求解为什么没有选择某个候选版本”，因为后者与求解器实际走过的路径有关。

Orbit 不再运行第二次反事实求解，也不解析 debug 日志。定制 PubGrub 在同一次求解中发送类型化事件：

| 事件 | Orbit 使用方式 |
|------|----------------|
| `PackageChoice` | 确认被观察候选在本轮选择前仍被允许 |
| `VersionChoice` | 区分 provider 选择顺序导致的版本偏好 |
| `Decision` | 记录候选真正进入 partial solution 的 decision level |
| `Derivation` | 找到候选由允许变为排除的精确传播步骤及原因树 |
| `Backtrack` | 记录已提交候选因冲突被回退及其学习到的原因树 |

最终原因分为：

- `ExcludedByPropagation`：候选在成为 decision 前被依赖传播排除；
- `Backtracked`：候选被提交后因冲突回溯；
- `ProviderPreferred`：候选仍允许，但 provider 顺序选择了另一个版本。

渲染器从事件携带的 `DerivationTree` 提取外部依赖事实，隐藏内部根包名称，并限制输出事实数量。

`diagnostics/tests.rs` 使用最小、确定性的依赖图分别触发上述三条路径。测试断言的是类型化事件生成的领域解释，不解析日志，也不运行第二条证明路径。

成功求解返回 `ResolutionReport`，其中 `diagnostics` 是类型化
`CandidateDiagnostic`；CLI 决定如何展示。不可解和本地校验也共用领域事实渲染器，
不再暴露 PubGrub 默认 reporter 的内部证明格式。

---

## 6. 本地图校验

```rust
check_local_graph(manifest, local_mods)
```

该路径不访问 provider：

1. 注册 Minecraft、loader 等平台包；
2. 使用 JAR 自声明的 `mod_id` 和版本注册本地模组；
3. 使用与候选图相同的 override、exclude 和运行时依赖规则注入 required dependencies；
4. 将被依赖但未安装的包注册为空版本列表；
5. 根包按 manifest 的实际版本约束依赖所有顶级包；
6. 调用普通 `pubgrub::resolve()`。

缺失的 manifest 顶层依赖和不满足 manifest 约束的本地版本都必须产生不可解结果。
override/exclude 在本地与联网路径一致的行为也有单元测试保护。

当前 `init` 使用此函数验证扫描结果；`check` 和 `sync` 命令仍未实现，不能写成已经接入。

---

## 7. 公共 API

| 函数 | 当前行为 |
|------|----------|
| `find_entry(input, entries)` | 匹配 `mod_id`，或备选匹配 `package.modrinth.slug` |
| `dependents(mod_id, entries)` | 从 lockfile 真实依赖中反查直接依赖者 |
| `check_version_conflict(mod_id, version, entries)` | 检查 lockfile 已有版本是否冲突 |
| `resolve_with_candidates(...)` | 构图、求解、必要时补抓依赖，并返回实际升级版本 |
| `resolve_with_candidates_report(...)` | 在升级结果之外返回类型化候选诊断 |
| `check_local_graph(...)` | 不联网验证本地安装图 |

不存在 `resolve_manifest()`、`ProviderVersionResolver`、`ModrinthVersionResolver` 或 `trapped_room_test()`。

---

## 8. 已收口的规范与剩余边界

| 规范 | 当前状态 |
|------|----------|
| `[resolver].platforms` 按顺序回退 | add 的候选发现和搜索已按配置顺序工作；补抓按 lockfile 来源选择 provider |
| `orbit-core` 不输出 UI 文本 | core 返回报告和错误，stdout/stderr 只由 CLI 使用 |
| 冲突信息可读且可测试 | 成功路径返回类型化候选原因；不可解路径渲染领域依赖事实 |
| `[overrides]` / `exclude` | 候选图和本地图共用规则；override 不新增依赖，exclude 按声明者移除边 |
| `optional` / `env` | `orbit add` 已持久化字段；它们按 Fat Lockfile 设计不改变求解图 |
| Java 依赖 | 联网和本地路径均明确忽略，不再注入虚假的 `0.0.0` |

仍未完成但不能从规范中删除的边界：

- CurseForge provider 和对应 lockfile 来源元数据尚未实现，因此多平台回退框架目前只有
  Modrinth 可实际使用；
- `orbit install` 全量还原尚未实现，所以 `--target`、`--no-optional` 和 groups 对实际
  文件安装的过滤仍未落地；
- 实际 Java 运行时探测属于后续能力；当前策略是明确且一致地忽略元数据中的 Java 约束。

---

## 9. 相关文档

- [orbit-versions.md](orbit-versions.md)：版本解析和约束语义
- [orbit-toml-spec.md](orbit-toml-spec.md)：manifest/lockfile 的规范行为
- [orbit-architecture.md](orbit-architecture.md)：crate 与模块边界
- [orbit-status.md](orbit-status.md)：实现进度与已知偏差
