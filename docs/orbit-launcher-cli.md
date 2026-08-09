# Orbit Launcher CLI

> 实现状态：实例、配置、账户持久化、Mojang Java runtime，Vanilla、Fabric、Quilt、Forge、
> NeoForge 客户端/独立服务端安装，以及客户端启动和受管理的服务端运行均已可用。
> 未实现的命令不会以空壳形式出现在 CLI 中。

`orbit-launcher` 与 Orbit 模组包管理器保持单向隔离。它使用自己的全局目录、实例注册表、
`orbit-launcher.toml` 和 `orbit-launcher.lock`，不读取、链接或调用 Orbit，也没有包身份、
运行时数据归属或 purge 接口。需要按逻辑包记录运行时数据时，用户或 GUI 调用的是
`orbit launch`：Orbit 作为父进程向 Launcher 启动的 Java 注入 Agent。该包装不改变 Launcher
的任何命令、状态或独立可用性。

全局 `--language system|en|zh-CN` 控制 help、文本输出、进度、交互提示和结构化错误中的展示
文字，缺省 `system`。JSON/NDJSON 的 schema、字段、枚举码与错误码保持稳定，协议编码固定为
UTF-8，不依赖 Windows 当前代码页。

## 实例上下文

实例选择顺序固定为：

1. `--instance <stable-id|name>`；
2. 当前目录直接包含 `orbit-launcher.toml`；
3. 全局默认实例，但只允许只读命令；
4. 否则报 `instance_context_required`。

Launcher 不向父目录搜索。重命名、注销、安装、更新、启动和进程控制等敏感命令不得从
无关目录静默使用全局默认实例。GUI 应始终传稳定 ID。

## 当前命令

```text
orbit-launcher config path
orbit-launcher config list
orbit-launcher config get <key>
orbit-launcher config set <key> <value>
orbit-launcher config unset <key>
orbit-launcher config yggdrasil list
orbit-launcher config yggdrasil add <id> <api-root> [--allow-insecure-http]
orbit-launcher config yggdrasil remove <id>

orbit-launcher minecraft directory
orbit-launcher minecraft move <absolute-destination>

# 已有实例
orbit-launcher [--instance <id|name>] install

# 导出当前实例的可变游戏状态；可追加到已有 Orbit 投影
orbit-launcher [--instance <id|name>] export <state.orbitbundle> \
  [--base <existing.orbitbundle>]

# 一条命令创建并安装客户端或服务端
orbit-launcher install --new <name> [--server-directory <path>] \
  --kind <client|server> --minecraft <exact|latest-release|latest-snapshot> \
  [--loader <vanilla|fabric|quilt|forge|neoforge>] [--loader-version <requirement>] \
  [--from <package.orbitbundle|pack.mrpack>]

# 只读检查包结构并返回供 CLI/GUI 使用的结构化运行时要求
orbit-launcher package inspect <package.orbitbundle|pack.mrpack>

orbit-launcher instance create \
  --name <name> \
  [--server-directory <path>] \
  --kind <client|server> \
  --minecraft <requirement> \
  [--loader <vanilla|fabric|quilt|forge|neoforge>] \
  [--loader-version <requirement>]

orbit-launcher instance import --directory <path>
orbit-launcher instance list
orbit-launcher [--instance <id|name>] instance show
orbit-launcher [--instance <id|name>] instance rename <new-name>
orbit-launcher [--instance <id|name>] instance configure \
  [--minecraft <requirement>] \
  [--loader <vanilla|fabric|quilt|forge|neoforge>] \
  [--loader-version <requirement>]
orbit-launcher [--instance <id|name>] instance remove
orbit-launcher instance default set <id|name>
orbit-launcher instance default clear
orbit-launcher instance default show

orbit-launcher versions minecraft
orbit-launcher versions loader \
  --loader <fabric|quilt|forge|neoforge> --minecraft <exact-version>
orbit-launcher versions java --minecraft <exact-version>

orbit-launcher [--instance <id|name>] server eula show
orbit-launcher [--instance <id|name>] server eula accept <sha256>
orbit-launcher [--instance <id|name>] launch [--dry-run]
orbit-launcher [--instance <id|name>] server run [--dry-run]
orbit-launcher [--instance <id|name>] server start
orbit-launcher [--instance <id|name>] server status
orbit-launcher [--instance <id|name>] server command <command...>
orbit-launcher [--instance <id|name>] server stop

orbit-launcher account login offline <profile-name>
orbit-launcher account login microsoft begin
orbit-launcher account login microsoft complete <login-session-id>
orbit-launcher account login yggdrasil \
  --provider <id> --username <login-name> [--profile <name-or-uuid>] [--password-stdin]
orbit-launcher account list
orbit-launcher account show [<account-id|profile-name|profile-uuid>]
orbit-launcher account refresh <account-id|profile-name|profile-uuid>
orbit-launcher [--instance <id|name>] account select <account> [--global]
orbit-launcher [--instance <id|name>] account clear [--global]
orbit-launcher account logout <account>

orbit-launcher java list [--verify]
orbit-launcher java verify <runtime-id>
orbit-launcher java remove <runtime-id>
```

