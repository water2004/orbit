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
add install fix remove purge sync upgrade import migrate-export remote-add remote-remove
```

`init` 始终初始化当前目录；实例注册表和 cache 命令操作全局数据；`export` 读取实例但只
写用户指定的输出文件。

凡是求解型命令会确定一个包集合，求解包键都是 JAR 的 `mod_id`，但优化目标按操作语义区分：

- `add` / `fix` 枚举相对当前 lock 的标准 Pareto 极小变更集合；
- `upgrade` / `outdated` 枚举标准版本 Pareto 极大集合；
- 迁移针对空的目标安装状态枚举目标版本 Pareto 极大集合。

唯一方案自动选择，多个互不支配方案必须明确选择（dry-run 也一样，`--yes` 也不能替用户
选择）。
选择之后，安装、升级、降级、同版本替换和删除合并成一个计划。只要计划会替换或删除
顶层 `mods/*.jar`，即使求解只有唯一方案也必须先展示精确逻辑包版本动作并确认。物理
JAR 路径是执行层事实，永不进入 UI；多方案选择额外显示每个候选的顶层 JAR basename，
用于区分同版本的不同真实候选。普通升级预览、诊断和删除确认仍不显示文件名。
contained JAR 不是独立删除目标。

全局标志：

| 标志 | 说明 |
|------|------|
| `-i, --instance <name>` | 显式选择注册实例 |
| `--config <file>` | 使用指定的全局配置文件；实例注册表位于其同目录 |
| `--cache-dir <directory>` | 使用指定的 content-addressed JAR 缓存目录 |
| `--data-layout system\|executable` | 选择平台目录或可执行文件相邻目录布局 |
| `--output-format text\|json` | 输出格式；`json` 输出单个 JSON 文档到 stdout，供自动化工具集成；与 `export --format zip\|mrpack` 无歧义 |
| `--progress-format none\|ndjson` | 进度协议；`ndjson` 把进度事件逐行写 stderr，每行一个 JSON 对象 |
| `-v, --verbose` | 显示实例选择等额外上下文 |
| `-q, --quiet` | 规范要求仅输出错误；当前只有部分上下文输出遵守，见 §8 |
| `-y, --yes` | 只跳过写入前确认；不会替用户选择多个包身份、搜索结果或 Pareto 解，也不会猜测缺失的可复现元数据 |
| `--dry-run` | 返回操作预览，不写目标状态 |

正常结果写 stdout，错误、警告、结构化操作进度和交互提示写 stderr。`--output-format json` 下
stdout 始终是且只是一个完整 JSON 文档（成功为结果，失败为空），进度（若启用）走 stderr
NDJSON，调用方可以安全 `orbit --output-format json ... | jq`。JSON 结果、NDJSON 进度、结构化
错误的 schema 见 [orbit-output-formats.md](orbit-output-formats.md)。交互终端默认显示
spinner/进度条；重定向时显示稳定文本。`config.toml` 的 `ui.progress_bar` 可设为
`modern`、`plain` 或 `off`，`--quiet` 始终关闭进度。

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
3. 从游戏 JAR 的 `version.json`、launcher version profile、Prism/MultiMC component
   或 dedicated-server 官方 launch spec 检测 Minecraft 与 loader；定位并解析实际
   Minecraft/loader/runtime JAR；
4. 扫描 `mods/*.jar`，忽略 `.old` / `.disabled`，解析对应 loader 元数据与内嵌 JAR；
5. 计算 SHA-1/SHA-256/SHA-512 和 CurseForge fingerprint；Modrinth 始终参与批量识别，
   已配置 API Key 时 CurseForge 也参与；
6. 同一 `mod_id` 的顶层 JAR 作为同一逻辑包的多个本地实现；无法在线识别的本地源复制到
   `.orbit/sources/`，以便后续精确恢复；
7. 不运行候选求解，也不删除或替用户选择重复实现；
8. 没有重复实现时按本地事实生成 Fat Lockfile；存在重复时保留全部文件和 TOML source，
   不创建含糊 lock，并明确要求 `orbit fix`；
9. 将实际平台 JAR 的相对路径与 SHA-256 写入 manifest，再将实例注册到全局
   `instances.toml`。

无法识别平台来源的 JAR 以 `file` remote 写入 manifest；有解时也作为所选 package 的
精确 lock 来源。每个受管逻辑包都保存至少一个候选远端。同一顶层包 JAR 中的其他模块只进入
父 package 的 `bundled`。若依赖图本身无解，init 保留所有文件和全部 manifest remotes，
写出空 lock 与诊断；没有选中方案时不会把冲突候选伪装成已锁定包。

支持标准共享游戏根目录、`versions/<实例>` 隔离目录、Prism/MultiMC 的
`.minecraft`/`minecraft`、CurseForge profile、GDLauncher 的 `instance/`，以及
Fabric、Quilt、现代 Forge/NeoForge 安装器生成的 dedicated server 根目录。
隔离布局只扫描当前实例，不读取 sibling profile；共享根目录出现多个 Minecraft 或
loader 候选时必须显式选择，不能按目录顺序猜测。

`mods/` 不存在表示空模组集合，不是损坏，也不会由 init 补建。只有 `mods` 这个路径已经存在
但不是目录时才报错。这个语义对 init、sync、list、outdated、audit 和迁移检查一致。

Dedicated server 的 `eula.txt` / `server.properties` 优先于通用 `versions/` 布局。
Orbit 读取 Fabric/Quilt launch JAR、Forge bootstrap shim 或当前平台
`unix_args.txt` / `win_args.txt`，但不执行启动脚本。缺少实际运行时文件、清单 hash
不匹配或存在多个不同安装时直接报错。

显式参数用于筛选实际候选，不能凭空创建平台工件。交互模式在多个候选时请求选择；
`--yes` 模式不读取 stdin，歧义或缺少实际 JAR 时要求显式参数/修复启动器安装。

### `orbit instances`

```text
orbit instances list
orbit instances register <name> <path>
orbit instances default <name>
orbit instances remove <name>
```

- `list` 展示名称、路径、Minecraft、loader 以及当前/默认标记，输出为统一自适应表格；
- `register` 只接管一个已经同时具有有效 `orbit.toml` 与 `orbit.lock` 的工作区；名称和路径必须
  显式提供，两份文件的平台元数据必须完全一致。它不执行探测、补全、sync 或任何兜底；
- `default` 保证只有一个默认实例，并同步 `config.toml` 的 `default_instance`；
- `remove` 只移除全局追踪，绝不删除实例目录；若移除默认实例，同时清除默认值。

### `orbit config`

```text
orbit config path
orbit config list
orbit config get <key>
orbit config set <key> <value>
orbit config unset <key>
```

配置键是固定、强类型的公开接口，不接受任意 TOML path。公开键使用连字符，例如
`cache.capacity-mib`、`core.max-concurrent-downloads` 和
`auth.curseforge-api-key`；`orbit config list` 展示完整集合、类型与文件层解析值
（包含 schema 默认值）。
密钥始终显示为 `<redacted>`。`set`/`unset` 原子更新单一字段且保留其它注释和排版；
它们遵守全局 `--config` 与 `--dry-run`。`core.default-instance` 与
`instances.toml` 的唯一默认标记作为一个领域操作同步维护。

这里展示的是持久化层，不叠加环境变量。正常业务命令仍遵守“环境变量 > 文件 >
schema 默认值”的有效配置优先级。完整键表、取值约束和路径规则见
[orbit-global-config.md](orbit-global-config.md)。

## 3. 添加、还原与删除

### `orbit add`

```text
orbit add <mod>
  [--platform <provider>]
  [--version <constraint>]
  [--env client|server|both]
  [--optional]
```

输入形式：

| 形式 | 行为 |
|------|------|
| `sodium` | 先按 `[resolver].catalogs` 尝试作为 project locator；未找到时搜索并让用户选择 |
| `mr:<project-id-or-search>` | 只用 Modrinth；持久化时规范为 project ID |
| `cf:<numeric-project-id>` | 只用 CurseForge；需要 API Key |
| `file:./mod.jar` | 解析并复制本地 JAR |

来源前缀与 `--platform` 同时出现时必须指向同一 provider；冲突会直接报错，不会把
`cf:` locator 交给 Modrinth 或反向处理。CurseForge 的持久远端只接受数值 project ID。

在线流程先取得并验证候选 JAR，再以 JAR 的真实 `mod_id`、版本和 required dependencies
求解。确认后写入 `mods/`、manifest 和 lockfile。请求包的 constraint、`optional` 和显式
传入的 `env` 持久化到 manifest；未传 `--env` 时保持自动状态。此次选择中的其他顶层
逻辑包也各自写入 `[packages]`，默认版本策略为 `*`。TOML 不区分根包与传递包，所有实际
包都能独立配置远端、环境和版本策略。

add 为新请求包声明设置完整字符串集合默认值 `all; intersect not contains(i"beta"); intersect
not contains(i"snapshot")`。它只影响新建条目；已存在的包和此次补入的其它包不会被默认规则
覆盖。该规则是用户策略，可通过 `constraint set --string` 修改。

`add` 以当前 lock 为基线枚举标准 Pareto 极小变更集合：如果方案 A 改动的已有逻辑包集合
是方案 B 的真子集，B 不会返回。这不是“改动数最少”；例如只能改 A 或只能改 B 的两个
方案互不支配，仍都必须交给用户选择。请求添加的包是所有可行方案的强制目标，不计入基线
偏好；未受管候选默认偏好保持不存在。因此不会为了选择更高的新包版本而顺便升级现有包，
也不会引入本可避免的新依赖。固定某个极小变更集合后，才在其中保留版本 Pareto 极大候选。

该流程会分别显示：递归发现 project、候选队列总数、JAR 下载/缓存校验/解析完成数、
离线求解的动态工作量，以及确认后的包物化进度。求解总量会在发现新的 continuation
run、preference probe 或 maximality probe 时增加，完成数随后推进，同时显示决策、传播、回溯、冲突和解
数量；这样可以区分网络阶段与仍在探索新投影的求解阶段。该动态总量不是剩余耗时上界；
Pareto 或 co-Pareto front 本身仍可能很大。

若同一个 provider locator 的不同候选 JAR 声明了多个真实 `mod_id`，会分别剔除无解
身份。唯一可行身份自动采用；多个可行身份会先询问要添加哪一个包。upgrade 不允许借此
静默改名：它只跟随已安装 `mod_id`，项目改名必须作为 remove/add 的包替换。

本地 `file:` 同样解析 loader 元数据、哈希、内嵌模组并校验依赖图，不绕过锁文件。
在线与本地添加都使用同一个方案选择和包事务报告；若选中方案替换或淘汰已有顶层包
版本，会与新安装项一起展示并确认。

### `orbit env`

```text
orbit env <package> <client|server|both|auto>
```

修改 `orbit.toml` 中一个受管包的环境过滤。`client`、`server` 和 `both` 是显式用户
策略；`auto` 删除显式覆盖，重新跟随 lock 中精确选中 JAR 的 `environment`。该命令只
接受 JAR 声明的 `mod_id`，不修改 lock，也不重新求解或下载。支持全局 `--dry-run` 和
`--output-format json`。

### `orbit versions` 与 `orbit constraint`

```text
orbit versions <package>
orbit constraint show <package>
orbit constraint set <package> any
orbit constraint set <package> exact <version>
orbit constraint set <package> <greater-than|at-least|less-than|at-most> <version>
orbit constraint set <package> range <lower> <upper> \
  [--lower-bound inclusive|exclusive] [--upper-bound inclusive|exclusive]
  [--string '<ordered-set-rule>']
```

`versions` 从该包在 TOML 中配置的全部远端联网枚举当前 Minecraft/Loader 工件，先进入
统一下载队列并按全局 cache 去重，再从 JAR 读取真实 `mod_id`、版本和依赖。输出按数值核心
降序排列；相同数字核心的不同完整版本或相同版本的不同内容候选分别列出。文本和 GUI 不显示
内容哈希；JSON 也只返回可展示的版本、来源和 JAR 详情。

候选同时报告数字核心是否可过滤。Fabric/Quilt 退化为不透明 Loader 版本的作者字符串，以及
无法可靠建立点分数字核心的版本，标为 `numeric_filterable=false` 并给出原因；它们旁路
数字约束但仍由完整原始版本的 `string` 规则过滤。Forge/NeoForge 的 JAR 声明版本若不以
数字开头，则按 Loader 自身规则在元数据入口报错。

`constraint show` 是只读查询。`constraint set` 接受结构化数字核心策略和一个原始 `--string`
顺序集合字符串，联网建立完整候选闭包，并在
新策略下求一个相对当前 lock 的标准 Pareto 极小包变更方案；多个互不支配方案仍要求用户
选择，随后使用与 add/fix 相同的事务确认和应用路径。求解失败、用户取消或应用失败时，
TOML、lock 和磁盘 JAR 都保持不变；`--dry-run` 只展示事务。`any` 是解除版本限制的唯一
写入方式，不保留单独的 clear 路径。

`exact 1.2.3` 匹配该数字核心的所有 Loader 合法完整表示。数字策略的边界只能是任意段
点分无符号整数；若要精确筛选 `1.2.3-alpha`，使用数字 `exact 1.2.3` 再配置完整字符串规则。
range 的端点包含关系显式传入，并由 core 转换为对应 Loader 家族的原生约束表示，调用方
不拼接 Fabric/Maven 约束文本。

完整字符串规则从 `all` 或 `none` 开始，以 `;` 分隔并从左到右执行。每项为
`intersect [not] <atom>`、`union [not] <atom>` 或对当前结果整体取补的 `complement`。
原子支持空/存在、精确、包含、开头和结尾字符串事实；`"text"` 区分大小写，`i"text"`
不区分大小写。匹配输入始终是完整 JAR 声明版本；Orbit 不把任何字符串硬编码为稳定版、
预发布或 Loader 名。该规则和数字范围在同一求解图中筛选候选。

### `orbit remote`

```text
orbit remote list <package>
orbit remote add <package> file <jar-path>
orbit remote add <package> modrinth <project-id>
orbit remote add <package> curseforge <numeric-project-id>
orbit remote remove <package> <provider> <locator>
orbit remote remove <package> --index <one-based-index>
```

`add` 会先下载并分析目标远端的全部候选，只有其中存在 JAR 实际声明 `<package>` 才写入。
不同 provider 一视同仁，现有和新增远端在后续 add/fix/outdated/upgrade/migrate 中全部进入
同一个候选闭包。`remove` 不能删除最后一个远端。`list` 使用用户可读的 provider/project
信息并以统一自适应表格展示，managed local source 用序号引用，不显示内容哈希。删除
discovery remote 后，当前 lock 的精确恢复来源保留到下一次内容选择，因而不会让已锁定
环境突然不可恢复。

### `orbit install`

```text
orbit install
  [--target client|server|both]
  [--group <name>]
  [--no-optional]
```

这是精确物化命令，不接受模组名。它严格使用 `[platform]` 记录的 Minecraft、Loader
和 runtime JAR 路径并校验内容，不读取 launcher profile、不搜索替代文件，也不刷新
平台快照。路径、哈希或 JAR 元数据不一致时拒绝并要求先 `orbit sync`。sync 刷新后的
loader JAR 及其 bundled 模块进入 lock 图校验；install 不另行选择候选。

选择顺序：

1. 根据 target、group 和 optional 过滤 manifest 的完整受管包集合；包未配置 `env` 时
   使用 lock 中选中 JAR 的 `environment`；
2. 用 lock 中真实依赖边校验过滤后的选择，并保留其必需闭包；
3. 要求 lock 已存在、平台 meta 与 manifest 完全一致，并校验精确 lock 图；
4. 已存在且 SHA-256 正确的 JAR跳过；
5. 缺失 JAR 从缓存、本地 `file:` 或 provider 来源恢复；
6. 下载/复制后再次校验。

`install` 没有 unlocked/frozen 分支；精确 lock 是唯一输入。它不表示物理离线：缓存未
命中时仍可使用 lock 已记录的下载 URL。每个 package 必须有 SHA-512、非空 remotes 和
可恢复所选字节的精确来源，缺项直接拒绝。该命令绝不发现候选、选择版本、修复依赖、
删除未选包或改写 `orbit.toml` / `orbit.lock`。

空 lock 且缺失 `mods/` 时 install 成功返回空操作；add/install/fix/upgrade 只有在某个选中
JAR 即将真正物化时才创建 `mods/`。失败、取消、dry-run 与纯删除计划都不补空目录。

### `orbit fix`

```text
orbit fix
```

这是唯一修复入口。它从 TOML 与已有 lock 的全部远端递归枚举项目闭包，先把当前
Minecraft/Loader 对应的所有候选 JAR 加入稳定下载队列，再统一下载、按内容哈希去重并
读取 JAR 声明元数据，最后离线求相对现有 lock 的标准 Pareto 极小变更 front。TOML 中存在
但 lock 缺失的包是必须恢复的目标；已有包优先保留精确内容实现；不在 TOML 中的候选优先
保持不存在。固定极小变更集合后再做版本 Pareto 极大化。唯一解自动采用，多解必须选择；任何
安装、升级、降级、替换或删除都在写入前展示并确认。

提交时以逻辑包为单位：安装入选顶层 JAR，删除所有未入选的本地实现，并在同一事务中让
`orbit.lock` 只保留入选解、让 `orbit.toml` 的完整包集合与所选解收敛并清理无效来源和空组。远端没有列出
JAR 中真实 `mod_id` 依赖时，完整图无解并报告 PubGrub 原因，不在下载层伪造 slug 映射。

### `orbit remove`

```text
orbit remove <mod>
```

只按 JAR-declared `mod_id` 查找受管逻辑包。若仍有其它 package 依赖它则拒绝删除；
否则删除所选包的已校验文件，并从 manifest/lockfile 移除条目。输入不匹配时，交互模式列出
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
| `removed` | 旧 lock 中的包已没有对应本地 JAR，因此不进入重建后的 lock |

sync 不做包求解。loader 版本变化会被如实写入平台快照和 lock meta；兼容性与依赖修复
留给后续 `fix`，loader 版本本身不被先验判为不兼容。

平台与包变更由统一展示层渲染为单张自适应表格。它不下载候选 JAR，但会联网访问
Modrinth 以及已配置的 CurseForge 批量哈希识别接口。匹配成功
时以 provider project/release 替换同一内容的自动 managed-file 回退；只有所有 provider
均未匹配的内容才成为本地精确恢复来源。provider 查询失败会报错，不能静默把所有 JAR
降级成 `file`。dry-run 不保存对账结果。sync 只用实际 JAR 重建事实 lock 并补充 TOML；
缺失依赖也照实记录而不修复。同一 ID 有多个本地实现时，lock 无法无损表达选择，因此
sync 保留全部 JAR 和来源、补充 TOML 后明确要求运行 `orbit fix`，绝不按扫描顺序覆盖或删除。

### `orbit outdated [mod]`

只读查询 package 全部 remotes 的兼容候选。按真实 `mod_id` 限定单包；不存在的输入、
未安装的包和没有可分析候选的包返回明确结果，不会静默当作“已是最新”。
若存在多个互不支配的 Pareto 方案，列出升级集合并请求选择；唯一方案自动
采用。共同动作只显示一次；每个选项只展开与其他选项不同的动作，并用 `◆` 与终端样式
高亮差异。dry-run 仍需选择具体方案；`--yes` 也不自动选择方案。

同一 JAR-declared 版本可以有多个不同内容候选。此时选项表使用 provider project/release
和依赖约束差异说明选择，不显示内部哈希，也不以物理 JAR 文件名充当包名。

输出区分三种结果：有可行更新时显示包/当前版本/可用版本表；存在更高候选但被依赖传播
或回溯排除时显示 PubGrub 推导事实；provider 对当前 Minecraft/loader 没有返回声明该
`mod_id` 的 JAR 时明确报告“无兼容远端候选”。后两种情况不得表述成“已是最新”。

### `orbit upgrade [mod]`

无参数时升级所有允许升级的在线 package；有参数时要求该包已经安装且有在线来源。
升级复用候选下载、真实 JAR 解析、PubGrub 诊断、确认与原子文件替换。已有包的 manifest
版本约束保持不变；若新选择引入或删除逻辑包，TOML 的完整包集合与 lock 同步收敛。多解规则与 `outdated` 相同；
方案选择发生在安装确认之前。批量 upgrade 方案要求至少一个包比当前安装版本更新；
单包 `upgrade <mod>` 的方案必须让指定逻辑包本身变新，不能用无关包的升级冒充成功。
允许为满足依赖而让其他包降级、同版本换源或被删除；这些变化全部列入同一个确认计划。
若没有可行升级则 upgrade 是 no-op，但仍显示阻止更高候选的同一份结构化诊断。dry-run
不替换文件。

更新表、事务表、诊断表和多方案差异由统一终端展示层通过 `comfy-table` 渲染，按终端
宽度自动换行；重定向输出时仍保留表格和 `◆` 差异标记，不依赖 ANSI 颜色传达含义。

## 5. 查询

```text
orbit search <query>
  [--platform <provider>] [--limit <n>]
  [--mc-version <version>] [--modloader <loader>]

orbit info <mod> [--platform <provider>]
orbit list [--tree] [--target client|server|both]
orbit versions <package>
```

- `search` 合并已配置 provider 的结果并应用可选的 Minecraft/loader 过滤；结果由统一
  展示层渲染为自适应表格，按 slug/名称、平台、下载量和最新 MC 版本分列，参考 MC 版本
  存在时附加 `✓` 兼容列；同一现有 JSON 结果还返回最新展示版本、side、categories、
  provider 官方 icon URL 与 RGB accent，原生界面无需调用另一套查询接口；
- `info` 按 provider 顺序查询详情；`mr:` / `cf:` 前缀可显式选择来源；字段渲染为自适应
  表格，内嵌 recent versions 子表；JSON 同时返回 provider 官方 project links 与 gallery；
- `list` 从 lockfile 展示版本、TOML 版本策略、全部 remotes、env/optional；`--tree` 展示
  JAR 依赖，`--target` 按完整包集合过滤并校验依赖闭包；bundled 内容只以模块总数摘要，
  不逐项打印内置模块或物理 JAR；
- `versions` 下载并分析一个受管包全部配置远端的真实 JAR 候选，按版本排序。

在线查询支持 Modrinth 与 CurseForge。`cf:` 和 `--platform curseforge` 只选择
CurseForge，不回退到 Modrinth；缺少 API Key 或目标文件没有 API 下载 URL 时返回明确
错误。

## 6. 导入、导出与检查

### `orbit import`

```text
orbit import <file>
  [--merge-strategy prefer-existing|prefer-import|interactive]
```

- `.toml`：同包 remotes 始终取并集；版本、端侧、optional、exclude 等语义冲突才按策略处理；
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

导出 manifest、lockfile、目标选择中校验通过的 JAR，以及 `config/`、`defaultconfigs/`、
`serverconfig/`、`options.txt` 中不存在符号链接的可移植配置。未指定文件名时使用安全化的
项目名称和版本。JAR 使用 ZIP Stored，避免对压缩容器二次 Deflate；校验和归档写入发出真实
字节进度。`mrpack` 生成 Modrinth index；在线文件可成为 downloads，必须内嵌的本地文件和
配置放入 overrides。dry-run 校验并统计计划，但不创建输出。

### `orbit migrate check` / `orbit migrate export`

```text
orbit migrate check <target-instance-directory>
orbit migrate export <target-instance-directory>
orbit migrate export <target-instance-directory> --source-pack <source.zip> --consume-source-pack
orbit migrate check <target-instance-directory> --allow-removals
```

目标必须是 Launcher 已安装完成的真实游戏实例目录。两个子命令调用同一个迁移规划器：
从目标目录准确探测 Minecraft、Loader、runtime JAR、路径和哈希；按目标版本/Loader 下载
所有远端候选 JAR 元数据；然后对完整依赖图联合求解，而不是逐包查询“有没有文件”。

默认流程不展示预先选择的保留策略。规划器先把 TOML 中每个源逻辑包都作为硬要求；若该
完整集合无解，CLI 先显示严格求解的冲突原因，再询问是否搜索软解。用户同意后，每个仍满足
自身 TOML 版本约束的源包成为一个 PubGrub 包状态偏好，fork 原生枚举未保留包集合的标准
Pareto 极小 front，并在每个固定保留集合内继续做版本 Pareto 极大化。不存在按包逐个删除、
反复重跑或按删除数量加权的路径。多个互不支配软解仍进入统一方案选择。

`--allow-removals` 表示调用方已经作出同一许可，适用于自动化以及 GUI 将已检查方案交给
`migrate export` 时避免重复询问；它不会代替多个 Pareto 方案的选择。没有该参数且 stdin
关闭、机器交互取消或用户拒绝时，迁移以严格无解失败，目标不发生写入。

`check` 只展示将发生的安装、升级、降级、替换和删除。`export` 复用同一规划路径，将目标
平台快照、入选 lock 和源实例的 `config/`、`defaultconfigs/`、`serverconfig/`、
`options.txt` 写入目标；拒绝覆盖已有 Orbit 状态或配置。它不把模组 JAR 安装到 `mods/`；
入选的 file-only 内容会进入目标按哈希寻址的 `.orbit/sources`，随后仍必须在目标目录运行
`orbit install` 统一物化。GUI 的迁移向导只编排源 export、Launcher 创建目标、
migrate export、`instances register` 与目标 install；GUI 不直接写 Orbit 全局注册表。

`--source-pack` 接受同一 `orbit export --format zip` 生成的便携源快照。规划器先在受限临时
目录安全解包并验证 TOML/lock，再将该冻结源状态和真实目标运行时联合求解；它不会从 GUI
状态或文件名猜源包。快照中为离线恢复添加的源版本 `file` 远端不是迁移候选；一个逻辑包
只要还有 Modrinth/CurseForge project，就仅按目标 Minecraft/Loader 重新下载该 project 的
候选。真正 file-only 的包仍进入同一 PubGrub 图，其 JAR 声明不兼容目标时会被严格排除或在
用户许可的软迁移中成为删除项。成功计划不会把已排除的 26.2 候选诊断误报成 26.1.2 迁移
失败。`--consume-source-pack` 只在用户确认且目标状态写入成功后删除源包。
GUI 因而先导出源快照，成功后才新建目标实例，再执行上述目标规划与 install。GUI 不显示
常驻的严格/软策略控件；严格无解时由同一 CLI 子进程的 schema 2 interaction 弹出确认，
GUI 只把选择写回该进程 stdin。

### `orbit audit`

```text
orbit audit
  [--min-risk 0..100]
  [--fail-on-risk 0..100]
  [--mod <id-or-name>]
  [--limit <count>]
  [--report <path>]
```

只读分析当前实例实际存在的 Minecraft、Loader、运行时依赖和 Mod JAR。它与
`orbit migrate` 不同：migrate 联网下载候选元数据并联合求解目标图，`audit` 不联网、不读取
provider 兼容声明，也不修改 manifest、lockfile、下载缓存或实例文件。输出格式由全局
`--output-format` 控制（`text` 或 `json`）；`audit` 不定义自己的 `--format`。

`--min-risk` 只控制 stdout 展示；`--fail-on-risk` 在完整分析后按 `risk_index`
阈值决定是否返回非零退出码。`risk_index` 是 0–100 的排序值，不是不兼容概率。
`--mod` 只匹配已安装 Mod 的 ID、展示名或文件名并过滤文本/JSON stdout；没有匹配项
会明确报错，但有匹配项时分析仍加载完整实例。默认文本把环境、覆盖率、warning、风险
及 behavioral interaction 分类和风险详情渲染为自适应表格，只展示排序最高的 20 条
风险且不展开 coverage/inactive/warning/evidence 明细；每条风险使用两列详情布局，
非 TTY 输出最大 120 列。
`--limit` 调整展示数量。

`--output-format json` 保留完整 evidence（audit 子 schema 5）。显式 `--report <path>` 额外写入
未按文本 limit、risk threshold 或 mod 过滤截断的完整结构化报告；默认模式不创建报告文件。
JSON 结果直接嵌入 audit 的 `AuditReport`（schema 5），顶层固定包含 `schema_version`、
`environment`、`readiness`、`namespace`、`artifacts`、
`registered_mixin_configs`、`registered_mixins`、`transformations`、`unary_risks`、`risks`、
`interactions`、`inactive_candidates`、`coverage_gaps`、`coverage` 和 `warnings`。
没有达到阈值的结果只表述为
“未发现达到当前阈值的字节码兼容风险”，不宣称全部 Mod 兼容。

core 严格读取 `[platform]` 中由 init/sync 固定的 Minecraft、Loader、runtime JAR
与物理端，并按当前精确 lock 通过共享 Loader 图选择实际顶层和嵌套 JAR；audit 不读取
launcher profile，也不另写 Loader classpath 发现规则。随后根据这些输入进行 capability
probe。Fabric/Quilt 需要 Loader 与 Mixin ABI；FML family 还必须具有可识别的
ModLauncher `ITransformer/ITransformationService`，或 NeoForge
`ClassProcessor/ClassProcessorProvider` SPI。Legacy LaunchWrapper 明确拒绝，不提供
`--force`。单个坏 Mod、真正 unresolved/ambiguous 的软引用、已知未支持或自定义
InjectionPoint 和解释预算耗尽进入 warning/coverage；缺失基础游戏或运行库则停止。
“JAR 没有 refmap”本身不是 warning。

ABI probe 后先建立 Loader runtime namespace。Fabric/Quilt 有实际 Tiny 类 mapping 时
投影 Class Universe，否则使用实际 identity 符号空间；若 Mod 与基础游戏符号空间结构
不一致则停止。该判断不读取 Minecraft 版本边界。
Forge/NeoForge 使用 launcher 选择且内嵌版本匹配的 runtime game JAR。Loader 规则要求的
mapping 缺失、冲突或无法唯一识别时直接返回高层 readiness 错误，不继续输出具体 Mod 风险。

审计通过 core 强类型事件报告六个真实阶段：准备 Loader-selected runtime、顶层工件
扫描、readiness、Mixin 分析、Transformer 分析和冲突比较。已知总量的阶段显示实际计数；非交互
文本约按 10% 间隔更新，交互终端使用进度条。`--quiet` 或
`ui.progress_bar = "off"` 关闭进度，不影响最终报告。

详细证据边界见 [orbit-bytecode-audit.md](orbit-bytecode-audit.md)。

## 7. Cache

```text
orbit cache clean
```

先检查配置解析后的缓存目录、文件数和大小；空缓存直接成功。非 `--yes` 时确认，dry-run
不删除。core 会拒绝清理文件系统根、当前目录/其祖先，或包含配置文件与实例注册表的
目录。

当前命令显式清空整个 cache。除此之外，每个 CLI 命令结束时都会按照全局
`[cache].capacity_mib` 执行一次持久化 LRU 淘汰；命令本身报错也不跳过该收尾步骤。
容量只统计 SHA-512 内容 JAR，淘汰时同步移除失效 SHA-1 别名。

## 8. 全局配置

`orbit config path/list/get/set/unset` 已实现；配置结果同时支持 text 和统一 JSON
信封。敏感值在 view-model 边界脱敏，修改命令不会把环境变量覆盖写回文件。字段校验、
默认值、cache 生效时机见 [orbit-global-config.md](orbit-global-config.md)。

## 9. 正确规范与剩余差距

以下不是“历史文档已过时”，而是仍正确但代码尚未完全遵守的 CLI 规范：

| 规范 | 当前差距 |
|------|----------|
| `--quiet` 只输出错误 | 多数 handler 仍直接 `println!`（text 模式），只有实例上下文日志检查 quiet |
| `--verbose` 展示网络/解析细节 | 当前主要展示实例选择，没有统一结构化日志层 |
| 用户取消使用独立退出码 3 | clap 参数错误为 2、普通错误为 1；部分取消当前为成功或普通错误 |
| 全局运行配置控制网络/并发/UI | `ui.progress_bar` 已接入；代理、重试、语言、颜色和下载并发尚未全部接入 |
| 大规模物化有界并发 | 候选验证并发，最终 JAR 物化仍按确定顺序执行 |

已经实现、旧文档不应再标为缺失的内容：

- 全部命令 handler 已接入 core，不再是 `exit(2)` 占位；
- Forge、NeoForge、Quilt 检测和 JAR 解析；
- `file:` 添加、精确 install、target/group/optional；
- list target、sync/fix/migrate/purge、导入导出、实例与 cache；
- 默认实例的修改型命令安全阻断；
- 非交互 init 不猜 loader/版本，重复 init 不覆盖项目；
- 全局 `--output-format text|json` 与 `--progress-format none|ndjson`；JSON 信封 + NDJSON 进度 +
  结构化错误 JSON + 稳定错误码 + 退出码（见 [orbit-output-formats.md](orbit-output-formats.md)）；
  `export --format zip|mrpack` 只选择归档格式，输出协议与之完全分离。

CurseForge 已接入 search/info/add/fix/migrate/outdated/upgrade 和 lock 精确恢复的共享
路径。它仍受 Core API Key 和项目第三方下载许可约束；这些是外部服务边界，不会用
硬编码 ID 或猜测 CDN URL 规避。
