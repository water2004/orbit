# Orbit Launcher 架构草案

> 状态：首个可运行基线已实现（2026-07-28）。当前可用命令以
> [`orbit-launcher-cli.md`](orbit-launcher-cli.md) 为准；本文同时保留明确标为“规划”的后续能力，
> 规划内容不等于当前 CLI 已宣称支持。
>
> 本文定义 `orbit-launcher` 的产品边界、领域模型、CLI 协议、平台策略与实现顺序。
> 文中的“必须”“不得”是实现约束；“建议”“待定”用于标出需要在评审后固化的决策。

## 1. 目标

`orbit-launcher` 是独立的 Minecraft Java Edition CLI Launcher。它负责构造、更新并启动
一个完整的 Minecraft 客户端或独立服务端运行时，同时为未来 GUI 提供稳定的 JSON 与
NDJSON 进程协议。

Launcher 负责：

- Minecraft 客户端、服务端及其官方元数据；
- Fabric、Quilt、Forge、NeoForge Loader 的下载、安装与更新；
- libraries、assets、natives、logging 配置与启动参数；
- Java 运行时的发现、下载、校验、选择、固定与更新；
- 客户端账户登录、令牌刷新和安全存储；
- 客户端与独立服务端进程的启动、监视和停止；
- 全局实例注册、当前目录实例上下文和一次命令创建并安装实例；
- 当前登录会话内的服务端 supervisor、IPC 与异常退出自动重启；

Launcher 不负责：

- 搜索、下载、更新或删除 mod；
- 解析 mod 元数据、mod 版本约束或 Jar-in-Jar；
- mod 依赖求解、多方案选择或字节码兼容审计；
- 识别、调用或适配其他包管理器；
- 解释任何不属于 Launcher 的实例文件。
- LittleSkin 等站点专属 OAuth 扩展、联机大厅、房间、好友、聊天、端口映射或服务器
  发现；
- Windows Service、systemd/launchd service 和开机启动。

Launcher 只管理自己声明拥有的运行时 artifact 和状态文件。实例中的其他内容一律视为
不透明用户数据：不扫描、不解释、不改写，也不根据其内容改变安装或启动行为。

## 2. 仓库和依赖边界

在现有 workspace 中新增两个 crate：

```text
orbit-launcher-cli       package name: orbit-launcher
                         binary name:  orbit-launcher
                         参数、交互、文本/JSON/NDJSON 输出
        ↓
orbit-launcher-core      安装、运行时、账户和启动领域逻辑
```

约束：

- 两个 Launcher crate 不依赖现有 Orbit crate 或任何 mod provider wrapper；
- Launcher 的源码、配置、协议和命令中不存在 Orbit 发现、调用、适配或集成逻辑；
- 现有 Orbit 同样不得新增 Launcher 发现、调用、适配或集成逻辑；
- 两个 CLI 可以采用相似的人机输出惯例，但不共享运行时状态，也不互相协商版本；
- HMCL 仅用于研究所需行为和异常边界。HMCL 是 GPLv3，本仓库使用 MIT，禁止复制、改写
  或移植 HMCL 源码；实现以官方格式、官方服务和独立测试夹具为依据。

## 3. 总体结构

```text
orbit-launcher-cli
  command                 clap 命令和输入校验
  interaction             仅 text + TTY 的选择、确认和登录引导
  output                  text renderer / JSON envelope / NDJSON progress
      ↓
orbit-launcher-core
  instance                实例意图、锁定状态和注册表
  platform                OS、架构、目录、可执行文件和进程能力
  metadata/mojang         版本清单、version JSON、规则与继承
  loader/{fabric,quilt,forge,neoforge}
                          Loader 特有的版本发现和安装 adapter
  java                    Java requirement、provider、runtime 和校验
  account                 Microsoft / Offline / External Yggdrasil
  authlib_injector        外置认证 agent 下载、校验与 client/server 启动参数
  artifact                下载计划、内容缓存、哈希和物化
  install                 staging、事务、恢复和 lock 写入
  launch                  LaunchPlan、参数、natives 和进程生命周期
```

公共模块不得比较 Loader 名称字符串。Loader 身份必须在 CLI/配置边界转换成封闭枚举：

```text
LoaderKind = Vanilla | Fabric | Quilt | Forge | NeoForge
```

“共享安装管线”表示各 adapter 生成相同的 `InstallPlan`，不表示不同 Loader 必须使用
相同安装规则。Fabric/Quilt profile 与 Forge/NeoForge installer 的差异必须保留在各自
adapter 中。

## 4. 核心领域模型

### 4.1 实例

```text
Instance
  id                  稳定的 Launcher 内部 ID
  name                用户可见名称
  root                游戏/服务端目录的绝对路径
  kind                Client | DedicatedServer
  desired             用户配置的版本和更新策略
  locked              已解析、已安装的精确状态
```

实例目录必须是用户显式创建或导入的目录。注册表中的路径只是索引，实例自身文件是事实
来源；移动实例后可通过 `instance import` 重新注册。

`id` 是不可变 UUID；`name` 是全局唯一但可修改的人类名称；`root` 可以在实例移动和重新
导入后变化。GUI 和 supervisor 必须始终使用 `id`，不能把名称或绝对路径当作进程身份。
路径只允许出现在 `instance create`、`instance import` 或 `install --new` 的创建边界，
日常命令不得要求位置参数形式的实例路径。

命令实例上下文按固定优先级解析：

1. 显式 `--instance <id|name>`；
2. 当前目录直接包含 `orbit-launcher.toml`；
3. 全局默认实例（仅只读命令）；
4. 否则返回 `instance_context_required`。

