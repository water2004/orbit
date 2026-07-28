# Orbit Provider 层

> HTTP wrapper 位于 workspace 顶层；Orbit 领域适配位于
> `orbit-core/src/providers/`。

## 1. 当前支持状态

| Provider | 状态 | 说明 |
|----------|:---:|------|
| Modrinth | ✅ | 搜索、详情、版本、依赖、SHA-512 批量识别 |
| CurseForge | ✅ | 搜索、详情、版本、依赖、认证下载、文件指纹批量识别；API Key 必填 |
| 本地 `file:` | ✅ | 不是网络 provider，但与网络来源进入同一候选目录和事务 |

默认搜索目录是 `["modrinth"]`，因为 CurseForge Core API 要求用户自己的 API Key。
配置 Key 后，可将 `curseforge` 加入 `[resolver].catalogs`。该数组只控制无限定
搜索/添加启用哪些目录，不是包候选的回退优先级。

## 2. 模块边界

```text
modrinth-wrapper/          Modrinth HTTP、请求/响应 DTO、传输错误
curseforge-wrapper/        CurseForge HTTP、认证、分页、DTO、传输错误

orbit-core/src/providers/
├── mod.rs                 trait、统一领域类型、provider factory
├── download.rs            统一 artifact 下载、域名限定认证与重定向校验
├── modrinth.rs            modrinth-wrapper → Orbit 领域类型
├── curseforge/mod.rs      curseforge-wrapper → Orbit 领域类型
└── rate_limiter.rs        单 provider 的 semaphore
```

