# Orbit CLI 命令规范

> 本文同时标明当前行为和仍有效的规范差距。数据格式见
> [orbit-toml-spec.md](orbit-toml-spec.md)，实现快照见
> [orbit-status.md](orbit-status.md)。

## 1. 全局上下文

实例选择优先级：

1. 显式 `-i <name>` / `--instance <name>`；
2. 当前目录含 `orbit.toml`；
3. `instances.toml` 中的全局默认实例；
4. 都没有时保留当前目录，由需要项目的命令返回缺少 manifest/lockfile。

只读命令可以静默使用全局默认实例。会修改实例的命令在从非项目目录回退到默认实例时
拒绝执行，要求显式 `--instance` 或进入项目目录。当前受保护的命令是：

```text
add install remove purge sync upgrade import
```

`init` 始终初始化当前目录；实例注册表和 cache 命令操作全局数据；`export` 读取实例但只
写用户指定的输出文件。

全局标志：

| 标志 | 说明 |
|------|------|
| `-i, --instance <name>` | 显式选择注册实例 |
| `-v, --verbose` | 显示实例选择等额外上下文 |
| `-q, --quiet` | 规范要求仅输出错误；当前只有部分上下文输出遵守，见 §8 |
| `-y, --yes` | 跳过确认；不会替缺失的可复现元数据猜值 |
| `--dry-run` | 返回操作预览，不写目标状态 |

正常结果写 stdout，错误、警告、搜索进度和交互提示写 stderr。

## 2. 初始化与实例

### `orbit init`

```text
orbit init <name>
  [--mc-version <version>]
  [--modloader fabric|forge|neoforge|quilt]
  [--modloader-version <version>]
```

行为：

1. 若 `orbit.toml` 已存在，在扫描、联网和写入前拒绝覆盖；
2. 从游戏 JAR 的 `version.json` 检测 Minecraft 版本；
3. 从 launcher version profile 的 Maven 坐标检测 loader 及版本；
4. 扫描 `mods/*.jar`，忽略 `.old` / `.disabled`，解析对应 loader 元数据与内嵌 JAR；
5. 计算 SHA-1/SHA-256/SHA-512，并批量向 Modrinth 做 SHA-512 来源识别；
6. 生成 manifest 与 Fat Lockfile，再做本地依赖图验证；
7. 将实例注册到全局 `instances.toml`。

无法识别平台来源的 JAR 作为 `provider = "file"` 写入 lockfile；manifest 始终只保存
`mod_id` 与版本约束。同一物理 JAR 中的其他逻辑模组只进入父 package 的 `bundled`。

显式参数优先于检测。交互模式在检测失败时请求输入；`--yes` 模式不读取 stdin，缺少
Minecraft、loader 或 loader 版本时要求显式参数，不静默选择 Fabric 或固定版本。

当前未实现 `.minecraft` 目录结构的单独预警；只要参数与目录内容足够，空目录也可用于
创建新项目。这属于规范体验差距，不影响生成数据的正确性。

### `orbit instances`

```text
orbit instances list
orbit instances default <name>
orbit instances remove <name>
```

- `list` 展示名称、路径、Minecraft、loader 以及当前/默认标记；
- `default` 保证只有一个默认实例，并同步 `config.toml` 的 `default_instance`；
- `remove` 只移除全局追踪，绝不删除实例目录；若移除默认实例，同时清除默认值。

## 3. 添加、还原与删除

### `orbit add`

```text
orbit add <mod>
  [--platform <provider>]
  [--version <constraint>]
  [--env client|server|both]
  [--optional]
  [--no-deps]
```

输入形式：

| 形式 | 行为 |
|------|------|
| `sodium` | 按 manifest 的 provider 顺序解析 |
| `mr:sodium` | 只用 Modrinth |
| `file:./mod.jar` | 解析并复制本地 JAR |
| `cf:jei` | 明确返回 CurseForge 暂不支持 |

在线流程先取得并验证候选 JAR，再以 JAR 的真实 `mod_id`、版本和 required dependencies
求解。确认后写入 `mods/`、manifest 和 lockfile。顶层 constraint、`optional`、`env`
持久化到 manifest；传递依赖只进入 lockfile。`--no-deps` 禁止传递安装。

本地 `file:` 同样解析 loader 元数据、哈希、内嵌模组并校验依赖图，不绕过锁文件。

### `orbit install`

```text
orbit install
  [--target client|server|both]
  [--group <name>]
  [--no-optional]
  [--locked | --frozen]
```

这是实例还原命令，不接受模组名，也不修改 manifest 顶级声明。

选择顺序：

1. 根据 target、group 和 optional 过滤 manifest 根依赖；
2. 保留已选根的传递依赖闭包；
3. 校验 manifest/lockfile 图；
4. 已存在且 SHA-256 正确的 JAR跳过；
5. 缺失 JAR 从缓存、本地 `file:` 或 provider 来源恢复；
6. 下载/复制后再次校验，并按需更新 lockfile。

`--locked` 与 `--frozen` 同义：要求 lockfile 与 manifest 完整一致，禁止重新解析来源
元数据。它不表示物理离线；缓存未命中时仍可使用 lockfile 已锁定的下载 URL。旧 lockfile
没有 URL 且缓存未命中时，locked 模式返回错误。

### `orbit remove`

```text
orbit remove <mod>
```

按 `mod_id` 或 Modrinth slug 查找顶层依赖。若仍有其它 package 依赖它则拒绝删除；
否则删除已校验的 JAR，并从 manifest/lockfile 移除条目。输入不匹配时，交互模式列出
可选依赖；`--yes` 要求精确标识，不进行猜测。dry-run 只报告计划。