不得向父目录递归猜测实例。`install`、`update`、`repair`、`launch`、`server start/stop/command`
等写入、启动或控制进程的命令，不得从无关目录静默使用全局默认实例；GUI 应始终传稳定
`id`。当前目录已有实例或显式传入 `--instance` 时不受此限制。

### 4.2 目标平台

```text
TargetPlatform
  os                  Windows | Linux | MacOS
  arch                X86_64 | Aarch64 | X86
  environment         Desktop | Headless
  capabilities        symlink、executable bit、keyring、process group 等
```

本机安装默认使用探测到的平台，但探测结果必须打印在 plan 中。下载或准备另一平台的
实例时必须显式传入 `--target`，不得根据文件名猜测。

首个稳定版本的目标矩阵：

| 实例 | Windows x86_64 | Linux x86_64 | Linux ARM64 | macOS x86_64 | macOS ARM64 |
| --- | --- | --- | --- | --- | --- |
| Client | 必须 | 必须 | 条件支持 | 必须 | 必须 |
| DedicatedServer | 必须 | 必须 | 必须 | 支持 | 支持 |

“条件支持”表示 Minecraft、Loader 和 Java provider 都能提供该平台所需 artifact；缺少
官方 artifact 时必须报 `unsupported_target_artifact`，不得下载另一架构后尝试启动。

### 4.3 版本意图与锁定状态

用户配置表达意图：

```text
MinecraftRequirement = Exact(version) | LatestRelease | LatestSnapshot
LoaderRequirement    = None | Exact(kind, version) | LatestStable(kind) | Latest(kind)
JavaPolicy           = Managed(provider) | System(path) | ExactManaged(runtime_id)
```

锁定状态记录一次解析后的精确事实：

```text
LockedRuntime
  minecraft_version
  minecraft_manifest_url + hash
  loader_kind + loader_version + installer/profile identity
  java_runtime_id + major + vendor + platform
  main_class
  resolved_arguments
  artifact inventory          Download | InstallerOutput provenance + SHA-256
  generated runtime files
```

`Latest*` 只在 `install` 或 `update` 的计划阶段解析。`launch` 永远使用 lock 中的精确
状态，不在启动时隐式检查更新。

## 5. 文件与目录

### 5.1 全局目录

使用平台目录 API，不手写 Windows 路径：

| 类型 | Windows | Linux | macOS |
| --- | --- | --- | --- |
| config | Roaming AppData | `XDG_CONFIG_HOME` | Application Support |
| data | Local AppData | `XDG_DATA_HOME` | Application Support |
| cache | Local AppData cache | `XDG_CACHE_HOME` | Caches |

各目录下使用独立的 `orbit-launcher` 子目录。CLI 必须
提供全局 `--config-dir`、`--data-dir`、`--cache-dir`，用于便携安装、测试和无标准用户
目录的服务器环境。显式路径优先于平台默认路径，路径解析失败直接报错。

全局数据建议布局：

```text
config/
  config.toml
data/
  instances.toml             实例 ID 到路径的注册表
  accounts.json              仅非秘密账户元数据
  auth-sessions/             有期限的登录会话，不含最终 refresh token 明文
  supervisors/               Linux 上按实例 ID 建立的权限受限 Unix socket
  runtimes/                  已物化的共享 Java runtime
cache/
  objects/sha256/<digest>    内容寻址缓存
  metadata/                  带 ETag/过期信息的远端元数据
  staging/                   可恢复的临时事务
```

### 5.2 实例目录

```text
<instance>/
  orbit-launcher.toml        用户意图和启动设置
  orbit-launcher.lock        精确运行时与 artifact inventory
  .orbit-launcher/
    transaction.json         仅事务进行中存在
    generated/               生成的 argfile、classpath 和运行脚本数据
    supervisor.lock          只用于单实例 supervisor 所有权，不作为 PID 猜测依据
    supervisor.*.log         后台 supervisor 的 stdout/stderr
  ...                        标准运行时文件及不透明的用户文件
```

`orbit-launcher.lock` 必须使用相对实例根目录的规范路径，并记录 schema version。秘密、
access token、refresh token、密码和 device code 不得进入配置、lock、日志或错误 detail。

## 6. Minecraft 元数据与安装

### 6.1 事实来源与兼容基准

Launcher 不定义自有的“Minecraft version JSON”格式，也不把内部 lock 伪装成官方
version JSON。远端模型和运行时模型必须分开：

```text
MojangVersionManifestV2       官方版本索引 DTO
MojangVersionJson             官方单版本 DTO
LoaderProfile                 各 Loader 官方 profile/installer DTO
        ↓ validate + resolve
ResolvedRuntime               仅在内存中的统一启动模型
        ↓ install
orbit-launcher.lock           artifact 来源、hash、路径和事务事实
```

这不禁止 Launcher 拥有自己的 TOML；相反，实例必须用 `orbit-launcher.toml` 保存用户可
编辑的意图和策略。三个持久层的职责不可混合：

| 持久层 | 内容 | 可否由用户编辑 |
| --- | --- | --- |
| 官方 JSON/profile | Mojang 与 Loader 发布的版本、库、参数和 hash 事实；按原文缓存 | 否 |
| `orbit-launcher.toml` | Minecraft/Loader 版本要求、Java policy、内存/JVM 参数、账户引用、服务端重启与认证策略 | 是 |
| `orbit-launcher.lock` | 精确解析版本、artifact inventory、相对路径、hash、main class 和已生成运行时事实 | 否 |

