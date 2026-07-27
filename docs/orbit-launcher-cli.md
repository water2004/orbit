# Orbit Launcher CLI

> 实现状态：实例、配置、账户持久化、Mojang Java runtime，以及 Vanilla、Fabric、Quilt、Forge、
> NeoForge 客户端和独立服务端安装已可用；启动命令仍在后续提交中。
> 未实现的命令不会以空壳形式出现在 CLI 中。

`orbit-launcher` 与 Orbit 模组包管理器完全隔离。它使用自己的全局目录、实例注册表、
`orbit-launcher.toml` 和 `orbit-launcher.lock`，不读取或调用 Orbit。

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

# 已有实例
orbit-launcher [--instance <id|name>] install

# 一条命令创建并安装客户端或服务端
orbit-launcher install --new <name> [--root <path>] \
  --kind <client|server> --minecraft <exact|latest-release|latest-snapshot> \
  [--loader <vanilla|fabric|quilt|forge|neoforge>] [--loader-version <requirement>]

orbit-launcher instance create \
  --name <name> \
  [--root <path>] \
  --kind <client|server> \
  --minecraft <requirement> \
  [--loader <vanilla|fabric|quilt|forge|neoforge>] \
  [--loader-version <requirement>]

orbit-launcher instance import [--root <path>]
orbit-launcher instance list
orbit-launcher [--instance <id|name>] instance show
orbit-launcher [--instance <id|name>] instance rename <new-name>
orbit-launcher [--instance <id|name>] instance remove
orbit-launcher instance default set <id|name>
orbit-launcher instance default clear
orbit-launcher instance default show

orbit-launcher [--instance <id|name>] server eula show
orbit-launcher [--instance <id|name>] server eula accept <sha256>

orbit-launcher account login offline <profile-name>
orbit-launcher account login microsoft begin
orbit-launcher account login microsoft complete <login-session-id>
orbit-launcher account login yggdrasil \
  --provider <id> --username <login-name> [--profile <name-or-uuid>] [--password-stdin]
