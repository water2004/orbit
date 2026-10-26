# 网络重试边界

## Launcher 可选 BMCLAPI 下载源

`network.download-source = official|bmclapi` 使用已有 `config get/set/unset/list`；全局
`--download-source` 只覆盖本次命令。默认官方源，GUI 的设置页和安装向导也只调用这些 CLI。
切换只影响之后的请求，不使有效缓存失效，也不改写已锁定的源 URL。

依据 [BMCLAPI 文档与使用协议](https://bmclapidoc.bangbang93.com/) 的路径映射，
`DownloadClient` 在公共下载边界映射 Mojang 元数据/Java/资源、Maven 库、Fabric、
Forge/NeoForge Maven 工件及 authlib-injector 公开下载。Mojang Piston 元数据保留原路径。
版本文档、asset index、Java manifest、校验 sidecar 与实际工件均经过所选源，不存在
“有摘要就回原站”的例外。所有读取共用同一有界传输/重试内核。

选镜像意味着信任镜像提供的元数据，不意味着具有独立的 Mojang 签名认证。版本 JSON、
asset index 和实际文件仍严格验证声明的摘要，错误立即拒绝。唯一的格式化例外是已实测
BMCLAPI 会重新序列化的 Java manifest：它作为该源的结构化元数据解析，用收到字节的
真实 SHA-1 建立 runtime/inventory 身份；不冒称通过官方原始 JSON 的字节摘要校验。
每个 Java 文件依然校验 manifest 声明的 SHA-1，不能借此忽略文件摘要不匹配。

Fabric profile 的库摘要取自同源详细 metadata，要求 coordinate 与仓库地址一致，重复的
冲突记录拒绝。镜像没有提供摘要的库（例如部分 Loader 主 JAR）按 HTTPS 来源信任下载并
固定本地 SHA-256；安装结果 `source_pinned_artifacts` 在 CLI/GUI 明确提示，不冒称已作
上游摘要校验。官方源仍获取缺失的 Maven sidecar。任何已知摘要均不能被响应头或本地
计算值覆盖。
authlib-injector 的镜像 metadata 会返回已改写的下载链接；其 adapter 只接受精确镜像
命名空间并恢复 canonical URL，再交给同一下载边界路由。SHA-256 和 JAR build 身份
仍严格校验，不允许任意主机，也不产生第二条直接下载路径。

Quilt 镜像在服务文档中标为不可用，未明确映射的公共 URL 直接报错，要求显式选官方源。
Forge/NeoForge 外部官方安装器自行联网，目前不能保证其内部请求走镜像，因此在 BMCLAPI
模式下明确拒绝其目录/安装路径；不得静默运行安装器并回源。上述边界不限制官方模式。

账户认证使用独立的官方/用户指定 HTTP client，EULA 也始终来自官方。模组 Provider
不属于 Launcher，切换不会转发 Modrinth/CurseForge 流量。匹配精确 HTTPS 主机，
不匹配带凭据、额外端口、相似域名或任意自定义域名的地址；镜像失败按同一有限重试预算
报错，不暗中切回上游。未在 BMCLAPI 之上新增对外 API/代理协议。

BMCLAPI 由 bangbang93 提供，资源版权属于原站；该服务不保证完整性/时效性。按其协议，
CLI 即使禁用普通进度也显示来源和文档地址，NDJSON 使用 `download_source` 类型化事件；
GUI 安装/设置和下载任务保留可见署名及协议链接。用户自行启用，不在账户登录时推广镜像。

`orbit-http` 是无业务语义的传输组件，分别编进 Orbit、Launcher 与平台 wrapper；不要求程序
同时安装。GUI 不重跑整条命令，事务、版本发现和求解也不重跑。

## 一次请求覆盖到响应体读完

旧实现只在请求发送/响应头阶段重试，响应体中途断开、超时或解压失败会直接退出。现在同一
`RetryState` 覆盖发送、HTTP 状态和完整响应体；各阶段共享预算，禁止嵌套重试倍增。

- 默认额外重试 3 次（最多 4 次尝试）。Orbit provider 使用既有 `network.max-retries`，0 关闭。
  Launcher 与 mrpack 单文件传输默认额外重试 3 次。
- 重试连接/请求传输、超时、响应体读取/解压错误，以及 HTTP 408、429、500、502、503、504。
- 不重试 400/401/403/404、完整响应后的 JSON 语法/领域校验错误、响应超限、哈希错误或本地 I/O。
- 200ms 起指数退避并加入小幅抖动；读取 `Retry-After` 的秒数和 HTTP 日期。单次等待超过
  30 秒时返回该响应，不忽略服务器要求提前重试，也不无限等待。
- 取消会中断异步请求/等待；只能重放可克隆的缓冲请求体。只读批量 POST 明确由各 wrapper
  确认语义，不能按 POST 方法或“认证请求”这一笼统类别决定重放安全性。

## 接入路径

Modrinth/CurseForge 的项目、搜索、版本、依赖和哈希批量查询，共享有界响应读取。错误正文
最多保留 4 KiB，成功 JSON 上限保持 32 MiB，JSON 在传输成功后解析。HTTP 状态保留为类型，
404 能继续走包未找到流程，不解析错误字符串。

Orbit provider JAR 下载与 mrpack indexed file 下载按单文件重试。Launcher 的 Minecraft、
Loader/installer、Java、authlib-injector 元数据、EULA、皮肤、Yggdrasil 公开元数据/profile GET
接入同一传输策略。

Microsoft 认证链按步骤显式标记：Xbox 用户凭据交换、XSTS 授权和 Minecraft login-with-Xbox
可以使用相同 proof 重试，所有权/profile GET 同样可重放，额外重试最多 3 次并覆盖完整
响应体。设备码申请/兑换及 OAuth refresh token 请求只尝试一次；`authorization_pending` /
`slow_down` 仍由 OAuth 轮询状态机处理，不被传输重试吞掉。401/403 和 Xbox `XErr` 等永久
认证拒绝按原路径返回，无效 JSON 不重试；既有账户只在整条登录成功后写入。
协议参考 [Xbox 调用最佳实践](https://learn.microsoft.com/en-us/gaming/gdk/docs/services/develop/best-practices/live-best-practices-calling-xbl)
与 [HMCL 的 Microsoft 认证实现](https://github.com/HMCL-dev/HMCL/blob/main/HMCLCore/src/main/java/org/jackhuang/hmcl/auth/microsoft/MicrosoftService.java)。

Launcher 工件下载仍流式写临时文件；重试时丢弃该次未完成文件，并重新计算哈希，成功后才
发布缓存对象。mrpack 仍在归档事务 staging 中处理，每次重试截断该文件，进度按当前文件
绝对完成字节报告，不能把重复传输累加成超过整合包总大小。重定向、凭据域、TLS、大小和
强哈希的校验规则不放宽；已存在且校验通过的全局缓存仍直接复用。

## 回归

本地 HTTP 故障注入覆盖：响应体截断后成功、503 与 body 错误混合预算耗尽、批量 POST 的
正文/头完整重放、永久状态不重试、响应大小限制、完整但无效 JSON 不作网络重试、Retry-After
秒数/日期及长等待拒绝、取消退避后不发新请求。不依赖线上服务故障或启动实际游戏。
Microsoft 链另验证 proof POST 的 408 后成功、混合 body 故障、四次尝试上限、相同正文/头、
XErr/无效 JSON 不重试、OAuth 请求不重放，以及所有响应大小有界；仅使用本地服务与虚构凭据。
