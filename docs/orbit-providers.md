# Orbit Provider 层

> 实现位置：`orbit-core/src/providers/`

## 1. 当前支持状态

| Provider | 状态 | 说明 |
|----------|:---:|------|
| Modrinth | ✅ | 搜索、详情、版本、依赖、SHA-512 批量识别 |
| CurseForge | ✅ | 搜索、详情、版本、依赖、认证下载、文件指纹批量识别；API Key 必填 |
| 本地 `file:` | ✅ | 由 `installer/local.rs` 处理，不是网络 provider |

默认平台仍是 `["modrinth"]`，因为 CurseForge Core API 要求用户自己的 API Key。
配置 Key 后，可将 `curseforge` 单独使用或放入 `[resolver].platforms` 的回退顺序。

## 2. 模块边界

```text
providers/
├── mod.rs                 trait、统一类型、provider factory
├── download.rs            统一 artifact 下载、域名限定认证与重定向校验
├── modrinth.rs            Modrinth SDK → 领域类型
├── rate_limiter.rs        单 provider 的 semaphore
└── curseforge/
    ├── client.rs          HTTP、认证、分页、状态错误
    ├── models.rs          REST JSON 与官方枚举
    └── mod.rs             CurseForge → 领域类型
```

平台实现只负责查询和归一化。CLI、安装器和 resolver 不直接调用平台 SDK；所有来源下载
后都进入同一个 JAR reader、候选图、PubGrub 求解、lockfile 和恢复路径。

## 3. 统一接口

```rust
#[async_trait]
pub trait ModProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn artifact_downloader(&self) -> &ArtifactDownloadClient;
    async fn search(...) -> Result<Vec<SearchResultItem>, OrbitError>;
    async fn get_mod_info(...) -> Result<ModInfo, OrbitError>;
    async fn identify_artifacts(
        &self,
        artifacts: &[ArtifactFingerprint],
    ) -> Result<Vec<ResolvedMod>, OrbitError>;
    async fn get_versions(...) -> Result<Vec<ResolvedMod>, OrbitError>;
    async fn get_versions_batch(...) -> Result<Vec<ResolvedMod>, OrbitError>;
    async fn get_categories(...) -> Result<Vec<String>, OrbitError>;
    async fn fetch_dependencies(...) -> Result<Vec<ResolvedDependency>, OrbitError>;
}
```

`ArtifactFingerprint` 同时携带 SHA-1、SHA-512 和 CurseForge fingerprint。平台自己选择
官方查询所需摘要：公共编排不再把所有 provider 强行塞进
`get_version_by_hash(sha512)`。

## 4. 统一类型与来源事实

`ResolvedMod` 的公共字段包括查询别名、来源名、文件名、下载 URL、发布日期、平台依赖
提示和可用哈希；专属字段分别位于：

- `modrinth: Option<ModrinthResolvedInfo>`；
- `curseforge: Option<CurseForgeResolvedInfo>`。

lockfile 同样使用 `[package.modrinth]` 和 `[package.curseforge]` 子表。公共层通过
`source_slug()`、`source_project_id()`、`source_version_id()` 等方法读取，不按平台
复制 install/restore/outdated/check/retry 逻辑。

平台的版本展示名不能代替 JAR 版本。Orbit 下载候选后读取 loader 元数据，用真实
`mod_id`、版本、`DependencyExpression` 与递归 `bundled` 构建求解图。平台依赖只用于
发现需要下载的项目；最终证明来自同一物理 JAR。

## 5. Provider 选择与认证

`create_providers()` 保持 `[resolver].platforms` 的顺序。候选发现选择第一个返回有效
候选的来源；已锁定包的恢复、补抓、检查和升级按 lockfile 记录的原始 provider，不跨
平台猜同名项目。

`orbit init` 尚无 manifest 可提供来源列表，因此使用专门的 identification factory：
Modrinth 始终参与，只有已配置 API Key 时才加入 CurseForge。它不会因为用户没有 Key
而破坏默认初始化，也不会跳过一个已显式配置但认证失败的 provider 错误。

- `mr:` → 只用 Modrinth；
- `cf:` → 只用 CurseForge；
- `file:` → 本地安装，不进入 provider factory。

CurseForge 使用 `x-api-key`，配置项为 `auth.curseforge_api_key`，环境变量为
`ORBIT_CURSEFORGE_API_KEY`。CurseForge provider 在 factory 和直接构造入口都会拒绝
缺失、空白 Key；不会提供匿名网页抓取或无 Key 降级模式。实例 manifest 或 lockfile
只要要求创建 CurseForge provider，缺 Key 就会在任何查询、恢复或检查开始前失败。
默认的 Modrinth 工作流不受影响。

