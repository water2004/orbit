# Orbit manifest 与 lockfile 规格

本文描述当前格式。旧的 `[dependencies]`、`[overrides]`、字符串依赖、单一
`provider`/`slug` 锁字段和 `[resolver].platforms` 已删除，不提供兼容解析。

## 1. 数据模型与职责

Orbit 区分四类对象：

- 逻辑包：一个由顶层 JAR 的 Loader 元数据声明的 `mod_id`；这是求解和事务的最小单位。
- 包版本：JAR 声明的版本字符串，用于约束和版本优先级。
- 候选实现：Orbit 按下载内容的 SHA-512 区分。同一 `mod_id`、同一版本可以有多个候选，
  因为依赖、环境或内嵌内容可能不同。
- 远端：只说明去哪里发现或恢复 JAR；不决定 `mod_id`、版本、依赖或环境。

一个逻辑包可由多个物理 JAR 载体共同提供；一个物理 JAR 也可包含递归 bundled 模块。
这些载体和模块不自动成为独立逻辑包。只有作为顶层选择参与求解的 `mod_id` 才分别出现在
`[packages]` 和 lock 的 `[[package]]` 中。

`orbit.toml` 是完整的受管逻辑包集合及其用户策略。每个实际选择的顶层包都必须有一个
`[packages.<mod_id>]`，不区分“根包”和“传递包”。`orbit.lock` 只记录当前精确选择和从
JAR 读取到的事实，不表达用户意图。精确恢复优先使用 lock；无 lock 时可按 TOML 重新求解。

`.orbit/runtime-data/ownership.toml` 不属于 manifest/lock schema，也不是 JAR cache。它是
`orbit launch` 的 Runtime Agent 产生的实例本地 provenance，以稳定逻辑 `mod_id` 记录文件的
最后编辑者及目录树的递归默认所有者。它不记录读取。另一个包成功修改文件即取得该文件；向
无主目录写入文件会认领直接父目录（共享实例根除外）；更具体节点覆盖父归属。排除项持续增长时，
Orbit 只在 snapshot 合并冷路径枚举元数据并按实际文件数重压缩目录默认值，逐文件有效归属不变。
运行时来源仍由 lock 的内容哈希精确识别，并在 snapshot 落盘前转换为 `mod_id`；因此替换 JAR
不会切断历史归属。包身份仍只由 lock 的 `mod_id` 和内容哈希映射，
归属账本不新增另一套包模型，也不参与依赖求解或 lock 的包版本选择；但 export/migration 会
用它携带入选包拥有的实例内用户数据。`external` 记录只参与源实例本机 reset/purge，不进入
便携导出或迁移目标。

## 2. `orbit.toml`

### 2.1 完整示例

为缩短示例，平台哈希中的 `...` 是文档占位符，不是可写入文件的有效值。

```toml
[project]
name = "survival"
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.16.10"
description = "example instance"

[platform]
minecraft_jar = { path = "../../versions/1.20.1/1.20.1.jar", sha256 = "..." }
loader_jar = { path = "../../libraries/net/fabricmc/fabric-loader/0.16.10/fabric-loader-0.16.10.jar", sha256 = "..." }
runtime_jars = [
  { path = "../../libraries/org/ow2/asm/asm/9.7/asm-9.7.jar", sha256 = "..." },
]
physical_environment = "client"

[resolver]
catalogs = ["modrinth"]

[packages]
sodium = { version = ">=0.5", remotes = [
  { type = "modrinth", project_id = "AANobbMI" },
  { type = "curseforge", project_id = 394468 },
] }

reeses_sodium_options = { version = "*", optional = true, env = "client", remotes = [
  { type = "modrinth", project_id = "Bh37bMuy" },
] }

local_helper = { version = "=1.0.0-local", remotes = [
  { type = "file", path = ".orbit/sources/local-helper.jar" },
] }

[groups]
benchmark = { packages = ["sodium", "reeses_sodium_options"] }
```

未知字段直接报错。每个包必须至少有一个非空且不重复的远端；CurseForge project ID
不能为 `0`。

### 2.2 `[project]` 与 `[platform]`

