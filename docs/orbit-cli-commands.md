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

凡是命令会确定一个包集合，都遵守同一策略：求解包键是 JAR 的 `mod_id`；唯一极大解
自动选择，多个极大解必须选择（dry-run 也一样，只有 `--yes` 自动选稳定的第一个）。
选择之后，安装、升级、降级、同版本替换和删除合并成一个计划。只要计划会替换或删除
顶层 `mods/*.jar`，即使求解只有唯一方案也必须先展示精确包版本与文件名并确认。
contained JAR 不是独立删除目标。

全局标志：

| 标志 | 说明 |
|------|------|
| `-i, --instance <name>` | 显式选择注册实例 |
| `--config <file>` | 使用指定的全局配置文件；实例注册表位于其同目录 |
| `--cache-dir <directory>` | 使用指定的 content-addressed JAR 缓存目录 |
| `--data-layout system\|executable` | 选择平台目录或可执行文件相邻目录布局 |
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
2. 验证当前目录是实际游戏目录；空目录或只有 `mods/` 的任意目录会拒绝；
3. 从游戏 JAR 的 `version.json`、launcher version profile 或 Prism/MultiMC component
   检测 Minecraft 与 loader；定位并解析实际 Minecraft/loader JAR；
4. 扫描 `mods/*.jar`，忽略 `.old` / `.disabled`，解析对应 loader 元数据与内嵌 JAR；
5. 计算 SHA-1/SHA-256/SHA-512 和 CurseForge fingerprint；Modrinth 始终参与批量识别，
   已配置 API Key 时 CurseForge 也参与；
6. 同一 `mod_id` 的顶层 JAR 作为一个包的候选，经共享 PubGrub portfolio 选择；
7. 多解时请求方案选择；未选中的顶层包版本列入删除计划并在写盘前确认；
8. 将实际平台 JAR 的相对路径与 SHA-256 写入 manifest，生成 Fat Lockfile，再将实例
   注册到全局 `instances.toml`。

无法识别平台来源的 JAR 作为 `provider = "file"` 写入 lockfile；manifest 始终只保存
`mod_id` 与开放版本约束。同一顶层包 JAR 中的其他模块只进入父 package 的
`bundled`。若依赖图本身无解，init 保留所有文件、写出诊断，不猜测应该删除哪个包。

支持标准共享游戏根目录、`versions/<实例>` 隔离目录、Prism/MultiMC 的
`.minecraft`/`minecraft`、CurseForge profile 和 GDLauncher 的 `instance/`。
隔离布局只扫描当前实例，不读取 sibling profile；共享根目录出现多个 Minecraft 或
loader 候选时必须显式选择，不能按目录顺序猜测。

显式参数用于筛选实际候选，不能凭空创建平台工件。交互模式在多个候选时请求选择；
`--yes` 模式不读取 stdin，歧义或缺少实际 JAR 时要求显式参数/修复启动器安装。

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
| `cf:jei` | 只用 CurseForge；需要 API Key |
| `file:./mod.jar` | 解析并复制本地 JAR |

来源前缀与 `--platform` 同时出现时必须指向同一 provider；冲突会直接报错，不会把
`cf:` slug 交给 Modrinth 或反向处理。

在线流程先取得并验证候选 JAR，再以 JAR 的真实 `mod_id`、版本和 required dependencies
求解。确认后写入 `mods/`、manifest 和 lockfile。顶层 constraint、`optional`、`env`
持久化到 manifest；传递依赖只进入 lockfile。`--no-deps` 禁止传递安装。

若同一个 provider locator 的不同候选 JAR 声明了多个真实 `mod_id`，会分别剔除无解
身份。唯一可行身份自动采用；多个可行身份会先询问要添加哪一个包。upgrade 不允许借此
静默改名：它只跟随已安装 `mod_id`，项目改名必须作为 remove/add 的包替换。

本地 `file:` 同样解析 loader 元数据、哈希、内嵌模组并校验依赖图，不绕过锁文件。
在线与本地添加都使用同一个方案选择和包事务报告；若选中方案替换或淘汰已有顶层包
版本，会与新安装项一起展示并确认。

### `orbit install`

```text
orbit install
  [--target client|server|both]
  [--group <name>]
  [--no-optional]
  [--locked | --frozen]
```

这是实例还原命令，不接受模组名。开始前重新探测实际平台，不从 manifest 中记录的旧
文件名寻找 JAR。Minecraft 版本与 manifest 不一致时拒绝并要求先 `orbit sync`。
loader 版本不一致不直接拒绝：实际 loader JAR 的版本和 bundled 模块进入同一次求解，
只在真实依赖约束不兼容时失败；成功写盘时刷新平台快照。

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

非 locked `install` 是依赖修复入口：lock 图不完整或冲突时会下载远端完整候选闭包，
再按 JAR 元数据重新求解。`sync` 则只做本地对账，不承担联网修复。

### `orbit remove`

```text
orbit remove <mod>
```

按 `mod_id` 或平台 slug 查找顶层依赖。若仍有其它 package 依赖它则拒绝删除；
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