wrapper 不依赖 core，也不知道 Orbit 的包身份、缓存或 solver。core provider adapter
只负责查询编排和领域映射；CLI、安装器和 resolver 不直接调用 wrapper。所有来源下载后
都进入同一个 JAR reader、候选图、PubGrub 求解、lockfile 和恢复路径。

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
    ) -> Result<Vec<RemoteArtifact>, OrbitError>;
    async fn get_versions(...) -> Result<Vec<RemoteArtifact>, OrbitError>;
}
```

`ArtifactFingerprint` 同时携带 SHA-1、SHA-512 和 CurseForge fingerprint。平台自己选择
官方查询所需摘要：公共编排不再把所有 provider 强行塞进
`get_version_by_hash(sha512)`。

## 4. 统一类型与来源事实

`RemoteArtifact` 只包含下载所需的来源 locator、文件名、URL 和 provider
强哈希；专属字段分别位于：

- `modrinth: Option<ModrinthResolvedInfo>`；
- `curseforge: Option<CurseForgeResolvedInfo>`。

该类型刻意没有 `mod_id`、模组版本、依赖范围、环境、provides 或 bundled。
lockfile 将候选发现入口统一保存在 `package.remotes`，将能恢复当前已选字节内容的
精确工件统一保存在 `package.artifact_sources`。公共层不按平台复制
install/restore/outdated/check 逻辑。

lockfile 的 `remotes` / `artifact_sources` 不保存或信任远端展示版本。平台 slug、
project ID 和查询结果里的版本名都不能代替 JAR 身份。Orbit 先沿
`RemoteProjectLocator` 递归枚举当前 Minecraft/loader 的完整 project/artifact 闭包，
所有 provider 的发现阶段都结束后，再用一个有界批次统一查 cache 或下载。之后才读取
loader 元数据，用真实 `mod_id`、版本、
`DependencyExpression` 与递归 `bundled` 构建求解图。

远端 dependency relation 只定位下一层 project，不向求解器贡献 required/optional 或
版本语义。反过来，JAR `mod_id` 绝不作为 slug/project 查询；闭包缺少 JAR required
identity 时，纯离线 solver 将其证明为无解。

候选主键是 Orbit 对下载内容自行计算的 SHA-512，而不是 provider、文件名或展示版本。
多个远端给出完全相同字节时合并为一个候选，并保留全部精确恢复来源。不同字节即使声明
相同 `mod_id + version` 也必须保持为不同候选，因为依赖、环境、provides 或 bundled
可能不同。只有同一内容哈希被解析出不一致元数据才是内部一致性错误。

反过来，一个 provider locator 在不同 artifact 中可能声明多个真实 `mod_id`，例如
项目改名或同时发布历史身份。`CandidateCatalog` 按真实 `mod_id` 分区，不要求
“一个 locator 等于一个包”。已有包的 upgrade 固定跟随 lockfile 的 `mod_id`；若该
身份已不再发布，明确要求按替换处理。新 `add` 会分别对每个真实身份做完整可行性求解：
只剩一个可行身份时自动采用，多个可行身份时先让用户选择包身份，再进行正常的多解
方案选择。provider slug/project ID 始终只是下载定位符。

## 5. Provider 集合与认证

`create_providers()` 保持 `[resolver].catalogs` 的顺序，供无限定搜索稳定展示。
对已有包执行 add/install/outdated/upgrade/check 时，manifest 与 lock 中所有确切
`remotes` 都加入同一发现任务；不会因第一个 provider 返回结果就停止，也不会按同名
slug 跨平台猜项目。

`orbit init` 尚无 manifest 可提供来源列表，因此可使用专门的 identification factory：
Modrinth 始终参与，只有已配置 API Key 时才加入 CurseForge。它不会因为用户没有 Key
而破坏默认初始化，也不会跳过一个已显式配置但认证失败的 provider 错误。

`orbit sync` 与 init 使用同一 identification factory：Modrinth 始终参与，配置 Key 后
CurseForge 也参与批量哈希识别。sync 不枚举项目候选、不下载 JAR、不联网修复依赖；
provider 错误会终止对账，不能把未完成识别的内容伪装成 `file`。

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
- 所有 dependency relation 的 project ID 都进入递归下载发现；relation type 仅供
  `info` 展示，不进入 JAR 依赖图；
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
包含 golden vectors；provider 合同测试验证指纹批量请求与项目映射。`init`
可以据此识别手动放入的 CurseForge JAR。识别只接受平台的精确 hash/fingerprint
匹配；批量接口失败会保留 provider 名称并报错，不会按文件名或展示版本猜来源。

## 8. 下载限制与错误

CurseForge 项目可以没有第三方可用的下载 URL。Orbit 只使用 API 返回的内联 URL 或
download-url 端点；若任一目标文件不可用、缺 SHA-1 或无法取得 API download URL，
整个发现阶段失败并列出文件，不会拿其余文件组成残缺候选集。完全没有匹配文件仍返回
空候选，使完整多远端候选集和 `orbit check` 的“不兼容”结果保持正常。Orbit 不会拼接
CDN URL，也不会把 HTML 错误页当 JAR。

根据 CurseForge 的
[文件下载认证公告](https://blog.curseforge.com/introducing-api-key-authentication-for-curseforge-file-downloads/)，
直接 CDN 下载从 2026-07-16 起也需要 `x-api-key`。所有 provider 共用
`ArtifactDownloadClient` 下载路径；CurseForge 只在运行时为 HTTPS
`forgecdn.net` 及其子域添加 Key，每一跳重定向都重新校验。Key 不进入
`RemoteArtifact`、`InstalledMod` 或 lockfile，也不会被发送给任意 API 返回 URL。

API 状态错误保留 HTTP 状态和最多 500 字符响应正文；API Key 不进入日志或错误文本。
候选队列必须完整验证；任一已排队 artifact 下载、校验或 JAR 解析失败都会返回具体
错误，避免在不完整搜索空间上伪造求解结果。

## 9. 测试边界

仓库测试通过本地 mock HTTP server 验证：

- `x-api-key` 与官方 query 参数；
- 下载 Key 的 HTTPS/域名范围，以及匿名下载不携带 Key；
- game/class 动态发现；
- loader/game version 搜索；
- download-url 回退；
- fingerprint 批量识别；
- CurseForge `remotes` / `artifact_sources` roundtrip；
- catalog 顺序和缺 Key 错误；
- 同一字节跨 provider 合并、同版本不同字节保持独立。

测试不依赖开发者私人 Key。需要上线前 smoke test 时，设置
`ORBIT_CURSEFORGE_API_KEY` 后执行实际 `search`、`info` 与 `add cf:<numeric-project-id>`；最后一项
同时覆盖真实 CDN 认证。
