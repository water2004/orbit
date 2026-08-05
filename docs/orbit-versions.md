# Orbit 版本号与包策略

> 实现位置：`orbit-core/src/versions/` 与 `orbit-core/src/version_string.rs`。

## 1. Loader 版本与 Orbit 包策略是两层语义

JAR 的版本首先由对应 Loader 解释。Orbit 不把四种 Loader 强行压成一种 semver：

- Fabric/Quilt 先尝试各自的 semantic version；合法但无法结构化的非空声明保留为 opaque
  完整字符串。
- Forge/NeoForge 在 `${file.*}` 替换后要求声明匹配 `^\d+.*`，随后采用 Maven
  `DefaultArtifactVersion` / `ComparableVersion` 与 Maven range。
- JAR 中依赖其它 mod 的约束始终使用 Loader 原生语法和完整语义，Orbit 不把依赖声明改写成
  用户过滤规则。

用户对一个受管包配置的策略则明确拆成两个正交字段：

```toml
[packages.example]
version = ">=1.2.3 <2.0.0"
string = 'all; intersect not contains(i"beta")'
```

`version` 只过滤数字核心；`string` 只按完整 JAR 声明版本文本过滤。两者在候选进入 PubGrub
根范围前同时执行，不存在求解后再补一次黑盒检查的路径。

## 2. 数字核心规则 `version`

数字核心是一段或多段点分无符号整数，段数不固定。例如 `1`、`1.2.3` 和
`26.1.2.4` 都是合法边界。结构化 CLI 支持：

| 策略 | 行为 |
|---|---|
| `*` / `any` | 不限制可建立数字核心的候选 |
| `=1.2.3` | 允许数字核心为 `1.2.3` 的全部完整表示 |
| `!=1.2.3` | 排除整个 `1.2.3` 数字核心类 |
| `> >= < <=` | 只按数字核心比较 |
| `range` | 使用明确的上下界及各自开闭状态 |

Fabric/Quilt TOML 仍可保存只含数字、操作符和末端通配符的 Loader-native predicate，例如
`^1.2`、`~26.1`、`0.8.x`、`>=1 <2 || =3`。Forge/NeoForge TOML 可保存数字端点的 Maven
range，例如 `[1,2)`、`(,2]`、`[1,2),[3,)`。裸 Maven 版本仍保持 Loader 的 recommendation
语义，不会被 Orbit 偷改成精确约束。

作者文本不能进入数字操作数：`=1.2.3-alpha`、`>=v2` 和 `[1-beta,2)` 都是无效的包数字
规则。要精确筛选 `1.2.3-alpha`，使用 `version = "=1.2.3"` 再配合 `string` 的精确条件。

## 3. 完整字符串规则 `string`

`string` 的输入始终是完整 JAR 声明文本，不拆 prefix/core/suffix，也不规范化大小写。例如：

```text
v1.2.3-beta+mc26
```

字符串规则看到的就是这一整串文本。规则必须从 `all` 或 `none` 开始，以分号分隔并从左到右
执行集合操作：

```text
all; intersect not contains(i"beta"); intersect not contains(i"snapshot")
none; union "1.2.3"; union starts_with(i"release-"); complement
```

- `intersect [not] <predicate>`：与该谓词集合或其补集取交；
- `union [not] <predicate>`：与该谓词集合或其补集取并；
- `complement`：对截至当前步骤的整体集合取补；
- 谓词：`empty`、`present`、精确字符串、`contains(...)`、`starts_with(...)`、
  `ends_with(...)`；
- `"text"` 区分大小写，`i"text"` 不区分大小写，字符串采用 JSON 转义。

操作顺序就是语义，Core 不做与/或正规化，也不把任何作者词汇解释为稳定版或预发布。
CLI 不会隐式写入规则。若调用方需要下面这条推荐规则，必须直接传给
`orbit add --string`：

```text
all; intersect not contains(i"beta"); intersect not contains(i"snapshot")
```

GUI 新建项时默认勾选该推荐规则，但勾选的效果仍是把原始字符串传给 CLI；取消勾选就不
传。init、sync、import、依赖补入和已有包都不会被改写。

## 4. 无数字核心的候选

Loader 接受一个版本字符串，不代表 Orbit 可以安全发明数字顺序。每个候选报告：

- `numeric_filterable=true` 与 `numeric_core`：数字规则正常执行；
- `numeric_filterable=false` 与 `numeric_error`：无法由 Loader 语义安全建立数字核心。

