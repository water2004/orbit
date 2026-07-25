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
language = "en"

[network]
timeout = 30
max_retries = 3
# proxy = "http://127.0.0.1:7890"

[auth]
# curseforge_api_key = "..."
# modrinth_token = "..."

[cache]
enable = true
# dir = "D:/OrbitCache"
eviction_policy = "size"
max_size_gb = 5.0

[ui]
color = "auto"
progress_bar = "modern"
```

`ui.progress_bar` 控制长事务的进度展示：

- `modern`：交互终端使用 spinner/进度条；stderr 被重定向时自动改为逐项文本；
- `plain`：始终输出稳定的文本阶段和完成计数；
- `off`：关闭进度事件展示；全局 `--quiet` 也会关闭。

在线 add/install/check/outdated/upgrade 会分别呈现 project 闭包发现、候选 JAR
下载/缓存校验/解析、离线求解和最终物化。候选 JAR 阶段有精确的 `已完成/总数`；
发现闭包无法预知远端递归总量，因此使用带已用时间的 spinner。多解枚举则把实际开始的
continuation run 和 maximality probe 作为工作单元：新分支出现时总量增长，完成时进度
推进，并同步显示 decision、propagation、backtrack、conflict 与已保留解计数。这个
动态计数用于证明阶段和活动状态，不预测剩余耗时；完整局部极大解枚举仍可能指数级。

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

因此配置值优先级是：环境变量 > 文件 > schema 默认值。路径参数在加载文件之前解析，不
属于这个字段覆盖层。

## 5. JAR cache

cache 不信任 provider 文件名，也不使用“文件名 → 哈希”的可变全局索引。结构为：

```text
{cache_dir}/
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

## 7. 当前尚未接入的正确规范

以下字段已正确加载和保存，但运行路径尚未完整消费；这是实现差距，不是废弃 schema：

- `core.max_concurrent_downloads`：统一候选下载当前使用固定有界并发；
- `network.proxy`、timeout、retry：各 HTTP client 尚未统一由配置构造；
- `auth.modrinth_token`：尚未传入 Modrinth client；
- `cache.enable`、自动淘汰策略和大小上限：尚未执行；
- `language` 与 `ui.color`：CLI 本地化和颜色策略尚未接入；`ui.progress_bar` 已接入。

CurseForge API Key 已真实接入 provider 创建、Core API 与受限 CDN 下载。Key 不进入
lockfile、日志或错误正文。