## 6. CurseForge API 映射

实现以 [CurseForge Core REST API](https://docs.curseforge.com/rest-api/) 为规格：

- base URL 为 `https://api.curseforge.com/v1/`；
- 分页每页最多 50 条，且 `index + pageSize <= 10000`；
- 通过 `/games` 和 `/categories` 查找 Minecraft game ID 与 Mods class ID，不硬编码
  社区常见数字；
- 搜索使用 `/mods/search`，slug 与 class ID 联合定位项目；
- 文件使用 `/mods/{modId}/files`，loader 枚举为 Forge=1、Fabric=4、Quilt=5、
  NeoForge=6；
- 文件没有内联 URL 时调用
  `/mods/{modId}/files/{fileId}/download-url`；不可用响应不会被替换成猜测的 CDN 地址；
- `relationType=3` 作为 required，`relationType=2` 作为 optional 候选发现提示；
- 文件 SHA-1 用于下载校验，下载后再计算 SHA-256/SHA-512 写入 lockfile 和缓存。

API 没有 license 和 client/server side 字段，因此 CurseForge `info` 对这三项显示
unknown；不会从分类或文件名猜测。

## 7. CurseForge 文件指纹

官方 REST 文档定义了 `/fingerprints/{gameId}` 和 `fileFingerprint`，但没有写出本地
计算算法。Orbit 没有凭印象实现：代码采用
[Prism Launcher 的公开实现](https://github.com/PrismLauncher/PrismLauncher/blob/develop/libraries/murmur2/src/MurmurHash2.cpp)
及其
[CurseForge 特定的空白过滤调用](https://github.com/PrismLauncher/PrismLauncher/blob/develop/launcher/modplatform/helpers/HashUtils.cpp)
作为可审计来源。

算法先移除字节 `9`、`10`、`13`、`32`，再以 seed 1 计算 32 位 MurmurHash2。单元测试
包含 golden vectors；provider 合同测试验证指纹批量请求与项目映射。`init` / `sync`
因此可以识别手动放入的 CurseForge JAR。识别只接受平台的精确 hash/fingerprint
匹配；批量接口失败会保留 provider 名称并报错，不会按文件名或展示版本猜来源。

## 8. 下载限制与错误

CurseForge 项目可以没有第三方可用的下载 URL。Orbit 只使用 API 返回的内联 URL 或
download-url 端点；若目标版本没有任何 API 可下载且带 SHA-1 的文件，会明确报告
`matching files, but none is API-downloadable with a SHA-1 checksum`。完全没有匹配文件
仍返回空候选，使 provider 回退和 `orbit check` 的“不兼容”结果保持正常。Orbit 不会
拼接 CDN URL，也不会把 HTML 错误页当 JAR。

根据 CurseForge 的
[文件下载认证公告](https://blog.curseforge.com/introducing-api-key-authentication-for-curseforge-file-downloads/)，
直接 CDN 下载从 2026-07-16 起也需要 `x-api-key`。所有 provider 共用
`ArtifactDownloadClient` 下载路径；CurseForge 只在运行时为 HTTPS
`forgecdn.net` 及其子域添加 Key，每一跳重定向都重新校验。Key 不进入
`ResolvedMod`、`InstalledMod` 或 lockfile，也不会被发送给任意 API 返回 URL。

API 状态错误保留 HTTP 状态和最多 500 字符响应正文；API Key 不进入日志或错误文本。
候选批量验证允许忽略个别坏的历史文件，但当所有候选都失败时返回第一个具体下载、
校验或 JAR 解析错误，不降级成不可读的“未找到”。

## 9. 测试边界

仓库测试通过本地 mock HTTP server 验证：

- `x-api-key` 与官方 query 参数；
- 下载 Key 的 HTTPS/域名范围，以及匿名下载不携带 Key；
- game/class 动态发现；
- loader/game version 搜索；
- download-url 回退；
- fingerprint 批量识别；
- `[package.curseforge]` roundtrip；
- provider 顺序和缺 Key 错误。

测试不依赖开发者私人 Key。需要上线前 smoke test 时，设置
`ORBIT_CURSEFORGE_API_KEY` 后执行实际 `search`、`info` 与 `add cf:<slug>`；最后一项
同时覆盖真实 CDN 认证。