后一种候选仍然有效时，仅旁路数字规则；完整 `string` 规则始终执行。若它进入最终方案，
事务会明确警告数字策略没有应用。它不会被伪装成 `0`、空文本或猜测出的 `1.2.3`。

Fabric/Quilt 可以产生 Loader-valid opaque 候选，例如 `release-vNext`。Forge/NeoForge 不走这条
旁路：不以数字开头的声明在 JAR 元数据入口就是 Loader-invalid，Orbit直接报错。对于以数字
开头但无法可靠形成完整点分核心的声明，Maven 版本仍可作为 Loader 版本存在，而包数字规则
旁路，字符串规则继续生效。

## 5. 候选身份、相等与 Pareto

Orbit 同时保留三种关系：

1. **内容候选身份**：内容哈希相同才是同一下载候选；哈希不进入用户界面。
2. **完整版本表示**：JAR 声明文本及 Loader 版本表示相同；用于区分可选方案。
3. **数字优先级**：数字核心相同；用于 upgrade/downgrade 与 Pareto 支配。

因此 `1.2.3-alpha` 与 `1.2.3-beta` 是两个方案，但数字优先级相同；切换记为 replace，不是
upgrade 或 downgrade。完全相同的版本字符串若内容与依赖不同也仍是两个候选。PubGrub fork
原生保留相同优先级的不同候选，并以 `same_version`、`same_precedence`、
`strictly_higher` 投影枚举完整 Pareto front；Orbit 不在求解后用哈希或文件名补筛。同一内容
哈希在 lock 与下载目录中的两种内部表示属于一个 `same_version` 实现，不会形成重复方案。

provider 发现的原始 JAR 数量不是解的数量。依赖元数据尚未解析时，不能按版本号预先支配或
删除候选；只有完整候选图中的可行方案才能比较 Pareto 支配关系。

Fabric/Maven 实现内部仍有 prerelease、qualifier、build 等 Loader 术语，这是 Loader 自身
比较规则。它们不会泄漏为包策略中的固定“稳定版/测试版”枚举。

## 6. 命令与机器输出

```text
orbit versions <package>
orbit constraint show <package>
orbit constraint set <package> any [--string '<ordered-set-rule>']
orbit constraint set <package> exact <numeric-core> [--string ...]
orbit constraint set <package> <greater-than|at-least|less-than|at-most> <numeric-core> [--string ...]
orbit constraint set <package> range <lower> <upper> \
  [--lower-bound inclusive|exclusive] [--upper-bound inclusive|exclusive] [--string ...]
```

`versions` 从包的全部远端枚举当前 Minecraft/Loader 工件，统一下载、按哈希缓存并读取真实
JAR 元数据，然后稳定排序。provider release 名或 project ID 从不充当版本。候选机器字段为：

- `version`：完整 JAR 声明版本；
- `numeric_core`、`numeric_filterable`、`numeric_error`；
- `string_tokens`：完整版本与可用于 GUI 快捷选择的文本片段；
- `sources`、`details`、`selected`、`matches_constraint`。

`constraint show` 只读。`constraint set` 立即联网发现候选，并将数字规则与完整字符串规则作为
同一个标准 Pareto 极小包事务求解、选择、确认和提交。若当前 JAR 已满足策略，只持久化策略；
无解、取消或失败不会修改 TOML、lock 或 JAR。省略 `--string` 保留已有规则；解除两部分限制
使用 `constraint set <package> any --string all`，没有第二条兼容写入路径。

GUI 只负责把数字边界控件和字符串操作列表序列化为同一条 CLI 命令。解析、候选过滤、求解、
交互 schema 与原子提交都在 CLI/core 中完成。

## 7. 必须保持的测试契约

- 数字操作数只接受任意段点分无符号整数；作者文本必须使用 `string`；
- 完整字符串规则按顺序执行交、并、原子取反和整体取补；
- 精确字符串区分大小写，`i` 字符串不区分大小写；
- opaque 候选只旁路数字规则，完整字符串规则仍可排除它；
- Fabric/Quilt 保持 semantic + opaque fallback；Forge/NeoForge 在属性替换后要求数字开头；
- 相同数字核心的不同完整表示是不同 Pareto 方案，切换属于 replace；
- 相同完整版本但不同内容身份仍是不同方案；
- 相同内容哈希的 lock 与下载表示是同一实现，不得产生笛卡尔积方案；
- Fabric build metadata 的 `Eq`/`Hash` 一致；
- Maven qualifier、原生精确范围、开闭范围和并集保持 Loader 依赖语义；
- Forge/NeoForge、Java 与 Jar-in-Jar 依赖仍使用各自 Loader/Maven 原生版本模型。
