# Orbit 项目开发规范

> 由三轮 Code Review 总结。**每次修改代码前必须遵守。**

---

## 架构铁律

1. **CLI 不包含任何业务逻辑**。`orbit-cli` 只做：clap 参数解析 + 调 core API + 格式化输出。TOML 解析、文件 I/O、依赖图操作全部在 `orbit-core`。
2. **core 层不输出到 stderr/stdout**。调试用 `tracing`，用户可见进度通过返回值传递给 CLI 层展示。
3. **依赖方向**：`cli → core → wrapper`。wrapper 之间不互相依赖。core 不依赖 cli。
4. **lockfile 是已安装依赖图查询的唯一数据源**。`find_entry`、`dependents`、`check_version_conflict` 集中在 `resolver/`；候选求解图则明确组合 manifest 根约束、lockfile 当前图和候选 JAR 元数据。CLI 不手工重建依赖图。
5. **主键使用 JAR `mod_id`，slug 只作平台查找别名**。manifest dependency key 和 `PackageEntry.mod_id` 是图中的键；`package.modrinth.slug` 仅供 `find_entry` 等用户输入匹配。human-readable name 不可靠。

---

## 编码规范

6. **`todo!()` 和不可达的空壳函数禁止进入业务 crate**。暂不支持的产品边界必须返回包含恢复建议的显式 `OrbitError`（例如 CurseForge），不能伪造成功。
7. **CLI handler 必须接入 core 的真实入口**。参数错误交给 clap；业务失败返回错误，不能以 `println! + Ok(())` 掩盖。
8. **写入 manifest/lockfile 时传 `mods_dir` 作为参数**——禁止硬编码 `Path::new("mods")`。
9. **`apply_to_lockfile` 使用每个 `InstalledMod.provider` 的真实来源**——禁止硬编码 `"modrinth"`；传递依赖只写 lockfile，不得自动提升为 manifest 顶级声明。
10. **每模组独立记录 provider 来源**——`InstalledMod.provider` 字段，不能假设所有 deps 来自同一平台。

---

## API 调用规范

11. **先调 API 确认返回值再编码**。不确定字段是否存在/什么格式时，用 curl 调一下看实际响应。
12. **优先使用批量 API**。`get_versions_from_hashes`、`get_projects` 等批量端点将 N 次请求压缩为 1 次。逐个调 `get_version_by_hash` 是 N+1 反模式。
13. **404 转 `ModNotFound`**。`map_api_error()` 统一处理，CLI 收到后触发搜索回退。
14. **错误响应保留 body**。`error_for_status()` 会丢弃 body。先读 body 再检查状态码。

---

## Provider 规范

15. **用 `create_providers()` 工厂**，不直接 `ModrinthProvider::new("orbit", 3)`。
16. **`RateLimiter::acquire()` 返回 `Result`**，调用方加 `?`。内部方法（如 `lookup_project_slugs`）不获取 permit。
17. **`ResolvedMod.sha512` 存的是 SHA-512**（Modrinth 原生哈希），不是 SHA-256。下载校验用 sha512_digest。
18. **下载完必须 SHA-512 校验**，已存在的 JAR 也要比对。

---

## 交互设计

19. **`--dry-run` 在下载前拦截**，不能在下载+写盘后才跳过 toml/lock 写入。
20. **`--yes` 跳过所有交互式选择**：搜索回退选第一个结果，remove 候选列表不弹提示。
21. **找不到 slug 时搜索并交互式选择**，不能直接报错退出。

---

## 版本号

22. **`||` 是 OR 分隔符**。`satisfies()` 先按 `||` 拆组，组内空格 AND。不能按空格拆分后把 `||` 当版本号解析。
23. **版本约束必须生效**。`resolve()` 中用 `SemanticVersion::parse` + `satisfies()` 过滤。
24. **`get_versions()` 必须传 loaders/game_versions 过滤参数**。

---

## 数据结构设计

25. **Provider 专属字段进子 struct**。公共类型（`ResolvedMod`、`PackageEntry`）不扁平存放平台专属字段。Modrinth 的 `project_id`/`version_id`/`version_number` 放在 `modrinth: Option<ModrinthInfo>` 子 struct 中。未来加 CurseForge 时加 `curseforge: Option<CurseForgeInfo>`，不影响现有字段。
26. **key 统一用 JAR loader 元数据声明的 ID**（即 `mod_id`）。slug 只在 `find_entry` 中作为备选匹配键，不用作主键。

## JAR 模块

27. **所有 JAR 元数据读取走 `jar` 模块**。`init.rs`、`installer.rs` 不直接打开 ZIP、不直接调用具体 parser。调用 `jar::read_mod_metadata(path, loader)`，由 jar 模块按 Fabric、Forge、NeoForge、Quilt 分发到对应 reader。
28. **`loader` 参数由调用者传入，禁止 auto-detect**。一个 JAR 可能同时兼容多个 loader（同时含 fabric.mod.json 和 META-INF/mods.toml），auto-detect 会选错。

## 文件 I/O

29. **manifest/lockfile 文件读写统一走 `ManifestFile` / `Lockfile` 封装**。其他模块不直接调 `std::fs::write` 操作 orbit.toml / orbit.lock。初始化用 `ManifestFile::new(dir, manifest)` + `save()`，运行时用 `ManifestFile::open(dir)` / `Lockfile::open(dir)`。
30. **`Lockfile::open_or_default(dir, meta)` 处理锁文件不存在**。不需要每个调用方手写 `if path.exists() { from_path } else { default }`。

## Resolver

31. **联网候选求解统一通过 `graph::build_solver_graph()` 构图**。平台包、lockfile、候选、root 和未知引用包的注册不得重新散落到求解循环。动态补抓候选必须复用 `graph::register_candidate_versions()`。
32. **lockfile 条目必须携带真实依赖进入候选求解图**。否则升级求解无法解释当前版本和候选版本之间的传递依赖冲突。`check_local_graph()` 是另一条纯本地输入路径，共享平台包注册规则，但不伪造或注入 lockfile。
33. **成功求解中的候选淘汰原因只来自同一次求解的类型化 `SolverEvent`**。禁止解析 debug 日志，禁止用第二次反事实求解替代实际的传播、decision 和 backtrack 路径。

## 代码卫生

34. **写完功能立即检查是否有死代码**：未用的函数、struct、trait、import、依赖项全部删除。
35. **字段命名必须准确**：存 SHA-512 就叫 `sha512`，不叫 `sha256`。
36. **`expect()` / `unwrap()` 只在不可能失败时使用**。library crate 优先返回 `Result`。
37. **修复一个问题时检查所有同类问题**（如一个 stub 改 exit(2) 就要全部改）。

## core 层输出

38. **`orbit-core/src` 当前不包含 `println!` / `eprintln!` / `eprint!`**。继续保持：用户可见结果和依赖诊断通过结构化返回值传递给 CLI；纯调试信息使用 `tracing`。

---

## 文档同步

39. **代码改动后同步更新 docs/**：`orbit-resolver.md`、`orbit-status.md`、`orbit-architecture.md`。
40. **modrinth-docs 是 API wrapper 的规格来源**，模型字段变更时同步更新。