配置键是稳定协议，目前包括网络并发数与超时、installer 超时、缓存上限，以及进度条和
颜色偏好。Java 只使用 Minecraft 官方元数据指定的 Mojang 受管 runtime，不暴露最终必然
失败的 provider 选择。`list`/`get` 会区分显式值与默认值；`unset` 删除显式值并恢复
默认值。修改经过强类型解析和完整配置校验后原子写入，同时保留已有 TOML 注释。External
Yggdrasil provider 属于复合对象，由 `config yggdrasil` 的强类型命令管理，不接受任意 TOML
路径写入。`add` 接受站点地址或精确 API root：缺少协议时只补全 HTTPS，随后执行
authlib-injector 的 API Location Indication（`X-Authlib-Injector-API-Location`）服务发现，
验证标准 metadata，并且只持久化解析后的精确 API root。API root 默认必须使用 HTTPS；
`--allow-insecure-http` 是会暴露账号密码与 token 的明确危险选择。账户请求固定使用 API root
下的 `authserver/authenticate`、`authserver/refresh` 与 `authserver/validate`。

`accounts.json` 是 Launcher 管理的全局内部状态，不是实例配置，也不属于
`orbit-launcher.toml`。其 schema 3 只保存 account ID、provider、角色 UUID/name、认证状态、可选 HTTPS
皮肤纹理 URL 和时间等非秘密元数据。皮肤只用于展示，URL 无效或缺失不会改变账户身份；token、
密码和纹理内容都不进入该文件。格式尚未发布，因此不保留旧 schema 迁移路径；不匹配时明确
报错，不能伪装成空账户列表。
账户 JSON view 不把皮肤纹理 URL 当作可直接显示的头像。Launcher 根据 Minecraft 皮肤布局
裁取脸部 `(8,8)-(16,16)`，再叠加帽子层 `(40,8)-(48,16)`，按 64 像素逻辑宽度适配
64×32、64×64 和等比例高清皮肤，最后返回全局派生缓存中的 `avatar_path`。GUI 只显示该
本地路径；皮肤 URL 变化会产生新指纹路径，登出会删除对应派生头像。
Windows 的 token 由当前用户作用域 DPAPI 加密后原子落盘；Linux 桌面使用当前登录会话的
Freedesktop Secret Service。Secret Service 不存在或被锁定时命令直接报 `secret_store`，
不会回退到明文文件。Microsoft device code 和最终 refresh/access token 都只进入同一秘密
存储；普通 auth-session JSON 只有 user code、verification URI、轮询间隔和过期时间。

