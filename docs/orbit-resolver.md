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
├── types.rs               # PackageId + CandidateVersion + ImplantedCandidate
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

候选 JAR 的初次发现和批量下载目前发生在 `outdated::download_candidates_bfs()`；resolver 本身只会在求解失败后补抓候选所引用、且 lockfile 已知的缺失依赖。因此“离线求解”只描述单次 PubGrub 运行，不代表整个 API 绝不访问网络。

---

## 3. 求解图构建

`graph.rs` 按以下顺序构建 `OrbitDependencyProvider`：

1. 注册平台内置包：`minecraft`、实际 loader、Fabric loader 别名、`java`，以及 Fabric 的 `mixinextras`；
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
- 不在 manifest 中的候选也以 `Ranges::full()` 加入根依赖，供 `orbit add` 使用。

`orbit add` 会先把 provider 查询标识映射到候选 JAR 自声明的 `mod_id`，再把命令行
constraint 临时加入求解 manifest。安装成功后，该 constraint 写入真实 `mod_id` 对应的
manifest 条目；后续升级只更新 lockfile 版本，不覆盖原约束。

---

## 4. 求解与缺失依赖补抓

`retry.rs` 每次尝试都会：

1. 为每个包的首个候选建立 `ResolutionTrace`；
2. 调用 `pubgrub::resolve_with_observer()`；
3. 成功时返回解和本次实际求解路径的 trace；
4. `NoSolution` 时检查候选及其内嵌模组声明的 required dependencies；
5. 对尚无候选、不是内嵌模组、且能在 lockfile 找到 Modrinth 元数据的包，获取版本、下载 JAR、解析元数据并注册；
6. 有新增候选则重新求解，否则用 `DefaultStringReporter` 返回原始不可解证明。

补抓得到的候选走 `graph::register_candidate_versions()`，与初始候选共享同一套版本去重、依赖注册和未知传递依赖注册逻辑。

这不是旧文档描述的“PubGrub 返回 `FetchRetryError` 后按缓存缺口抓取”。`ProviderError` 只表示图构造漏掉了包版本或版本依赖，是内部错误；正常的未知依赖已注册为空版本列表，并表现为 `NoSolution`。

目前补抓只使用 `providers.first()`。这与 `[resolver].platforms` 的顺序回退规范尚不一致。

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

当前诊断只覆盖“求解成功但首个候选未被选择”的场景。真正不可解时仍返回 `DefaultStringReporter` 的字符串，见第 8 节。

---

## 6. 本地图校验

```rust
check_local_graph(manifest, local_mods)
```

该路径不访问 provider：

1. 注册 Minecraft、loader 等平台包；
2. 使用 JAR 自声明的 `mod_id` 和版本注册本地模组；
3. 注入本地 JAR 的 required dependencies；
4. 将被依赖但未安装的包注册为空版本列表；
5. 根包精确依赖已安装的 manifest 包，并以 full range 依赖缺失的 manifest 包；
6. 调用普通 `pubgrub::resolve()`。

缺失的 manifest 顶层依赖必须进入根约束，否则空版本列表不会被求解器访问。该行为有单元测试保护。

当前 `init` 使用此函数验证扫描结果；`check` 和 `sync` 命令仍未实现，不能写成已经接入。

---

## 7. 公共 API

| 函数 | 当前行为 |
|------|----------|
| `find_entry(input, entries)` | 匹配 `mod_id`，或备选匹配 `package.modrinth.slug` |
| `dependents(mod_id, entries)` | 从 lockfile 真实依赖中反查直接依赖者 |
| `check_version_conflict(mod_id, version, entries)` | 检查 lockfile 已有版本是否冲突 |
| `resolve_with_candidates(...)` | 构图、求解、必要时补抓依赖，并返回实际升级版本 |
| `check_local_graph(...)` | 不联网验证本地安装图 |

不存在 `resolve_manifest()`、`ProviderVersionResolver`、`ModrinthVersionResolver` 或 `trapped_room_test()`。

---

## 8. 仍然有效但代码尚未满足的规范

以下条目不是过时文档，不能为了匹配现状而删除：

| 规范 | 当前代码差距 |
|------|--------------|
| `[resolver].platforms` 应按顺序回退 | add、outdated、BFS 下载和 resolver 补抓目前只使用第一个 provider |
| `orbit-core` 不直接输出 UI/进度文本 | `resolver/mod.rs` 和 `retry.rs` 仍有 `eprintln!`；诊断应通过结构化返回值交给 CLI |
| 冲突报告应面向用户且可读 | 成功但跳过候选已有领域化解释；真正 `NoSolution` 和本地校验仍直接返回 `DefaultStringReporter` 字符串 |
| `[overrides]`、`optional`、`env`、`exclude` 应影响解析或安装 | manifest 已能反序列化这些字段，但当前候选求解路径未应用它们 |
| Java 依赖必须有明确语义 | 联网候选图把 `java` 注册为 `0.0.0`，本地校验却忽略 Java 依赖；两条路径尚未统一为“检测运行时版本”或“明确忽略” |

---

## 9. 相关文档

- [orbit-versions.md](orbit-versions.md)：版本解析和约束语义
- [orbit-toml-spec.md](orbit-toml-spec.md)：manifest/lockfile 的规范行为
- [orbit-architecture.md](orbit-architecture.md)：crate 与模块边界
- [orbit-status.md](orbit-status.md)：实现进度与已知偏差