先从当前 launcher 布局重新探测平台，再扫描真实 `mods/` 并对账
manifest/lockfile。旧 `[project]` 版本和 `[platform]` 路径都只是用于生成变更报告，
不作为探测选择器；Minecraft 与 loader JAR 即使改名也按内容及 launcher 元数据重新
定位。报告：

| 分类 | 含义 |
|------|------|
| `platform` | Minecraft/loader 版本、JAR 路径或内容哈希发生变化 |
| `added` | 磁盘新增 JAR，已识别并写入声明/锁 |
| `changed` | 已锁文件内容或元数据变化，锁记录已更新 |
| `missing` | manifest/lockfile 期望的 JAR 不在磁盘 |
| `unlocked` | manifest 有顶层声明但 lockfile 无对应 package |
| `removed` | 同一 `mod_id` 下未被所选方案采用的顶层包版本 |

平台刷新和包求解使用同一次实际 loader JAR 分析。loader 版本变化不被先验判为错误；
若某个 mod 对新 loader 的真实约束不成立，正常返回依赖无解。

它不下载 JAR；为识别手动加入的文件，批量反查可能访问 Modrinth SHA-512 或 CurseForge fingerprint 接口。dry-run 不保存对账
结果。同 ID 的所有本地文件先作为候选统一求解；不会按扫描顺序让后一个覆盖前一个。
实际删除 `removed` 前总会展示文件名并确认。

### `orbit outdated [mod]`

只读查询在线 package 的最新兼容版本。可按真实 `mod_id` 或 slug 限定单包；不存在的
输入、未安装的包和 `file:` 包返回明确结果，不会静默当作“已是最新”。
若存在多个“其它包不变时已无法单独升级”的方案，只在交互模式列出升级集合并请求
选择；唯一方案自动采用。dry-run 仍需选择具体方案；`--yes` 才稳定选择第一个方案。

### `orbit upgrade [mod]`

无参数时升级所有允许升级的在线 package；有参数时要求该包已经安装且有在线来源。
升级复用候选下载、真实 JAR 解析、PubGrub 诊断、确认与原子文件替换。manifest 中的版本
约束保持不变，只更新 lockfile 的实际版本与来源事实。多解规则与 `outdated` 相同；
方案选择发生在安装确认之前。一个 upgrade 方案只要求至少一个包比当前安装版本更新，
允许为满足依赖而让其他包降级、同版本换源或被删除；这些变化全部列入同一个确认计划。
若没有任何包变新则 upgrade 是 no-op。dry-run 不替换文件。

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

在线查询支持 Modrinth 与 CurseForge。`cf:` 和 `--platform curseforge` 只选择
CurseForge，不回退到 Modrinth；缺少 API Key 或目标文件没有 API 下载 URL 时返回明确
错误。

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

### `orbit audit`

```text
orbit audit
  [--format text|json]
  [--min-severity low|medium|high|critical]
  [--fail-on low|medium|high|critical]
  [--mod <id-or-name>]
```

只读分析当前实例实际存在的 Minecraft、Loader、运行时依赖和 Mod JAR。它与
`orbit check` 不同：`check` 查询目标版本是否有远端文件，`audit` 不联网、不读取
provider 兼容声明，也不修改 manifest、lockfile、下载缓存或实例文件。

`--min-severity` 只控制报告展示；`--fail-on` 在完整分析后按等级决定是否返回非零退出
码。`--mod` 只匹配已安装 Mod 的 ID、展示名或文件名并过滤报告；没有匹配项会明确报错，
但有匹配项时分析仍加载完整实例。JSON 顶层固定包含
`schema_version`、`environment`、`readiness`、`artifacts`、`risks`、`coverage` 和
`warnings`。没有达到阈值的结果只表述为
“未发现达到当前阈值的字节码兼容风险”，不宣称全部 Mod 兼容。

执行前根据实际 classpath 进行 capability probe。Fabric/Quilt 需要 Loader 与 Mixin
ABI；现代 Forge/NeoForge 还必须具有可识别的 ModLauncher `ITransformer`、
`Target` 和 `ITransformationService` ABI。Legacy LaunchWrapper 明确拒绝，不提供
`--force`。单个坏 Mod、缺少 refmap、自定义 InjectionPoint 和解释预算耗尽进入
warning/coverage；缺失基础游戏或运行库则停止。

详细证据边界见 [orbit-bytecode-audit.md](orbit-bytecode-audit.md)。

## 7. Cache

```text
orbit cache clean
```

先检查配置解析后的缓存目录、文件数和大小；空缓存直接成功。非 `--yes` 时确认，dry-run
不删除。core 会拒绝清理文件系统根、当前目录/其祖先，或包含配置文件与实例注册表的
目录。

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

CurseForge 已接入 search/info/add/install/sync/check/outdated/upgrade/restore 的共享
路径。它仍受 Core API Key 和项目第三方下载许可约束；这些是外部服务边界，不会用
硬编码 ID 或猜测 CDN URL 规避。
