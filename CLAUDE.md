# Orbit 项目开发规范

> 由三轮 Code Review 总结。**每次修改代码前必须遵守。**

---

## 架构铁律

1. **CLI 不包含任何业务逻辑**。`orbit-cli` 只做：clap 参数解析 + 调 core API + 格式化输出。TOML 解析、文件 I/O、依赖图操作全部在 `orbit-core`。
2. **core 层不输出到 stderr/stdout**。调试用 `tracing`，用户可见进度通过返回值传递给 CLI 层展示。
3. **依赖方向**：`cli → core → wrapper`。wrapper 之间不互相依赖。core 不依赖 cli。
4. **lockfile 是依赖图的唯一数据源**。所有依赖图查询（`find_entry`、`dependents`、`check_version_conflict`）都在 `resolver.rs` 中，通过 lockfile 重建图。不允许在 CLI 或其他模块手工遍历 lockfile/manifest 做依赖判断。
5. **使用 slug 匹配，不用 name**。`DependencySpec::slug` 和 `LockEntry::mod_id`/`LockEntry::slug` 是匹配键。human-readable name 不可靠。

---

## 编码规范

6. **`todo!()` 禁止在 library crate 中使用**。所有未实现函数返回 `Err(OrbitError::Other(anyhow!("not yet implemented")))`。
7. **空壳 CLI 命令用 `eprintln! + exit(2)`**，不允许 `println! + Ok(())`。
8. **写入 manifest/lockfile 时传 `mods_dir` 作为参数**——禁止硬编码 `Path::new("mods")`。
9. **`apply_to_manifest_and_lock` 传 `provider_name` 作为参数**——禁止硬编码 `"modrinth"`。
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

## 代码卫生

25. **写完功能立即检查是否有死代码**：未用的函数、struct、trait、import、依赖项全部删除。
26. **字段命名必须准确**：存 SHA-512 就叫 `sha512`，不叫 `sha256`。
27. **`expect()` / `unwrap()` 只在不可能失败时使用**。library crate 优先返回 `Result`。
28. **修复一个问题时检查所有同类问题**（如一个 stub 改 exit(2) 就要全部改）。

---

## 文档同步

29. **代码改动后同步更新 docs/**：`orbit-resolver.md`、`orbit-status.md`、`orbit-architecture.md`。
30. **modrinth-docs 是 API wrapper 的规格来源**，模型字段变更时同步更新。