Microsoft 登录拆为 `begin`/`complete`，便于 CLI/GUI 跨进程恢复；`complete` 会轮询授权，
随后完成 Xbox Live、Minecraft ownership 与 profile 校验并保存可续期会话。External
Yggdrasil 密码只从安全 TTY 或显式 `--password-stdin` 读取，永不保存；保存的是 access/client
token，启动前按 `validate -> refresh -> interaction_required` 处理。Orbit 项目注册的 Microsoft
public-client ID 固定在 Launcher 中，不属于用户配置或秘密；refresh/access token 仍只进入上述
系统秘密存储。一个账号有多个角色时
必须显式传 `--profile`。Offline 账号不产生秘密记录，也不会显示成已通过 Microsoft 验证。
`account refresh` 显式执行同一会话续期路径，并更新公开角色名和皮肤 URL；GUI 的账户卡片
只调用该命令，不自行请求 Microsoft 或 Yggdrasil API。
只有 token 端点明确拒绝 refresh token、Yggdrasil 明确返回会话无效，或本地秘密缺失/损坏时，
账户才进入 `reauthentication-required`。超时、限流和服务端故障不会把账户误标为失效。重新
登录复用原 account ID 并原子恢复 `active`；登录开始失败不得写 `accounts.json`。

`account select` 默认修改具体客户端实例的 `[launch].account`；`--global` 只修改全局缺省
账户且不能与 `--instance` 同用。服务端实例不使用客户端账户。`logout` 删除本地秘密与元
数据；若实例仍引用该 account ID，之后启动会明确报错，不会静默选择另一个账号。

客户端 `create/install --new` 不接受任意 root：它们始终使用唯一托管 Minecraft 仓库，并把
game directory 建为 `<minecraft-directory>/instances/<name>`。该目录保存实例自己的
`minecraft.jar` 与 manifest/lock；`mods`、`config`、`saves` 在对应领域第一次真正写入时
按需产生。共享 `libraries`、`assets` 和
Loader 库留在仓库根。HMCL 同样把解析后的主游戏 JAR 定位到版本根内；Orbit Launcher 不生成
Mojang/HMCL profile，而由自己的 lock 直接记录实例 JAR 与共享 classpath。这是 Launcher 的
实例隔离策略，不是 Mojang 规定的实例描述格式。服务端使用 `--server-directory`；省略时使用
当前目录，方便 headless 部署。`instance import --directory` 必须指向现有实例的精确目录；
客户端目录必须是某个 `instances/` 的直接子目录，不接受扁平单版本目录兜底。注册表持久化规范化绝对路径，但路径
不是实例身份。`remove` 只注销实例并保留全部文件。
`instance configure` 原子修改现有 `orbit-launcher.toml` 的期望运行时，不下载、不修改 lock；
随后运行同一个 `install` 事务完成 Minecraft、loader 与 Java 更新。切换到非 Vanilla loader
时必须同时给出 loader requirement；切换到 Vanilla 会删除 loader requirement。
GUI 的 Loader 版本更新只允许在同一 Minecraft 与同一 Loader 类型内选择官方目录返回的精确
版本，并严格编排 `configure --loader-version -> install`。Minecraft/Loader 类型变化属于新实例
迁移，不原地改写实例；若实例已有 Orbit 状态，Launcher 成功后由 GUI 调用 `orbit sync` 重新
探测和记录平台，但 Launcher 本身仍不知道 Orbit。

GUI 与其他前端不得用自由文本或安装失败重试来猜版本。`versions minecraft` 直接返回 Mojang
version manifest v2 的完整有序目录、类型、发布时间及 latest 标记；选定精确 Minecraft 后，
`versions loader` 返回对应官方来源声明的全部兼容 Loader 版本与 latest/stable/recommended
标记。`versions java` 读取该 Minecraft 的官方 version JSON，返回必须自动下载的 Java
component/major。新建、更新和修复复用这些只读目录与同一个 `configure -> install` 事务。

非 Vanilla Loader 必须提供 `--loader-version`；Vanilla 禁止提供该参数。当前 `create` 只
建立用户意图和全局注册，不下载任何内容。一次命令创建并安装由真实安装事务入口
`install --new` 提供，不复用 `instance create` 伪装安装成功。bootstrap 失败时会注销临时
实例并删除 provisional manifest，不删除用户文件。