orbit-launcher account list
orbit-launcher account show [<account-id|profile-name|profile-uuid>]
orbit-launcher [--instance <id|name>] account select <account> [--global]
orbit-launcher [--instance <id|name>] account clear [--global]
orbit-launcher account logout <account>
```

配置键是稳定协议，目前包括网络并发数与超时、installer 超时、缓存上限、Java 默认来源、
Microsoft client ID，以及进度条和颜色偏好。`list`/`get` 会区分显式值与默认值；`unset` 删除显式值并恢复
默认值。修改经过强类型解析和完整配置校验后原子写入，同时保留已有 TOML 注释。External
Yggdrasil provider 属于复合对象，由 `config yggdrasil` 的强类型命令管理，不接受任意 TOML
路径写入。API root 默认必须使用 HTTPS；`--allow-insecure-http` 是会暴露账号密码与 token
的明确危险选择。

`accounts.json` 只保存 account ID、provider、角色 UUID/name 和时间等非秘密元数据。
Windows 的 token 由当前用户作用域 DPAPI 加密后原子落盘；Linux 桌面使用当前登录会话的
Freedesktop Secret Service。Secret Service 不存在或被锁定时命令直接报 `secret_store`，
不会回退到明文文件。Microsoft device code 和最终 refresh/access token 都只进入同一秘密
存储；普通 auth-session JSON 只有 user code、verification URI、轮询间隔和过期时间。

Microsoft 登录拆为 `begin`/`complete`，便于 CLI/GUI 跨进程恢复；`complete` 会轮询授权，
随后完成 Xbox Live、Minecraft ownership 与 profile 校验并保存可续期会话。External
Yggdrasil 密码只从安全 TTY 或显式 `--password-stdin` 读取，永不保存；保存的是 access/client
token，启动前按 `validate -> refresh -> interaction_required` 处理。一个账号有多个角色时
必须显式传 `--profile`。Offline 账号不产生秘密记录，也不会显示成已通过 Microsoft 验证。

`account select` 默认修改具体客户端实例的 `[launch].account`；`--global` 只修改全局缺省
账户且不能与 `--instance` 同用。服务端实例不使用客户端账户。`logout` 删除本地秘密与元
数据；若实例仍引用该 account ID，之后启动会明确报错，不会静默选择另一个账号。

`create` 和 `import` 中省略 `--root` 时只使用当前目录。相对 `--root` 相对当前目录解析，
注册表持久化规范化绝对路径，但路径不是实例身份。`remove` 只注销实例并保留全部文件。

非 Vanilla Loader 必须提供 `--loader-version`；Vanilla 禁止提供该参数。当前 `create` 只
建立用户意图和全局注册，不下载任何内容。一次命令创建并安装将由真实安装事务入口
`install --new` 提供，不会复用 `instance create` 伪装安装成功。bootstrap 失败时会注销临时
实例并删除 provisional manifest，不删除用户文件。

当前 `install` 接受 Vanilla/Fabric/Quilt/Forge/NeoForge client/server 实例。它先解析 Mojang version manifest v2、
目标版本 JSON 和该版本声明的 Java component/major，再一次性确定完整下载队列。客户端
严格执行 Mojang 的顺序规则、library/classifier、asset index、logging 配置与 native 解压
语义；相同 asset 内容按哈希下载一次，但会保留全部 legacy virtual/resources 逻辑映射。
文件按上游 SHA-1 校验后进入本地 SHA-256 CAS；下载可并发，runtime 和实例分别在 staging
中验证后原子提交。旧 lock 拥有但新精确状态不再需要的文件会在同一事务中移除；目标位置
已有但不属于旧 lock 的文件时拒绝覆盖。

Fabric 与 Quilt 都通过各自官方 Meta API 解析与目标 Minecraft 版本匹配的 profile，并将
Loader libraries、main class 和参数合并到同一个精确运行时模型。两者共享 profile 机制，
但版本选择规则不混用：Fabric 支持 `latest`、官方 `stable` 标记和精确版本；Quilt 支持
`latest` 和精确版本。Quilt Meta 没有 Loader stable 标记，因此 `stable` 会明确报错，不按
版本字符串猜测。缺少内联哈希的官方 Maven 条目必须取得 `.sha1` sidecar 后才进入队列。

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

## 全局路径

默认使用 Windows AppData 或 Linux XDG 目录。测试、便携运行和 GUI 可显式注入：

```text
--config-dir <path>
--data-dir <path>
--cache-dir <path>
```

实例注册表位于 data 目录的 `instances.toml`。配置、data 和 cache 路径彼此独立；业务模块
不直接读取 AppData、HOME 或 XDG 环境变量。

## JSON

所有当前命令支持 `--format json`。成功结果只写 stdout：

```json
{
  "schema_version": 1,
  "command": "instance.show",
  "ok": true,
  "result": {
    "id": "007f20b6-10a1-4746-8211-7b211b7285b3",
    "name": "main-server",
    "root": "D:\\minecraft\\main-server",
    "kind": "server",
    "is_default": false,
    "context": "explicit",
    "desired": {
      "minecraft": "1.21.1",
      "loader": "fabric",
      "loader_version": "stable",
      "java_policy": "auto"
    }
  }
}
```

错误只写 stderr，并提供稳定 `code`。GUI 不得依赖本地化 message 分支。

`--progress-format text|ndjson|none` 控制 stderr 进度，默认 `text`。安装进度来自真实阶段、
文件下载字节数、缓存命中、Java 物化计数、staging 校验和事务提交；NDJSON 使用
`schema_version/type/command/sequence/data.event` 稳定字段。最终 JSON 只写 stdout，进度与
错误只写 stderr。Microsoft `complete` 的轮询、Xbox、Minecraft 和安全持久化阶段也使用
同一 NDJSON envelope，且不会包含 device code 或任何 token。