| 字段 | 含义 |
|---|---|
| `project.name` | Orbit 实例名称 |
| `project.mc_version` | 当前实例的 Minecraft 版本 |
| `project.modloader` | `fabric`、`quilt`、`forge` 或 `neoforge` |
| `project.modloader_version` | 上次实际探测到的 Loader 版本 |
| `platform.minecraft_jar` | 精确 Minecraft JAR 路径与 SHA-256 |
| `platform.loader_jar` | 精确 Loader JAR 路径与 SHA-256 |
| `platform.runtime_jars` | Launcher 为此实例选择的其余运行时 JAR，按内容去重 |
| `platform.physical_environment` | `client`、`server` 或无法确定时的 `both` |

只有 `init` 和 `sync` 运行隔离的 launcher/服务端探测模块，并整体写入平台快照。其他命令
只读取这些精确路径并校验哈希和元数据；不存在、变化或互相矛盾时直接要求 `orbit sync`，
不搜索邻近文件、不按文件名猜测，也不回退到旧路径。Loader 版本变化本身不会先验报错，
实际兼容性由后续 JAR 元数据求解与 audit 判断。

### 2.3 `[resolver]`

| 字段 | 默认值 | 含义 |
|---|---|---|
| `catalogs` | `["modrinth"]` | 无限定 `search`/`add` 使用的 provider 集合 |

`catalogs` 不是远端优先级。一个包配置的全部 `remotes` 都进入同一次候选发现，并按内容
哈希去重；不会在首个 provider 返回结果后停止。CurseForge provider 必须配置 API Key。

### 2.4 `[packages]`

```toml
[packages]
package_id = {
  version = "*",
  string = 'all; intersect not contains(i"beta")',
  enabled = false,
  optional = false,
  env = "client",
  exclude = ["broken_optional_edge"],
  remotes = [
    { type = "modrinth", project_id = "exact-project-id" },
    { type = "curseforge", project_id = 123456 },
    { type = "file", path = "../sources/package.jar" },
  ],
}
```

| 字段 | 默认值 | 含义 |
|---|---|---|
| `version` | `"*"` | 该包的数字核心策略；操作数不能含作者文本 |
| `string` | `"all"` | 对完整 JAR 声明版本从左到右执行的集合规则 |
| `enabled` | `true` | Loader 是否应发现该包；`false` 对应 lock 中的 `.jar.disabled` 载体 |
| `optional` | `false` | 是否可由 `install --no-optional` 过滤 |
| `env` | 无 | 可选 `client`、`server`、`both`；缺失时跟随选中 JAR 声明 |
| `exclude` | `[]` | 用户明确排除的 JAR 依赖边 |
| `remotes` | 无 | 非空候选发现来源集合 |

表键必须是 JAR 实际声明的 `mod_id`。Modrinth 持久化 project ID，CurseForge 持久化
数值 project ID；slug 只用于搜索和展示。远端新增前会下载 JAR 验证其确实声明目标
`mod_id`。

所有包条目地位相同。若包 A 的 JAR 依赖包 B，依赖边仍记录在 lock 的 A 元数据中，
而实际选中的 B 也必须有自己的 `[packages.B]`，通常由操作自动补入、版本策略为 `*`。
不需要也不存在 override 表；用 `orbit constraint` 直接修改任一包的策略。

`env` 是可选用户过滤，不是 Loader 元数据副本。未配置时，当前展示使用 lock 中选中
JAR 的声明；下一次求解使用每个候选 JAR 的真实声明。没有包级环境声明的 Loader 适配器
产生 `both`。`init`、`sync` 和未传 `--env` 的 `add` 不把自动结果写回 TOML。

### 2.5 版本约束

包策略把 Loader 可建立的数字核心与完整原始版本字符串分开：

| 约束 | 行为 |
|---|---|
| `*` | 允许全部版本 |
| `=1.2.3` | 匹配数字核心 `1.2.3` 的全部 Loader 合法完整表示 |
| `!=1.2.3` | 排除整个 `1.2.3` 数值核心类 |
| `> >= < <=` | 只按数值核心比较 |
| Fabric/Quilt 的 `x`、`*`、`~`、`^` | 按相同数值核心边界生成范围 |
| Forge/NeoForge 的 Maven range | 只允许数字端点，保持 Loader 的开闭/并集语法 |