当前 `install` 接受 Vanilla/Fabric/Quilt/Forge/NeoForge client/server 实例。它先解析 Mojang version manifest v2、
目标版本 JSON 和该版本声明的 Java component/major，再一次性确定完整下载队列。客户端
严格执行 Mojang 的顺序规则、library/classifier、asset index、logging 配置与 native 选择
语义；安装只保存经校验的 native classifier JAR 及排除规则，启动准备阶段才重建实例自己的
`natives` 目录，不在安装阶段做无意义的全量解压。相同 asset 内容按哈希下载一次，但会保留
全部 legacy virtual/resources 逻辑映射。asset index 同样按已校验 SHA-1 命名；Mojang 的
`assetIndex.id` 仅记录为来源事实，不能造成不同索引内容争用同一个共享文件。
文件按上游 SHA-1 校验后进入本地 SHA-256 CAS；下载可并发，runtime 和实例分别在 staging
中验证后原子提交。旧 lock 拥有但新精确状态不再需要的文件会在同一事务中移除；目标位置
已有但不属于旧 lock 的文件时拒绝覆盖。实例写事务使用跨进程独占文件锁；进程崩溃留下的
有效 journal 会在下一次 `install` 取得锁后自动回滚，损坏或包含非规范化路径的 journal
会明确拒绝，不依赖不存在的手动 repair 命令。

Java 下载不是 GUI 或独立脚本的第二条实现：`install` 根据目标 Minecraft 官方 version JSON
中的 component/major 解析 Mojang Java runtime manifest，将全部文件加入统一下载队列，逐项
校验 SHA-1 后物化到 Launcher data `runtimes/<runtime-id>`，并在实例 lock 中记录精确 runtime。
`java list` 查看已安装版本、平台、路径、文件数与大小；`--verify` / `java verify` 重新校验
完整 inventory。`java remove` 只允许删除没有被任何已注册实例 lock 引用的 runtime，且只删除
经过目录边界校验的单个 runtime 目录。`runtimes/.staging` 是安装事务的内部工作区，不属于
runtime inventory，也不得被 `list`、`verify` 或 `remove` 枚举为已安装 Java。

Fabric 与 Quilt 都通过各自官方 Meta API 解析与目标 Minecraft 版本匹配的 profile，并将
Loader libraries、main class 和参数合并到同一个精确运行时模型。两者共享 profile 机制，
但版本选择规则不混用：Fabric 支持 `latest`、官方 `stable` 标记和精确版本；Quilt 支持
`latest` 和精确版本。Quilt Meta 没有 Loader stable 标记，因此 `stable` 会明确报错，不按
版本字符串猜测。缺少内联哈希的官方 Maven 条目必须取得 `.sha1` sidecar 后才进入队列。
profile 只作为经校验的解析输入，完整 runtime classpath、入口和参数进入
`orbit-launcher.lock`；实例目录不再复制 profile 或维护第二套可启动描述。需要审计来源时，
lock 保留 profile URL 与 SHA-256，原始响应由元数据缓存负责。

## 实例状态导出与恢复

`orbit-launcher export <state.orbitbundle>` 只写所选实例的 Launcher 投影，不接收 Minecraft、
Loader 或目标实例参数，也不做兼容性推断。客户端包含 `options.txt`、`servers.dat` 和隔离
game directory 下的 `saves/`；独立服务端包含 `server.properties`、白名单/管理员/封禁列表、
服务端图标，以及 `server.properties` 的 `level-name` 指向的世界目录（缺省 `world`）。凭据、
EULA 接受、日志、缓存和 Minecraft/Loader/Java artifact 都不进入该投影。包内每个文件
有独立 SHA-256，路径、符号链接和实例目录边界在写入前验证。`--base` 要求运行时和端侧匹配，
保留并验证已有 Orbit 投影后原子写出同一路径；Launcher 不解析 Orbit 文件。

