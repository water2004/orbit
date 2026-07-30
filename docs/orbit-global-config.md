# Orbit 全局配置与运行路径

本文描述当前实现。项目级 `orbit.toml` 见
[orbit-toml-spec.md](orbit-toml-spec.md)。

## 1. 路径不是全局常量

core 不读取固定的 Windows 路径，也不在业务模块中调用 `APPDATA`、`HOME`、
`current_exe()`。入口先构造：

```rust
RuntimeContext::load(RuntimePathOptions {
    layout,
    config_file,
    cache_dir,
})
```

`RuntimeContext` 持有解析后的 `RuntimePaths`、`GlobalConfig` 和 `JarCache`，再显式传给
实例注册、provider factory、下载和 cache 命令。

调用方有三种选择：

- 直接传入 `config.toml` 的精确路径和 JAR cache 目录；
- 使用 `system` 布局；
- 使用 `executable` 布局。

CLI 对应全局参数：

```text
--config <file>
--cache-dir <directory>
--data-layout system|executable
```

精确路径不依赖宿主目录发现。若只显式传配置文件，`[cache].dir` 也可以提供缓存目录。
`instances.toml` 始终与实际使用的 `config.toml` 同目录。

## 2. 内置布局

### `system`

| 平台 | 配置与实例注册表 | JAR cache |
|---|---|---|
| Windows | `%APPDATA%\orbit\` | `%LOCALAPPDATA%\orbit\`，缺失时回退 `%APPDATA%\orbit\` |
| Linux | `$XDG_CONFIG_HOME/orbit/`，否则 `$HOME/.config/orbit/` | `$XDG_CACHE_HOME/orbit/`，否则 `$HOME/.cache/orbit/` |
| macOS | `$HOME/Library/Application Support/orbit/` | `$HOME/Library/Caches/orbit/` |

无法取得所需宿主目录时返回错误，并提示传显式路径或选择 executable 布局；不会静默写
当前工作目录。

### `executable`

```text
<executable-dir>/
  config.toml
  instances.toml
  cache/
```

构建 `orbit` 或 `orbit-core` 时启用 Cargo feature `portable`，只会把编译出的默认布局
改为 `executable`：

```bash
cargo build -p orbit --features portable
```

无该 feature 时默认 `system`。编译默认值不会禁止运行时用 `--data-layout` 或精确路径
覆盖。

## 3. 路径优先级

配置文件：

1. `--config` / `RuntimePathOptions.config_file`
2. 所选 layout 的默认 `config.toml`

缓存目录：

1. `--cache-dir` / `RuntimePathOptions.cache_dir`
2. 已加载配置的 `[cache].dir`
3. 所选 layout 的默认 cache

## 4. `config.toml` schema

```toml
[core]
default_instance = "survival" # 可省略
max_concurrent_downloads = 8
language = "system"

[network]
timeout = 30
max_retries = 3
# proxy = "http://127.0.0.1:7890"

[auth]
# curseforge_api_key = "..."
# modrinth_token = "..."

[cache]
# dir = "D:/OrbitCache"
capacity_mib = 5120