### `orbit purge`

```text
orbit purge <mod>
```

先按 mod ID/slug 在 `config/` 下寻找归一化名称候选，逐项确认后执行 remove 和配置清理。
候选路径必须位于 config 根目录。`--yes` 选择全部候选；dry-run 展示全部但不删除。

## 4. 同步与更新

### `orbit sync`

扫描真实 `mods/` 并对账 manifest/lockfile，报告：

| 分类 | 含义 |
|------|------|
| `added` | 磁盘新增 JAR，已识别并写入声明/锁 |
| `changed` | 已锁文件内容或元数据变化，锁记录已更新 |
| `missing` | manifest/lockfile 期望的 JAR 不在磁盘 |
| `unlocked` | manifest 有顶层声明但 lockfile 无对应 package |

它不下载 JAR；为识别手动加入的文件，批量哈希反查可能访问 Modrinth。dry-run 不保存对账
结果。

### `orbit outdated [mod]`

只读查询在线 package 的最新兼容版本。可按真实 `mod_id` 或 slug 限定单包；不存在的
输入、未安装的包和 `file:` 包返回明确结果，不会静默当作“已是最新”。

### `orbit upgrade [mod]`

无参数时升级所有允许升级的在线 package；有参数时要求该包已经安装且有在线来源。
升级复用候选下载、真实 JAR 解析、PubGrub 诊断、确认与原子文件替换。manifest 中的版本
约束保持不变，只更新 lockfile 的实际版本与来源事实。dry-run 不替换文件。

## 5. 查询

```text
orbit search <query>
  [--platform <provider>] [--limit <n>]
  [--mc-version <version>] [--modloader <loader>]

orbit info <mod> [--platform <provider>]
orbit list [--tree] [--target client|server|both]
```

- `search` 合并已配置 provider 的结果并应用可选的 Minecraft/loader 过滤；
- `info` 按 provider 顺序查询详情；`mr:` / `cf:` 前缀可显式选择来源；
- `list` 从 lockfile 展示版本、provider、manifest env/optional；`--tree` 展示依赖，
  `--target` 过滤根并保留传递闭包。

当前在线查询只有 Modrinth。`cf:` 与 `--platform curseforge` 返回暂不支持，不回退到
Modrinth。

## 6. 导入、导出与检查

### `orbit import`

```text
orbit import <file>
  [--merge-strategy prefer-existing|prefer-import|interactive]
```

- `.toml`：合并 manifest 依赖，冲突按策略处理；
- `.zip`：只提取安全的 `mods/*.jar` 路径；
- `.mrpack`：先应用 bundled overrides，再按 index 从官方允许的 HTTPS 来源下载缺失
  JAR，并验证 file size、SHA-1 与 SHA-512；
- `--yes` 未指定策略时等同 `prefer-import`；
- dry-run 不写 manifest、JAR 或 lockfile。

ZIP 与 mrpack index 路径都经过规范化，绝对路径、`..` 与非 mods JAR 不会写入实例；
导入完成后统一触发 sync。

### `orbit export`

```text
orbit export [output] [--target client|server|both] [--format zip|mrpack]
```

导出 manifest、lockfile 与目标选择中校验通过的 JAR。未指定文件名时使用安全化的项目
名称和版本。`mrpack` 生成 Modrinth index；在线文件可成为 downloads，必须内嵌的本地
文件放入 overrides。dry-run 只统计计划。

### `orbit check`

```text
orbit check <mc-version> [--modloader <loader>]
```

对 lockfile 中在线 package 查询目标 Minecraft/loader 的兼容版本并返回逐包矩阵。本地
`file:` package 没有平台兼容性事实，会明确标为无法在线判断。

## 7. Cache

```text
orbit cache clean
```

先检查配置解析后的缓存目录、文件数和大小；空缓存直接成功。非 `--yes` 时确认，dry-run
不删除。core 会拒绝清理文件系统根、当前目录或 Orbit 数据目录本身。

当前命令清空整个 cache；配置中的自动淘汰策略和大小上限尚未执行。

## 8. 正确规范与剩余差距

以下不是“历史文档已过时”，而是仍正确但代码尚未完全遵守的 CLI 规范：

| 规范 | 当前差距 |
|------|----------|
| `--quiet` 只输出错误 | 多数 handler 仍直接 `println!`，只有实例上下文日志检查 quiet |
| `--verbose` 展示网络/解析细节 | 当前主要展示实例选择，没有统一结构化日志层 |
| 用户取消使用独立退出码 3 | clap 参数错误为 2、普通错误为 1；部分取消当前为成功或普通错误 |
| 全局运行配置控制网络/并发/UI | schema 已加载，但代理、重试、语言、样式和下载并发尚未全部接入 |
| 大规模 restore 有界并发 | 候选验证并发，最终 JAR 物化仍按确定顺序执行 |

已经实现、旧文档不应再标为缺失的内容：

- 全部命令 handler 已接入 core，不再是 `exit(2)` 占位；
- Forge、NeoForge、Quilt 检测和 JAR 解析；
- `file:` 添加、全量 restore、target/group/optional；
- list target、sync/check/purge、导入导出、实例与 cache；
- 默认实例的修改型命令安全阻断；
- 非交互 init 不猜 loader/版本，重复 init 不覆盖项目。

CurseForge 是单独的产品边界，继续保持暂不支持。