`version` 的边界是任意段点分无符号整数；`=1.2.3-alpha` 属于无效数字规则。若要选择这个
完整表示，应使用 `version = "=1.2.3"` 和完整字符串规则。`1.2.3-alpha` 与
`1.2.3-beta` 是不同候选、不同可选方案，但具有相同升级/Pareto
优先级；从一个切换到另一个记为 `replace`，不是 upgrade/downgrade。若二者都可行且处于
Pareto 前沿，必须交给用户选择。候选身份可由内容哈希区分，但交互只显示版本、远端和 JAR
依赖差异，不显示哈希。

相关命令：

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

`versions` 联网枚举该包全部配置远端，统一下载、缓存、读取 JAR 元数据后按数字核心降序
列出真实候选，并返回数字核心、完整版本字符串的快捷片段和可过滤状态。`constraint show` 只读；
`constraint set` 把结构化数值策略转换为当前 Loader 的
原生 TOML 表示，并在内存清单上完成 Pareto 极小求解、选择、确认和文件事务后才一起提交
TOML、lock 与 JAR。无解或取消不会留下半更新状态；解除数值限制使用 `any`。

`string` 不是正则表达式或发布阶段枚举，而是针对完整 JAR 声明版本的顺序集合程序：以 `all`/`none` 初始化候选集，
随后用 `intersect [not] <atom>`、`union [not] <atom>` 和 `complement` 从左到右更新集合。
原子为 `empty`、`present`、`"精确字符串"`、`contains(...)`、`starts_with(...)` 或
`ends_with(...)`；在字符串引号前加 `i` 表示不区分大小写。CLI 对该原始字符串做严格解析，
保存规范空白但不改变操作顺序。数字约束与 string 规则在进入 PubGrub 时一起筛选真实有限
候选，不存在求解后补查。CLI 的 add 不添加隐式规则；只有显式 `--string` 才写入相应约束。
GUI 默认勾选推荐的 `beta` / `snapshot` 排除规则，但实现仍是把规则原文传给 `--string`，
取消勾选就不传，也不能修改已有条目。Fabric/Quilt 的 Loader-valid 不透明版本旁路数字规则，
但仍执行完整字符串规则；Forge/NeoForge 声明在文件属性替换后必须以数字开头。

### 2.6 本地远端与组

包激活是 TOML 用户状态，不是 JAR 元数据。启用包的精确载体名以 `.jar` 结尾；禁用包以
`.jar.disabled` 结尾。`orbit enable/disable` 同一事务更新物理文件、`enabled` 与 lock 的
真实文件名；`sync` 从磁盘后缀重新建立三者的一致状态。禁用不会把包移出受管集合或 lock。

普通 `file` 远端可使用相对实例目录路径或绝对路径。若源位于事务输出 `mods/`，Orbit
先复制到 `.orbit/sources/<content>.jar` 作为实例级持久本地源；它不是全局 LRU JAR cache。
正常输出把该路径显示为 `managed local source`，不会显示内容哈希。

`[groups]` 的 `packages` 只引用已经存在的受管包 ID，不能重复。组只影响按目标安装时的
过滤，不创建第二类包身份。

## 3. `orbit.lock`

下例中的 `...` 仅为篇幅占位；实际 lock 不接受省略哈希或省略 `filename`。

```toml
[meta]
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.16.10"

[[package]]
mod_id = "sodium"
version = "0.5.8"
sha1 = "..."
sha256 = "..."
sha512 = "..."
filename = "sodium.jar"
remotes = [{ type = "modrinth", project_id = "AANobbMI" }]
artifact_sources = [
  { type = "modrinth", project_id = "AANobbMI", version_id = "release-id", download_url = "https://..." },
]
dependencies = []
environment = "both"
provides = []
embedded_artifacts = []
bundled = []
```

`remotes` 枚举以后可重新发现候选的逻辑来源；`artifact_sources` 只枚举能恢复当前内容
哈希的精确工件。同一字节来自多个 provider 时只有一个候选但可有多个恢复来源；不同字节
即使声明相同版本也保持为不同候选。

