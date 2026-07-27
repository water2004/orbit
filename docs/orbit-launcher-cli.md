# Orbit Launcher CLI

> 实现状态：实例、配置、Mojang Java runtime 以及 Vanilla 客户端/独立服务端安装已可用；
> Fabric/Quilt/Forge/NeoForge adapter、账户与启动命令仍在后续提交中。
> 未实现的命令不会以空壳形式出现在 CLI 中。

`orbit-launcher` 与 Orbit 模组包管理器完全隔离。它使用自己的全局目录、实例注册表、
`orbit-launcher.toml` 和未来的 `orbit-launcher.lock`，不读取或调用 Orbit。

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

# 已有实例
orbit-launcher [--instance <id|name>] install

# 一条命令创建并安装 Vanilla 客户端或服务端
orbit-launcher install --new <name> [--root <path>] \
  --kind <client|server> --minecraft <exact|latest-release|latest-snapshot>

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
```

配置键是稳定协议，目前包括网络并发数与超时、缓存上限、Java 默认来源、Microsoft client
ID，以及进度条和颜色偏好。`list`/`get` 会区分显式值与默认值；`unset` 删除显式值并恢复
默认值。修改经过强类型解析和完整配置校验后原子写入，同时保留已有 TOML 注释。External
Yggdrasil provider 属于复合对象，后续由账户领域命令管理，不接受任意 TOML 路径写入。

`create` 和 `import` 中省略 `--root` 时只使用当前目录。相对 `--root` 相对当前目录解析，
注册表持久化规范化绝对路径，但路径不是实例身份。`remove` 只注销实例并保留全部文件。

非 Vanilla Loader 必须提供 `--loader-version`；Vanilla 禁止提供该参数。当前 `create` 只
建立用户意图和全局注册，不下载任何内容。一次命令创建并安装将由真实安装事务入口
`install --new` 提供，不会复用 `instance create` 伪装安装成功。bootstrap 失败时会注销临时
实例并删除 provisional manifest，不删除用户文件。

当前 `install` 接受 Vanilla client/server 实例。它先解析 Mojang version manifest v2、
目标版本 JSON 和该版本声明的 Java component/major，再一次性确定完整下载队列。客户端
严格执行 Mojang 的顺序规则、library/classifier、asset index、logging 配置与 native 解压
语义；相同 asset 内容按哈希下载一次，但会保留全部 legacy virtual/resources 逻辑映射。
文件按上游 SHA-1 校验后进入本地 SHA-256 CAS；下载可并发，runtime 和实例分别在 staging
中验证后原子提交。旧 lock 拥有但新精确状态不再需要的文件会在同一事务中移除；目标位置
已有但不属于旧 lock 的文件时拒绝覆盖。

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
错误只写 stderr。