TOML 可以引用非秘密账户 ID 和 External Yggdrasil provider ID，但 access/refresh token、
密码、device code 和会话秘密只能进入凭据存储。安装与更新把 TOML 意图和官方 JSON/profile
解析成 lock；启动只消费 lock、TOML 中允许启动时覆盖的配置以及安全凭据，不在启动时改写
官方 metadata。

事实来源按以下优先级使用：

1. Mojang 官方 `version_manifest_v2.json`、它指向的单版本 JSON、asset index 和 Java
   runtime manifest；
2. Fabric、Quilt、Forge、NeoForge 官方 metadata、profile、Maven 和 installer 输出；
3. 对官方没有单独文档说明的继承、旧版字段和异常格式，以 HMCL 当前稳定行为作为兼容
   基准，并用真实官方 metadata fixture 固定行为；
4. 仍无法从上述来源确定的情况必须标为不支持或写成有来源的显式 compatibility policy，
   不允许凭经验猜测。

Mojang 提供并持续更新官方 JSON payload，但没有公开一份覆盖所有历史字段、继承合并和
Launcher 行为的独立版本化规范。因此“官方规范”在这里指 Mojang Launcher 实际发布的
metadata 及其 hash 链，而不是项目自行整理的一份替代 schema。

HMCL 的 `Version` 类是兼容性超集，其中既有 Mojang 字段，也有 `priority`、`root`、
`hidden`、`patches`、`resolved` 等内部或其他 Launcher 扩展。Launcher 不会把这整个类
照搬成远端 DTO：

- 官方 DTO 只声明官方 payload 中已知并实际消费的字段；
- 原始官方 JSON 按内容 hash 缓存，未知字段允许存在并随原文保留；
- 已知必需字段缺失、类型错误或 hash 不匹配时直接报错；
- 未知字段不进入启动语义，也不因当前代码尚不认识就拒绝整个官方文件；
- Loader profile 的扩展只由对应 Loader adapter 解释，不能污染 Mojang DTO；
- 内部 `ResolvedRuntime` 不序列化成一个自创的 Launcher version JSON；
- 必须写入标准游戏目录的 profile JSON 时，优先保存或组合官方 Loader 给出的格式，
  不添加只有 Orbit Launcher 能理解的启动字段。

HMCL 作为行为基准的方式是建立差分 fixture：同一份官方 Minecraft/Loader metadata 应
得到等价的继承结果、libraries 顺序、规则选择、natives、assets、main class 和参数。
由于许可证不同，只比较输入输出行为，不复制其源码、测试或常量表。

### 6.2 客户端

客户端安装必须完整处理：

- Mojang version manifest v2；
- 目标 version JSON 及 `inheritsFrom` 继承链；
- client JAR；
- 普通 libraries 和平台适用规则；
- native classifiers、解压排除项和 native 目录；
- asset index、content-addressed asset objects 和旧版 virtual/resources 布局；
- logging client 配置；
- `arguments.jvm`、`arguments.game` 以及旧版 `minecraftArguments`；
- feature、OS name/version/arch 规则；
- main class、classpath 顺序和 placeholder；
- Java component 与 major version 要求。

版本继承必须先构造可测试的 `ResolvedVersion`，后续下载和启动只能消费解析结果，不能
在命令生成时再次临时合并 JSON。

### 6.3 独立服务端

服务端不复用客户端 JAR 或账户逻辑。Vanilla adapter 从 version JSON 的 server download
获得服务端 JAR，校验后生成服务端 `LaunchPlan`。

服务端必须支持：

- `server eula show` 和 `server eula accept`；
- 前台启动并继承终端 stdin/stdout/stderr；
- 通过 stdin 发送 `stop` 后等待正常退出；
- Ctrl+C / SIGTERM 的首次信号执行优雅停止，超时后才允许强制结束；
- `nogui`、内存和 JVM 参数；
- PID、启动时间、退出码和日志路径的结构化事件；
- headless Linux 上没有图形环境、浏览器和系统 keyring 的正常运行。