`dependencies`、`environment`、`provides`、`language_loader`、`embedded_artifacts` 与递归
`bundled` 全部来自下载后的 JAR。稳定 lock 中每个顶层 `mod_id` 恰有一个 `[[package]]`，
并要求内容身份、非空远端和非空精确恢复来源。`sha256` / `sha512`（以及存在时的 `sha1`）
必须是对应长度的小写十六进制；`filename` 必须是没有目录部分的 `.jar` 或
`.jar.disabled`，且整个 lock 在大小写不敏感文件系统上也不能发生文件名碰撞。lock 不标记
根/传递关系，也不保存版本策略；不符合这些约束的状态直接报错，不把哈希或文件名当成路径
继续处理。

## 4. 命令状态转换

- `init`：探测平台、扫描现有顶层 JAR，把每个实际包写入 TOML；无重复实现时创建事实 lock。
- `sync`：联网做 provider 哈希识别并重新探测平台、扫描 JAR，重建事实 lock、补齐 TOML；
  不枚举版本候选、不求解依赖、不挑选重复实现。查询失败不能静默降级为 `file`。
- `add`：递归发现远端 project，统一下载所有候选，按 JAR 元数据求解并将完整选择写入
  TOML/lock。新发现并选中的依赖包也写入 TOML。
- `fix`：按 TOML 完整联网求解并修复；未选逻辑包/实现会在确认后同时从 `mods/`、lock、
  TOML、group 引用和未使用本地源中清理。
- `upgrade`、`migrate export`：提交选择后同样让 TOML 与所选顶层包集合收敛；迁移默认
  严格保留全部源包，严格无解且用户许可软解时才允许从目标 TOML/lock 同时移除未选包。
- `install`：只精确物化现有 lock，不联网求解、不修改 TOML/lock、不修复。
- `remove`：移除逻辑包，同时清理其 TOML 与 lock 条目；仍被其他 JAR 依赖时拒绝。
- `reset`：使用运行时归属账本事务性删除所选包的数据，但保持 JAR、TOML、lock、远端、环境和
  约束不变；它不是 remove 的别名。
- `purge`：按逻辑包 ID 读取运行时归属计划，并用 lock 中当前顶层 JAR SHA-256 防止确认后包被
  替换；展示准确路径及保留的
  嵌套例外；确认后复用 `remove` 的同一事务清理 JAR/TOML/lock，再递归删除该包的所有权根，
  最后清理账本引用。更具体的其它当前所有者、未知或未观测数据不删除。

Provider relation 只用于继续发现 project；真实 required/optional 关系只来自 JAR。所有候选
先加入统一队列，再查询全局 LRU cache 或下载并分析，然后离线交给 PubGrub。缺少某个真实
required `mod_id` 时按无可行解处理。

`upgrade` / `outdated` 的标准版本 Pareto 极大解中，所有投影包的数值核心不能在不降低
任何包的前提下再提高至少一个包。`add` / `fix` 改用相对当前 lock 的标准 Pareto 极小
变更集合：一个方案改动的逻辑包集合若是另一方案的真超集就被排除；互不包含的集合全部
保留。在同一极小变更集合内，再保留版本 Pareto 极大候选。唯一方案自动进入事务确认；
凡是实际进入求解的路径，出现多个互不支配方案时都必须让用户选择。
`sync` 不求解。任何删除在唯一方案下也必须先明确展示并确认。

迁移软解以“保留每个源 manifest 包”为独立偏好，未满足偏好集合按包含关系做标准 Pareto
极小，而不是按删除个数排序。每个包的 `version`、`env`、`exclude` 仍约束它被选中时的
候选定义域；软解只允许包缺席，不能放宽版本策略。所选集合经统一 reconciliation 写入目标
TOML 与 lock，所以被移除包及其无效 group 引用不会残留。

## 5. 远端管理

```text
orbit remote list <package>
orbit remote add <package> modrinth <project-id>
orbit remote add <package> curseforge <numeric-project-id>
orbit remote add <package> file <jar-path>
orbit remote remove <package> <provider> <locator>
orbit remote remove <package> --index <number>
```

全部远端一视同仁且按内容哈希去重。不能删除最后一个远端。删除发现远端不会立即破坏
当前 lock：其精确 `artifact_sources` 保留到下一次选择其它内容。

应同时提交 `orbit.toml` 与 `orbit.lock`。若使用真正本地包，还必须分发其 `.orbit/sources/`
内容或先增加团队可访问的网络远端。