包只能由 `install --new ... --from <package>` 消费，不能恢复到已有实例；目标实例目录
必须尚不存在。对服务端恢复因此必须用 `--server-directory` 指向一个新路径，不能把当前已有
目录当作目标。安装器先按目标实例 TOML 和官方元数据完成目标 Minecraft、Loader、Java 与
默认服务端设置，再校验并应用 Launcher 投影；同版本恢复和跨版本迁移没有第二条路径。任一步失败都
注销 provisional 实例并删除该新目录。安装不会删除源包，因为同一个包还可能包含需要由 Orbit
消费的独立投影。
client/server 类型不一致直接报错。`.mrpack` 的 Minecraft/Loader dependencies 是精确要求；
自有包省略安装参数时使用包内运行时，显式参数则可为跨版本迁移选择新的目标运行时。
`package inspect` 只读检查包的 schema、路径/inventory 和运行时元数据，并返回名称、版本、可用端侧、精确运行时要求、Launcher 状态是否
存在及逐项 optional 文件，不创建目录。GUI 先调用它构造原生安装表单，再调用同一个
`install --from`，不会自行解析包。

服务端的 `server.properties` 不会跨版本整文件覆盖。目标 Minecraft 在正常目标安装事务中
通过自己的 `--initSettings` 生成目标版本字段集合；运行时安装完成后，仅把源包中同名字段的值合并
进去，保留目标新增字段及其默认值，并在结果中结构化列出目标已不存在而被跳过的源字段。
世界内容恢复到合并后的目标 `level-name`。Launcher 不硬编码字段表，也不会迁移 `eula.txt`；
目标实例仍必须单独展示并接受当前 Minecraft EULA。

Forge 与 NeoForge 从各自官方版本索引和 Maven 仓库解析精确 installer。Forge 的 `stable`
对应官方 `recommended` promotion，`latest` 对应官方 `latest` promotion；NeoForge 的
`stable` 只选择没有预发布后缀的兼容版本。installer 必须先通过官方 `.sha256` sidecar
校验，并确认 JAR 内 `install_profile.json` 的 schema 与 Minecraft 版本。它随后只在安装
事务的 staging 中由已安装的受管 Java 执行，并服从 `installer.timeout-seconds`（默认
1800 秒）。Launcher 不执行 installer 生成的 `run.bat`/`run.sh`，而是检查客户端 profile
或服务端平台 argfile，并将每个保留产物的 SHA-256 与 installer 来源写入 lock。

服务端安装每次都会获取当前官方 EULA。文本 TTY 会完整展示正文、官方 URL 和正文
SHA-256，并且只有用户准确输入 `I AGREE` 才继续；没有 `--yes` 绕过。JSON、管道或
`--non-interactive` 必须先对已有实例执行 `server eula show`，再用返回的 digest 执行
`server eula accept`。接受记录与实际展示正文绑定；正文 digest 变化后必须重新接受。

## 启动与服务端管理

`launch` 和 `server run` 在启动前逐项校验 lock、受管 Java 和所有 Launcher 拥有的运行时
文件；不会联网更新，也不会根据目录内容猜测替代 JAR。`--dry-run` 输出已经脱敏的精确命令，
账户 token 不进入 stdout、stderr 或错误信息。客户端使用实例账户；服务端不读取客户端账户。

`server run` 在前台运行 supervisor，直接输入 Minecraft 控制台命令即可发送到服务端；输入
`stop` 或按 Ctrl+C 会发送 `stop`、等待 `graceful_stop_timeout_seconds`，超时才强制终止，
随后最多再等待 `kill_timeout_seconds`。`server start` 启动同一套 supervisor 的后台形式，
其标准输出和结构化进度分别写入实例的
`.orbit-launcher/supervisor.stdout.log` 与 `.orbit-launcher/supervisor.stderr.log`。