[ui]
color = "auto"
progress_bar = "modern"
```

### 命令式管理

通常不需要手工编辑 TOML。CLI 提供强类型配置入口：

```text
orbit config path
orbit config list
orbit config get <key>
orbit config set <key> <value>
orbit config unset <key>
```

键名使用面向 CLI 的连字符形式，例如：

```powershell
orbit config set cache.capacity-mib 2048
orbit config set network.timeout 60
orbit config set ui.progress-bar plain
orbit config unset network.proxy
```

`list` 和 `get` 显示文件层解析结果（文件中省略的必填字段显示 schema 默认值），不显示
环境变量覆盖后的有效值，防止进程环境中的 API Key 被意外回写或泄露。
`auth.curseforge-api-key` 与 `auth.modrinth-token` 即使已经保存也只显示
`<redacted>`；自动化输出同样脱敏。命令行本身可能进入 shell 历史，因此无人值守环境
仍优先使用对应环境变量。

`set` 在写盘前按字段类型和取值域验证；未知键、非法整数、空字符串和非法枚举值直接
报错。`unset` 清除可选字段；对 schema 必填字段则写回 schema 默认值。
`core.default-instance` 还会验证 `instances.toml` 并同步唯一的 `is_default` 标记，
不能指向不存在的实例。全局 `--dry-run` 对 `set`/`unset` 只验证和展示，不写文件。

配置修改只更新目标字段，保留其它字段、注释和排版，并以同目录临时文件原子替换。
`cache.capacity-mib` 在该命令结束时立即用于 LRU 清理；`cache.dir` 决定后续命令打开
哪个 cache，当前命令不会把已打开的 cache 中途换目录。若传入全局 `--config`，上述
命令操作的就是该精确文件。

`ui.progress_bar` 控制长事务的进度展示：

- `modern`：交互终端使用 spinner/进度条；stderr 被重定向时自动改为逐项文本；
- `plain`：始终输出稳定的文本阶段和完成计数；
- `off`：关闭进度事件展示；全局 `--quiet` 也会关闭。

`core.language` 接受 `system`、`en`、`zh-CN`。CLI 没有显式传
`--language` 时使用该值；显式参数优先于环境变量和文件。`ui.color` 接受：

- `auto`：只在对应 stdout/stderr 是交互终端时输出 ANSI 样式；
- `always`：即使重定向也保留 ANSI 样式；
- `never`：始终输出无样式文本。

颜色策略只作用于人类可读的 text 表格，不改变 JSON、NDJSON 或交互协议。

在线 add/fix/migrate/outdated/upgrade 会分别呈现 project 闭包发现、候选 JAR
下载/缓存校验/解析、离线求解和最终物化。候选 JAR 阶段有精确的 `已完成/总数`；
发现闭包无法预知远端递归总量，因此使用带已用时间的 spinner。多解枚举则把实际开始的
continuation run 和 maximality probe 作为工作单元：新分支出现时总量增长，完成时进度
推进，并同步显示 decision、propagation、backtrack、conflict 与已保留解计数。这个
动态计数用于证明阶段和活动状态，不预测剩余耗时；Pareto 或 co-Pareto front 本身仍可能
很大。

`orbit audit` 复用同一 UI 开关，但使用独立的审计强类型事件：按实际阶段和已知总量显示
classpath 准备、artifact、Mixin、Transformer 与冲突分析。plain 模式按比例节流，
不会逐个打印 artifact 文件名。

所有 provider 元数据请求和 artifact 下载统一消费 `network.timeout`、
`network.max_retries` 与 `network.proxy`。可重试范围是连接错误、HTTP 408、429 与 5xx；
非瞬态 4xx 不重试。`core.max_concurrent_downloads` 是同一次命令中所有 provider 共享的
artifact 下载许可数，不会因同时启用 Modrinth 和 CurseForge 而分别放大。
`auth.modrinth_token` 作为 Modrinth `Authorization` 请求头使用；CurseForge Key 则只
进入官方 API 和限定 HTTPS 域名的 CDN 下载请求。两种凭据都不会进入 lockfile。

首次加载不存在的配置文件时，Orbit 在解析出的路径创建默认文件。环境变量随后覆盖内存
值：

| 环境变量 | 字段 |
|---|---|
| `ORBIT_PROXY` | `network.proxy` |
| `ORBIT_TIMEOUT` | `network.timeout` |
| `ORBIT_RETRIES` | `network.max_retries` |
| `ORBIT_LANGUAGE` | `core.language` |
| `ORBIT_CURSEFORGE_API_KEY` | `auth.curseforge_api_key` |
| `ORBIT_MODRINTH_TOKEN` | `auth.modrinth_token` |

因此一般配置值优先级是：环境变量 > 文件 > schema 默认值。语言额外允许显式
`--language` 覆盖有效配置，所以它的完整优先级是：命令行 > `ORBIT_LANGUAGE` > 文件 >
`system`。路径参数在加载文件之前解析，不属于这个字段覆盖层。

## 5. JAR cache

cache 不信任 provider 文件名，也不使用“文件名 → 哈希”的可变全局索引。结构为：

```text
{cache_dir}/
  lru-index.json            # SHA-512 → 最近使用序号
  lru.lock                  # 跨进程维护锁
  jars/
    sha512/
      <locally-computed-sha512>
  aliases/
    sha1/
      <provider-sha1>        # 内容是对应的 SHA-512
```

写入时始终从实际 bytes 计算 SHA-1/SHA-512。统一 artifact 队列执行时，先按 provider
给出的强哈希查 cache；命中后再次校验并直接解析，不发 HTTP。未命中才调用
provider-owned downloader。

`capacity_mib` 是 JAR 内容的硬容量上限，不计算索引、锁和 SHA-1 别名的体积。每次
`get_bytes`、`copy_to` 和 `store_bytes` 都会 touch 对应 SHA-512 内容；CLI 在每一次
命令执行结束后（包括命令返回错误时）合并本次访问记录，按标准 LRU 从最久未使用的
JAR 开始删除，直到内容总量不超过上限，并同时删除指向已淘汰内容的 SHA-1 别名。
持久化顺序使用索引内的单调逻辑时钟，不依赖系统时间或文件系统 atime。LRU 索引通过
临时文件原子替换，维护过程以 cache 内的跨进程文件锁串行化。

`capacity_mib = 0` 表示一个命令内仍可复用刚下载的内容，但命令结束后不保留任何 JAR。
索引中没有访问记录的 content-addressed JAR 统一视为最旧条目；不存在第二套目录、
旧路径查询或按文件系统 atime 的兜底逻辑。

cache 配置只有 `dir` 和 `capacity_mib`。旧的 `enable`、`eviction_policy`、
`max_size_gb` 字段会作为未知字段直接报错，必须从配置中删除并改用整数 MiB 容量；
Orbit 不会静默猜测旧字段的含义。

`orbit cache clean` 使用同一个注入目录。core 拒绝递归删除文件系统根或当前工作
目录/其祖先，也拒绝删除包含 `config.toml` 或 `instances.toml` 的目录。

## 6. 平台抽象

`RuntimeEnvironment` 只暴露：

```rust
trait RuntimeEnvironment {
    fn executable_dir(&self) -> Result<PathBuf, OrbitError>;
    fn config_root(&self) -> Result<PathBuf, OrbitError>;
    fn cache_root(&self) -> Result<PathBuf, OrbitError>;
}
```

Windows、Linux、macOS 只实现目录发现。`RuntimePaths` 负责公共 `orbit/`、文件名和
layout 组装；配置、缓存、installer 和 resolver 不包含平台分支。测试通过 fake
environment 验证布局，不修改真实用户目录。

## 7. 生效范围

本 schema 中的字段均由运行路径消费：语言、颜色和进度只控制展示；网络、认证、下载
并发与缓存字段控制共享运行时服务。凭据不进入 lockfile、日志或错误正文。