不得自动接受 EULA，也不得让普通 `--yes`、默认配置或安装脚本静默代替法律确认。服务端
bootstrap/install 在提交运行时和写入 `eula=true` 前，必须获取并展示
[Minecraft 官方 EULA](https://www.minecraft.net/en-us/eula) 的完整正文，然后针对该正文的
SHA-256 digest 明确询问用户是否同意。终端文本模式完整写出正文，不能只打印摘要或链接。

JSON、GUI 和其他非交互调用使用两步协议：`server eula show` 返回完整正文、官方 URL、
获取时间与 digest；随后 `server eula accept <digest>` 或 bootstrap 的等价参数只接受刚刚
展示的 digest。Launcher 在实例 lock 中记录 URL、digest、接受时间和交互方式，不记录用户
身份信息。官方正文 digest 变化后，下一次 server install/update 必须重新展示并确认；launch
不隐式联网检查 EULA。无法取得完整正文时停止安装，不能用缓存摘要或旧链接伪造确认。

后台服务由一个持续拥有子进程 stdin 的 supervisor 管理。`server start` 同时提供本地 IPC、
状态查询、控制台命令和优雅停止；实现不得只写 PID 后丢失 stdin。

## 7. Loader adapter

每个 adapter 实现相同边界：

```text
LoaderAdapter
  list_versions(minecraft, side, target)
  resolve(request, minecraft, side, target)
  plan_install(resolved_loader, resolved_minecraft, staging_root)
  inspect_install(staging_root)
  plan_launch(locked_runtime, side)
```

`side` 是 `Client` 或 `DedicatedServer`，不是可忽略字符串。一个 adapter 可在内部共享
客户端和服务端的 metadata DTO，但必须允许两条安装路径不同。

| Loader | Client 策略 | Dedicated server 策略 |
| --- | --- | --- |
| Vanilla | Mojang version JSON | Mojang server JAR |
| Fabric | Fabric Meta profile JSON | Fabric server profile/bootstrap |
| Quilt | Quilt metadata/profile | Quilt server installer/profile |
| Forge | 官方 installer 的客户端格式 | installer `--installServer` 语义 |
| NeoForge | 官方 installer 的客户端格式 | installer `--installServer` 语义 |

Forge/NeoForge installer 是被下载并执行的 Java 程序。Launcher 必须：

- 只从配置的官方仓库和解析出的精确版本下载；
- 在 staging 目录内用已选择的受管 Java 执行；
- 捕获并限量转发 stdout/stderr，检查退出码，并服从可配置超时；
- 执行后重新检查生成的 version、libraries、argfile 和主入口；
- 不把 `run.sh` / `run.bat` 当作跨平台事实来源；
- 将 installer URL、installer SHA-256、`install_profile.json` SHA-256 和逐文件输出清单写入 lock；
- 遇到未知 installer schema 时返回 `unsupported_requirement`，不猜测参数。

Loader 更新只替换 lock 中由 Launcher 拥有的运行时文件。其他实例文件不进入计划。

## 8. Java 运行时

### 8.1 Java requirement

Java 要求按来源合并：

1. Minecraft version JSON 的 `javaVersion.component` 和 `majorVersion`；
2. Loader 官方 metadata/installer 明确给出的更严格要求；
3. 对缺失 `javaVersion` 的旧版本，使用有来源、版本化且可测试的 compatibility policy；
4. 用户显式的 Java 选择只能比最低要求更严格，不能绕过不兼容检查。

规则必须产出 `JavaRequirementReason`，错误中说明要求来自 Minecraft、Loader 还是用户
配置。禁止散落硬编码 `if minecraft >= ...`。

### 8.2 Provider

建议首版支持：

- `mojang`：Mojang Java Runtime manifest；
- `temurin`：Eclipse Temurin，用于 Mojang 未覆盖的平台；
- `system`：用户显式指定或选择的本机 Java，不由 Launcher 更新。

`auto` 若存在，必须是文档化的确定性 provider 顺序，而不是异常后的静默兜底。解析结果
要在计划中显示最终 provider、版本、架构和下载大小，用户确认后才能执行。

### 8.3 校验和更新

受管 Java 必须：

- 逐文件验证远端提供的 hash 和 size；
- 安全处理目录、可执行位和符号链接，拒绝路径穿越；
- 读取 runtime 的 `release` 文件；
- 运行 `java -XshowSettings:properties -version` 验证真实 major、OS 和 arch；
- 以不可变 runtime ID 安装，成功后原子切换实例引用；
- 允许多个实例共享同一 runtime；
- 更新时保留仍被 lock 引用的旧 runtime。

系统 Java 不因路径存在就视为有效；每次选择和必要的 launch preflight 都要验证可执行文件
及其报告的平台。Launcher 不修改系统 `PATH`。

## 9. 下载、缓存与事务

### 9.1 Artifact

所有远端文件统一转换为：

```text
ArtifactSpec
  logical_name
  urls[]
  expected_size?
  hashes { sha1?, sha256? }
  destination
  executable
  extraction?
  provenance
```

`logical_name` 用于输出，hash 和 URL 不作为主要用户界面名称。没有任何可验证 hash 的
artifact 必须在 plan 中标记 `unverified_remote`；安全策略可以拒绝。完成下载后总是计算
SHA-256，用它作为本地缓存主键。

### 9.2 下载阶段

安装严格分为：

1. 解析全部 metadata 和版本选择；
2. 生成完整下载队列；
3. 按内容 hash 查询缓存；
4. 并发下载缺失对象；
5. 校验并写入 CAS；
6. 从 CAS hardlink/reflink/copy 到 staging；
7. 执行需要的 Loader installer；
8. 检查 staging 中的完整 runtime；
9. 原子提交实例与 lock。

元数据解析不得在 artifact 下载 worker 中递归添加任意任务。若远端 manifest 在解析后
引入新 artifact，必须产生新的明确 plan revision 并更新总工作量。

### 9.3 缓存

Launcher 缓存采用内容寻址和命令结束后的 LRU 清理：

- lock、进行中事务和已安装共享 runtime 引用的对象不可淘汰；
- 容量可由 `config get/set cache.max-size` 管理；
- 访问时间通过单命令 journal 合并，避免每个下载 worker 竞争写索引；
- 清理失败不回滚已经成功的实例事务，但必须返回 warning；
- `cache clean`、`cache gc` 和 `cache verify` 提供 JSON 报告。

### 9.4 事务

实例内一次只能有一个写事务。锁必须包含进程 ID、开始时间、命令和可验证的进程身份，
不能仅凭旧 PID 判断锁仍存活。

事务规则：

- 下载和 installer 只写 staging；
- 旧运行时在新运行时验证完成前不删除；
- lock 最后写入并使用临时文件 + atomic replace；
- 崩溃后 `repair` 根据 transaction journal 完成回滚或提交；
- 跨卷无法原子 rename 时，计划阶段必须显示降级策略；
- 未记录为 Launcher 所有的文件永远不进入 Launcher 删除集合。

## 10. 账户和登录

### 10.1 Provider

建议账户模型：

```text
AccountProvider
  Microsoft
  Offline
  ExternalYggdrasil { api_root }
```

Microsoft 在线账户至少实现：

- OAuth public client flow；
- device code，供 CLI 和无本地浏览器环境使用；
- authorization code + PKCE，供桌面浏览器体验使用；
- Xbox Live、XSTS、Minecraft Services 交换；
- Java Edition entitlement 与 profile 检查；
- access token 过期和 refresh token 轮换；
- 用户撤销授权、儿童账户和无游戏许可证的明确错误。

Orbit Launcher 必须使用自己的 Microsoft Entra application/client ID。client ID 不是秘密，
但必须来自本项目的应用注册；禁止复用 HMCL 或其他 Launcher 的 client ID。发布构建可以
通过编译参数注入 client ID，开发构建允许从显式配置读取。

Offline account 只生成离线身份，不得把它描述为已经通过 Microsoft 验证。

External Yggdrasil 使用 authlib-injector 兼容的标准 API metadata 与 Yggdrasil
authenticate/refresh/validate/invalidate 路径，处理账户登录、角色选择、令牌刷新和吊销。
服务端配置外置认证后，Launcher 必须下载并校验精确的 Authlib Injector artifact，把
`-javaagent` 和经验证的 API root 写入服务端 `LaunchPlan`；客户端启动外置账户时使用同一
受管 agent。agent 路径、版本、来源与 hash 写入 launcher lock，但账户 token 不得进入。

不实现 LittleSkin 或其他站点专属 OAuth、扫码、网页登录扩展。某个兼容站点只要提供标准
External Yggdrasil/authlib-injector 接口即可通过通用 provider 使用，CLI 和 core 不按站点
名称分支。

### 10.2 秘密存储

账户列表只保存 provider、profile name、UUID、最后登录时间等非秘密元数据。秘密通过
core 的 `SecretStore` trait 按稳定 `account_id` 读取、原子替换和删除，不允许账户模块
直接操作某个平台的密钥 API。

持久化会话内容按 provider 区分：

- Microsoft 保存 refresh token、当前 access token、过期时间和轮换版本；启动先静默刷新，
  成功持久化新 token 后才删除旧 token；
- External Yggdrasil 保存 access token 与 client token；启动执行
  `validate -> refresh -> interaction_required`，密码只用于首次 authenticate，永不保存；
- Offline 不产生 secret record。

Windows backend 使用当前用户作用域 DPAPI 保护任意长度的版本化 secret envelope，密文
原子写入 Launcher data 目录；不得使用 machine scope。Linux 桌面 backend 使用 Secret
Service。两者都应在同一操作系统登录会话中静默读取，不要求用户重复登录游戏账户。

无 Secret Service 的 headless Linux 当前明确返回 `secret_store`，不静默降级到明文或内置
应用密钥混淆。加密 vault 与 credential agent 属于后续规划，当前不能写进可用能力列表。
纯服务端 External Yggdrasil/Authlib Injector 配置不含用户 token，因此不依赖桌面 keyring。

HMCL 只作为“公开账户 metadata 与私有可续期 session 分离、启动时静默 validate/refresh”
的行为参考。不得复制其源码，也不得采用内置固定密钥的便携混淆作为安全存储。

所有 secret buffer 在使用后清零；JSON、日志、错误、`launch --dry-run`、导出和诊断包只
出现 `account_id`。logout 先尽力调用远端 invalidate/revoke，再删除本地 secret；远端不可用
不能阻止本地凭据删除，但必须返回 warning。

### 10.3 非交互协议

JSON 模式不得弹 prompt。登录拆成可恢复步骤：

```text
orbit-launcher account login microsoft begin
orbit-launcher account login microsoft complete <login-session-id>
```

`begin` 返回 verification URI、user code、过期时间和不含秘密的 session ID；`complete`
轮询并在成功后保存凭据。临时 device code 只能放入权限受限、到期自动删除的 auth session，
不得出现在普通日志。

CLI 和 GUI 都使用显式的两步协议；这样退出或切换前端后仍能在有效期内继续同一个授权会话。

## 11. LaunchPlan

安装状态与启动动作通过不可变 `LaunchPlan` 分离：

```text
LaunchPlan
  instance_id
  side
  java_executable
  working_directory
  environment
  jvm_arguments[]
  main_class | executable_jar
  game_arguments[]
  classpath[]
  native_directory?
  redacted_arguments[]
```

生成规则：

- 只消费 `orbit-launcher.lock` 和显式 launch override；
- 使用 `std::process::Command` 的参数数组，不经过 shell 拼接；
- placeholder 必须有封闭枚举，缺少值直接报错；
- classpath 顺序保持 metadata/Loader 语义；
- Windows 命令长度超限时使用明确的 Java argfile/classpath 方案；
- Unix 可执行位、符号链接和进程组由平台 backend 处理；
- access token、client token、UUID 等敏感参数在日志和 JSON 中替换成标记；
- `launch --dry-run` 只输出脱敏计划，永远不输出可复用 token。

客户端启动前刷新必要的账户 token，但不检查 Minecraft、Loader 或 Java 更新。服务端启动不创建
账户或认证参数。

## 12. 进程生命周期

客户端：

- 结构化上报 spawned、running、stdout/stderr line、exited；
- 可选择是否将游戏输出原样转发到当前终端；
- Launcher 被中断时，不默认杀死已经成功启动的客户端；
- 临时 natives 只在进程结束且没有其他进程引用后清理。

服务端：

- 默认前台运行并保有 stdin；
- 记录 PID、启动时间、实例 ID 和进程身份；
- `stop` 先发送 Minecraft `stop` 命令；
- 超时策略分为 graceful timeout 和 kill timeout；
- detached 模式只能由受管理 supervisor 提供；
- 日志轮转可以由 Launcher 或服务端 logging 配置负责。

服务端重启策略是封闭枚举：

```text
RestartPolicy = Never | OnUnexpectedExit | Always
```

`OnUnexpectedExit` 是推荐策略。只有 `server stop`、当前 supervisor 收到 Ctrl+C/SIGTERM、
维护操作或显式 IPC shutdown 才是 expected exit；未被标记为 expected 的进程退出即使状态码
为 0 也应重启。启动前校验失败、EULA 未接受、lock 损坏和 Java 不兼容不属于可重试的游戏
进程崩溃。

supervisor 必须实现指数退避、最大退避、固定窗口内的重启次数上限，以及稳定运行后重置
失败计数。每个 spawned、exited、backoff、restarting 和 restart_limit_reached 状态都进入
结构化事件。前台 `server run` 在当前进程内监督；`server start` 启动同一版本的
后台 supervisor，并通过 Windows Named Pipe 或 Unix Domain Socket 保有 stdin、status、
command 和 stop 能力。

detached supervisor 只承诺在当前登录会话内存活。不实现 Windows Service、systemd 或
launchd，不承诺开机启动，也不承诺用户退出登录后继续运行。

客户端与服务端共同使用 core 中的参数数组与进程事件模型；Windows Named Pipe 和 Unix
Domain Socket 只存在于平台 IPC 模块，不进入安装或账户领域。

## 13. CLI 设计

目标命令树如下；其中尚未出现在当前 CLI 的节点仍是规划，不是空壳命令：

```text
orbit-launcher
  instance create|import|list|show|rename|remove|default
  versions minecraft|loader|java
  install [--new <name>] [--root <path>] [--kind <client|server>]
  update [--minecraft] [--loader] [--java]
  verify
  repair
  launch
  server eula show|accept
  server run|start|stop|status|command
  java list|discover|install|select|update|remove
  account login|list|show|select|refresh|logout
  cache info|verify|gc|clean
  config path|get|set|unset|list
```

Fabric/Quilt 的官方 profile 同时是可互操作的落盘事实。安装事务必须逐字节保留已验证响应，
写到标准 `versions/<profile-id>/<profile-id>.json` 并作为 Loader artifact 纳入 lock；不得只在
内存中合并后丢弃。Orbit、第三方启动器和诊断工具由此可以读取标准 profile，而不需要认识
`orbit-launcher.lock`。Forge/NeoForge 则保留官方 installer 生成的 profile/argfile，二者在
统一 runtime 模型之上维持各自真正不同的安装规格。

当前 Java 管理已实现 `java list [--verify]`、`java verify <runtime-id>` 和
`java remove <runtime-id>`。下载与更新不另设一条旁路：实例 `install` 根据官方 Minecraft
version JSON 解析所需 Java component，把 Mojang runtime 与游戏/Loader 一起纳入同一个安装
事务。删除前扫描全部已注册实例 lock；仍被引用的 runtime 必须拒绝删除。`discover`、手动
`install/select/update` 仍是规划节点，不得由 GUI 猜测或模拟。

版本选择的只读面已经实现为 `versions minecraft`、`versions loader` 和 `versions java`。
它们与安装器复用同一 Mojang/Loader 官方 metadata adapter；前端只展示并提交精确 choice，
不维护第二份版本排序或兼容规则。实例 `show` 同时返回 desired intent 与可选的 installed lock
摘要，更新界面必须用两者形成差异，而不能把 `stable/latest-release` 当作已安装版本。

全局选项：

```text
--format text|json
--progress-format none|ndjson
--quiet
--non-interactive
--instance <id|name>
--config-dir <path>
--data-dir <path>
--cache-dir <path>
```

text + TTY 可以选择版本、账户和更新方案。以下情况禁止 prompt：

- `--format json`；
- `--non-interactive`；
- stdin 不是 TTY；
- 服务或 supervisor 调用。

非交互情况下缺少选择必须返回 `interaction_required`，detail 中给出稳定 choice ID、用户
描述、差异和可用于重试的等价 CLI 参数。

`install` 同时支持已有实例和一次命令 bootstrap：

```text
# 当前目录尚无实例：创建、注册并完整安装
orbit-launcher install --new main-server --kind server \
  --minecraft 1.21.1 --loader fabric --loader-version stable --java auto

# 从任意目录创建；--root 只在这个创建边界出现
orbit-launcher install --new main-client --root <path> --kind client \
  --minecraft latest-release --loader fabric --loader-version stable --java auto

# 已有局部或显式全局实例
orbit-launcher install
orbit-launcher --instance <id> install
```

`--new` 默认把当前目录作为 root。bootstrap 必须在一个事务内完成实例文件、artifact 和
全局注册：安装失败不得留下一个宣称可用的注册表条目；成功结果返回稳定 `instance_id`。
客户端安装不要求账户，账户只在 launch 时使用。

## 14. 输出协议

### 14.1 最终结果

stdout 成功结果：

```json
{
  "schema_version": 2,
  "command": "install",
  "ok": true,
  "result": {
    "instance_id": "main-client",
    "minecraft": "1.21.1",
    "loader": { "kind": "fabric", "version": "0.16.14" },
    "java": { "major": 21, "provider": "mojang" },
    "downloaded": 12,
    "reused": 314
  }
}
```

JSON 错误写入 stderr，使用稳定 code：

```json
{
  "schema_version": 2,
  "type": "error",
  "command": "install",
  "ok": false,
  "code": "unsupported_target_artifact",
  "message": "No Java runtime is available for the selected target",
  "detail": {
    "target": "linux-aarch64",
    "required_java_major": 21,
    "provider": "mojang"
  }
}
```

人类可读 message 不作为 GUI 分支条件。GUI 只能依赖 schema、code 和 detail 字段。

### 14.2 NDJSON 进度

长命令的 stderr 事件：

```json
{"schema_version":2,"type":"progress","command":"install","sequence":7,"phase":"metadata","data":{"event":"minecraft_resolved","version":"1.21.1","total_artifacts":327}}
{"schema_version":2,"type":"progress","command":"install","sequence":8,"phase":"download","data":{"event":"artifact_finished","logical_name":"Minecraft client","size":24876123}}
```

约束：

- `sequence` 在单进程内严格递增；
- `phase` 与 Orbit 包管理命令共用同一个枚举；GUI 不从 `event` 名称猜阶段；
- 已知总工作量通过 `total` 提供，发现新工作时可以增加但不能减少；
- 下载、安装器、Java 和登录轮询都必须提供心跳或进度，不能长时间无输出；
- 进度事件不得包含完整 URL query、token、密码或本地凭据路径；
- 最终结果只出现一次，不混入 NDJSON 流。

### 14.3 退出码

建议：

| code | 含义 |
| --- | --- |
| 0 | 成功 |
| 1 | 一般运行错误 |
| 2 | CLI 参数错误 |
| 3 | 网络或远端服务暂时失败 |
| 4 | 非交互模式需要用户选择/确认 |
| 5 | 安装或 lock 损坏，需要 repair |
| 6 | 账户或授权失败 |
| 7 | 子进程（Loader installer、Java、Minecraft）失败 |

稳定的 JSON `code` 比数值退出码更精细。

## 15. 配置草案

全局 `config.toml`：

```toml
schema = 1

[network]
concurrency = 8
connect_timeout_seconds = 15
request_timeout_seconds = 120

[cache]
max_size = "20 GiB"

[java]
default_provider = "mojang"

[microsoft]
# 发布构建通常由编译参数提供；开发构建可显式配置。
client_id = "..."

[[yggdrasil.providers]]
id = "private-auth"
api_root = "https://auth.example.com/api/yggdrasil"

[ui]
progress_bar = "auto"
color = "auto"
```

实例 `orbit-launcher.toml`：

```toml
schema = 1
id = "018f4f42-..."
name = "main-client"
kind = "client"

[minecraft]
requirement = "1.21.1"

[loader]
kind = "fabric"
requirement = "stable"

[java]
policy = "managed"
provider = "mojang"

[launch]
min_memory_mib = 512
max_memory_mib = 4096
jvm_args = []
game_args = []

[server]
restart = "on-unexpected-exit"
restart_limit = 5
restart_window_seconds = 600
restart_backoff_max_seconds = 60
graceful_stop_timeout_seconds = 30
kill_timeout_seconds = 10

[server.authentication]
provider = "mojang" # mojang / external-yggdrasil
# external-yggdrasil 时必须显式配置：
# yggdrasil_provider = "private-auth"
# authlib_injector = "managed"

```

所有字段都必须可由 `config` 或相应领域命令修改。用户不应为了普通操作被迫手改 TOML。
直接编辑仍受支持，但下次读取时必须进行 schema 和语义校验。

## 16. 更新语义

`update` 先生成方案，不直接写磁盘：

```text
Current
  Minecraft 1.21.1
  Fabric 0.16.10
  Java 21.0.5

Candidate A
  Minecraft unchanged
  Fabric 0.16.14
  Java unchanged

Candidate B
  Minecraft 1.21.4
  Fabric 0.16.14
  Java 21.0.6
```

规则：

- `--minecraft`、`--loader`、`--java` 限定允许变化的轴；
- 至少一个被允许轴严格更新才称为 update；
- 依赖关系要求时，另一个轴可以随方案变化，但必须高亮；
- 多个有效方案在 text 模式中询问，JSON 模式返回 choices；
- 唯一方案若会删除或替换 runtime 文件，仍在执行前报告；
- `--yes` 只确认当前完整 plan digest，plan 变化后必须重新确认；
- update 只处理 Launcher lock 中的 runtime artifact，不触碰其他实例文件。

## 17. 安全约束

- 所有 archive 解压都防止绝对路径、`..`、NTFS alternate stream 和符号链接逃逸；
- manifest 中的 destination 必须规范化并验证位于预期根目录；
- HTTP 默认要求 HTTPS，允许的非 HTTPS 源必须由用户显式配置；
- 下载重试只重试幂等请求，遵守退避和 `Retry-After`；
- hash 不匹配绝不重试使用已有缓存对象；
- 外部 Java installer、Loader installer 和 Minecraft 都作为不同的子进程类型记录；
- 命令参数不经过 shell；
- 日志和错误在序列化前统一脱敏；
- `launch --dry-run`、bug report 和诊断包不得包含 token；
- 账户密码只允许在安全 stdin/TTY 或 OS 凭据 API 输入，不接受普通命令行参数；
- 删除实例、runtime 和 cache 前解析并验证绝对目标，实例删除默认保留 worlds/saves，除非
  用户明确选择完整删除。

## 18. 错误模型

错误按领域分类并保留 cause chain：

```text
ConfigError
MetadataError
UnsupportedPlatformError
DownloadError
IntegrityError
InstallPlanError
TransactionError
LoaderInstallerError
JavaRuntimeError
AuthenticationError
LaunchError
ProcessError
InteractionRequired
```

文本 renderer 可以把 cause chain 格式化成人类说明；JSON 必须同时提供稳定 code、阶段、
artifact/实例的逻辑名称和可操作的 recovery。不得直接把 reqwest、serde 或 `io::Error`
的 Debug 文本当作最终消息。

## 19. 测试策略

### 19.1 单元测试

- Mojang version inheritance 和 rule evaluation；
- 每个 Loader adapter 的 metadata/installer schema；
- Java requirement 合并和平台映射；
- placeholder、classpath、native classifier 和 argument 生成；
- JSON envelope、NDJSON sequence、脱敏和错误 code；
- archive 路径安全；
- transaction recovery；

### 19.2 fixture 测试

将远端 JSON、installer profile 和小型伪 artifact 固定在测试 fixture 中。fixture 必须记录
来源 URL、抓取日期、许可证/再分发条件和内容 hash。测试不能依赖实时网络。

### 19.3 集成测试

- 本地 mock HTTP server：超时、断点、ETag、hash mismatch、动态 plan total；
- 临时实例：install、update、失败回滚、repair；
- 伪 Java 可执行文件：版本/架构不匹配；
- Windows、Linux、macOS CI 的路径、权限、参数和进程行为；
- 真实网络 smoke test 独立于普通 CI，不能成为确定性测试的前提。

### 19.4 支持声明

每个对外声明的 Minecraft/Loader/side/platform 组合必须至少有 metadata fixture 和
LaunchPlan golden test。没有测试覆盖的组合不得笼统宣称“支持”。遇到未知格式必须
结构化报错，并附带安全的诊断信息。

## 20. 实现切片

首个基线已经按以下可独立审阅、可单独测试的边界提交：

1. `docs: define orbit-launcher boundaries and protocols`
2. `feat(launcher): add instance, platform and configuration model`
3. `feat(launcher): add artifact cache and transactional install plans`
4. `feat(launcher): install vanilla client and dedicated server runtimes`
5. `feat(launcher): manage verified Java runtimes`
6. `feat(launcher): add Fabric and Quilt adapters`
7. `feat(launcher): add Forge and NeoForge adapters`
8. `feat(launcher): add accounts and secure credential storage`
9. `feat(launcher): build client and server launch plans`
10. `feat(launcher): complete CLI renderers and non-interactive flows`
11. `test(launcher): cover supported platform and loader matrix`
12. `build: package orbit-launcher for MSI, deb and release archives`

每笔功能提交同时包含对应测试；账户、Authlib Injector、LaunchPlan 与 supervisor 又进一步
拆成独立提交。后续功能仍沿用同一约束，不把多个未验证的大子系统塞进一笔提交。

## 21. 已确认决策与外部前置

1. **账户范围**：实现 Microsoft、Offline 和标准 External Yggdrasil；不实现 LittleSkin
   等站点专属 OAuth。External Yggdrasil 的 client/server 启动都必须支持受管
   Authlib Injector。
2. **Microsoft 应用注册**：需要项目自有的 Entra client ID。源码和开发构建允许显式配置，
   官方 release 通过 CI secret/variable 注入并记录应用所有权；没有 client ID 时相关命令
   明确报错，不阻塞其他模块实现。
3. **Java provider**：当前安装事务只支持 Mojang 受管 runtime。Temurin 和 system Java 是
   规划能力；在完整下载、校验和平台测试落地前不得宣称支持，也不得作为异常后的静默回退。
4. **服务端后台模式**：`server start` 与 supervisor、IPC、stop、自动重启一起交付；不实现
   Windows Service、systemd/launchd service、开机启动或退出登录后继续运行。
5. **旧版本范围**：支持声明按 fixture 和 LaunchPlan golden test 逐步扩大；未覆盖组合不得
   best-effort 宣称支持，遇到未知历史格式返回结构化 unsupported 错误。
6. **实例删除策略**：默认只注销实例并保留目录；删除文件需要第二次明确选择，并默认保留
   world/save/截图。服务端世界同样处理。
7. **发布形态**：`orbit-launcher` 使用自己的 MSI、deb 发布物及 `launcher-v*` 版本生命周期，
   不要求其他程序存在，也不把其他程序打进自己的安装包。

## 22. 参考实现与官方资料

- [Mojang version manifest v2](https://piston-meta.mojang.com/mc/game/version_manifest_v2.json)：
  Minecraft 版本索引；每项 URL 和 SHA-1 指向对应的官方 version JSON；
- [Mojang Java runtime manifest](https://piston-meta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json)：
  官方 Java component、平台和逐文件 manifest；
- [Fabric Meta API](https://meta.fabricmc.net/)：Fabric 版本、client profile 与 server
  metadata；
- [Quilt server installation](https://quiltmc.org/en/install/server/) 及 Quilt 官方
  installer/metadata：Quilt client/server 安装；
- Forge、NeoForge 官方 Maven 和 installer；
- [Microsoft device authorization grant](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-device-code)
  与 authorization code + PKCE：Microsoft public client 登录；
- Xbox Live/Minecraft Services：Microsoft 账户到 Minecraft profile 的认证链；
- [HMCL repository](https://github.com/HMCL-dev/HMCL)：用于核对完整启动链、历史版本和
  Loader 异常边界，不复制 GPLv3 实现。

所有网络 adapter 在实现时必须把实际使用的 endpoint、认证方式、缓存语义和响应 fixture
补充到独立协议文档，不能仅以第三方 Launcher 的代码行为作为规范。