后台 supervisor 使用实例 UUID 作为身份，并通过当前用户可访问的 Windows Named Pipe 或
权限为 `0600` 的 Unix Domain Socket 提供 `status`、`command` 和 `stop`。同一实例由文件锁
保证至多有一个 supervisor；状态查询不扫描进程，也不根据旧 PID 猜测。它只承诺在当前登录
会话内运行，不注册 Windows Service 或 systemd unit，也不提供开机启动和退出登录后保活。

服务端实例 TOML 的默认策略如下：

```toml
[server]
restart = "on-unexpected-exit"
restart_limit = 5
restart_window_seconds = 600
restart_backoff_max_seconds = 60
graceful_stop_timeout_seconds = 30
kill_timeout_seconds = 10
```

只有用户 `server stop`、前台 Ctrl+C 或通过受管通道输入 `stop` 才属于预期退出。其他自然退出
即使退出码为 0，也按异常退出处理；supervisor 使用上限受控的指数退避，并在固定窗口达到
重启次数上限后停止。所有 generation、exit、backoff、restart 和 limit 事件都可用 NDJSON
消费。`restart = "never"` 禁用自动重启；显式停服在任何策略下都不会重启。

服务端 External Yggdrasil 只使用实例中明确配置的 provider，并在安装时锁定经官方元数据、
SHA-256 和 JAR Manifest 验证的 Authlib Injector。客户端选择 External Yggdrasil 账户时也
使用同一受管 agent。未配置、lock 不一致或 agent 校验失败都会直接报错，不猜测或降级。

## 全局路径

默认使用 Windows AppData 或 Linux XDG 目录。测试、便携运行和 GUI 可显式注入：

```text
--config-dir <path>
--data-dir <path>
--cache-dir <path>
```

实例注册表位于 data 目录的 `instances.toml`。配置、data 和 cache 路径彼此独立；业务模块
不直接读取 AppData、HOME 或 XDG 环境变量。

客户端仓库缺省为 Launcher data 目录下的 `minecraft`：Windows 位于当前用户 AppData，Linux
遵循 XDG data，macOS 使用 Application Support。用 `minecraft directory` 查看准确路径；
`minecraft move <absolute-destination>` 迁移完整仓库并原子改写所有客户端注册位置。同卷使用
rename，跨卷逐文件复制并用 SHA-256 验证后才切换注册表与配置；服务器目录不会随之移动。

## JSON

所有当前命令支持 `--output-format json`。成功结果只写 stdout：

```json
{
  "schema_version": 2,
  "command": "instance.show",
  "ok": true,
  "result": {
    "id": "007f20b6-10a1-4746-8211-7b211b7285b3",
    "name": "main-server",
    "root": "D:\\minecraft\\main-server",
    "kind": "server",
    "is_default": false,
    "context": "explicit",
    "selected_account_id": "ebc2d5d2-4e8b-4e52-a119-c459c971b7ff",
    "desired": {
      "minecraft": "1.21.1",
      "loader": "fabric",
      "loader_version": "stable"
    },
    "installed": {
      "minecraft": "1.21.1",
      "loader": "fabric",
      "loader_version": "0.16.14",
      "java": {
        "provider": "mojang",
        "version": "21.0.3",
        "major": 21,
        "platform": "windows-x64"
      }
    }
  }
}
```

错误只写 stderr，并提供稳定 `code`。GUI 不得依赖本地化 message 分支。

`--progress-format text|ndjson|none` 控制 stderr 进度，默认 `text`。安装进度来自真实阶段、
文件下载字节数、缓存命中、Java 物化计数、staging 校验和事务提交；NDJSON 使用
`schema_version/type/command/sequence/phase/data.event` 稳定字段。成功、错误和进度均直接
使用与 Orbit 本体共享的 `orbit-machine-protocol` schema 2，不存在 Launcher/GUI 专用信封。
最终 JSON 只写 stdout，进度与
错误只写 stderr。Microsoft `complete` 的轮询、Xbox、Minecraft 和安全持久化阶段也使用
同一 NDJSON envelope，且不会包含 device code 或任何 token。
