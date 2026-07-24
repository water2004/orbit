# Orbit Provider 层

> 实现位置：`orbit-core/src/providers/`

## 1. 当前支持状态

| Provider | 状态 | 说明 |
|----------|:---:|------|
| Modrinth | ✅ | 搜索、详情、解析、哈希反查、版本批量查询、依赖查询 |
| 本地 `file:` | ✅ | 由 `installer/local.rs` 处理，不是网络 provider |
| CurseForge | ⏸ | 明确暂不支持；不会注册空 provider |

Provider 抽象支持按 `[resolver].platforms` 顺序回退，但“抽象支持多个平台”不等于已有
多个可用实现。当前默认值仅为 `["modrinth"]`；显式配置 `curseforge` 或未知名称会在
创建 provider 时返回可读错误。

## 2. 模块边界

```text
providers/
├── mod.rs             trait、统一类型、provider factory
├── modrinth.rs        Modrinth SDK 到领域类型的适配
├── rate_limiter.rs    单 provider 的 semaphore
└── curseforge.rs      统一“暂不支持”错误边界
```

`modrinth-wrapper` 只封装 HTTP 与平台 JSON。`ModrinthProvider` 负责：

- 将 Orbit 的 Minecraft/loader/版本约束映射为 API 查询；
- 选择主 JAR 文件；
- 将 project/version/file/dependency 响应归一化为领域类型；
- 从下载后的 JAR 获取真实 `mod_id`、版本和依赖；
- 为批量 hash/project 查询使用平台批量端点。

CLI 和 resolver 不直接调用 wrapper。

## 3. 统一接口

`ModProvider` 当前提供：

```rust
#[async_trait]
pub trait ModProvider: Send + Sync {
    fn name(&self) -> &'static str;
    async fn search(...) -> Result<Vec<SearchResultItem>, OrbitError>;
    async fn get_mod_info(...) -> Result<ModInfo, OrbitError>;
    async fn resolve(...) -> Result<ResolvedMod, OrbitError>;
    async fn get_version_by_hash(...) -> Result<Option<ResolvedMod>, OrbitError>;
    async fn get_versions_by_hashes(...) -> Result<Vec<ResolvedMod>, OrbitError>;
    async fn get_versions(...) -> Result<Vec<ResolvedMod>, OrbitError>;
    async fn get_versions_batch(...) -> Result<Vec<ResolvedMod>, OrbitError>;
    async fn get_categories(...) -> Result<Vec<String>, OrbitError>;
    async fn fetch_dependencies(...) -> Result<Vec<ResolvedDependency>, OrbitError>;
}
```

批量方法有逐项默认实现，支持的平台应覆盖为真正的批量 API，避免 N+1 请求。

## 4. 统一类型与来源事实

`ResolvedMod` 的公共字段包括：

| 字段 | 含义 |
|------|------|
| `mod_id` / `version` | 下载 JAR 自声明的包 ID 与版本 |
| `slug` / `provider` | 用户查询标识与来源名称 |
| `sha1` / `sha512` | 平台提供或已验证的文件哈希 |
| `download_url` / `filename` | 选定主文件 |
| `date_published` | 候选排序信息 |
| `dependencies` | 平台可提供的依赖提示 |
| `client_side` / `server_side` | 平台端侧元数据 |
| `modrinth` | Modrinth 专属 project/version 信息 |

平台结果只是候选来源。下载后必须读取 JAR 元数据，以真实 `mod_id`、版本、required
dependencies 和 implanted mods 构建求解图。平台的 `version_number` 可能是展示字符串，
不能代替 JAR 版本。

同样，lockfile 公共字段不扁平存储 `project_id` 等平台字段，而是使用
`[package.modrinth]` 子表。未来新增 provider 时应添加自己的子结构。

## 5. 并发限制

每个网络 provider 自己持有 `RateLimiter`。当前 Modrinth factory 使用并发数 3：

```text
request
  → acquire owned semaphore permit
  → call SDK
  → permit drops
```

这样 limiter 生命周期覆盖完整请求，不依赖调用方记得释放。批量 API 只占用一个 permit。
候选下载的任务并发与 provider API 限流是两层不同控制：前者控制文件验证任务，后者控制
平台请求。

全局配置存在 `max_concurrent_downloads`，但尚未接到全部下载编排；这是有效配置规范与
实现之间的剩余差距。

## 6. Provider 选择

`create_providers(platforms)` 保持传入顺序：

```text
["modrinth"] → [ModrinthProvider]
["curseforge"] → error
["unknown"] → error
[] → error
```

候选发现依次询问 provider，选择第一个返回有效候选的来源。resolver 后续补抓已锁定
依赖时按 lockfile 的来源字段选择 provider，不跨平台猜测同名项目。

显式 CLI 前缀的归一化：

- `mr:` → `modrinth`
- `cf:` → `curseforge`，随后返回暂不支持
- `file:` → 本地安装路径，不进入 provider factory

## 7. 哈希与批量识别

`sync` / `init` 对实际 JAR 计算哈希，并优先调用批量反查。Modrinth 使用 SHA-512。
平台识别结果写入来源子表，JAR 自身字段仍由本地解析与哈希结果决定。

不同平台的哈希算法不可由公共层硬编码成同一种。真正接入新 provider 时，必须同时定义
该平台的哈希生成与 API 参数，不能只实现搜索。

## 8. CurseForge 的接入门槛

CurseForge 继续保持暂不支持。完整接入至少需要：

1. 可测试的 SDK/HTTP 客户端与认证；
2. 搜索、详情、版本、主文件和依赖映射；
3. CurseForge 指定的文件哈希算法与批量识别策略；
4. loader/game version 过滤和 release channel 规则；
5. `ResolvedMod` 与 lockfile 的 CurseForge 专属子结构；
6. restore、sync、outdated、upgrade、check 的端到端测试；
7. 最后才修改 factory、默认平台和文档。

在这些边界完成前，`CurseForgeProvider` 的方法统一返回
`CurseForge support is not yet implemented`。这是一条显式产品边界，不是可吞掉的
fallback。
