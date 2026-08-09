# Orbit 版本兼容与格式边界

> 本文是实现约束。版本差异必须进入 `orbit-compatibility` 的显式范围记录；实际文件格式
> 差异必须停留在输入适配器。调用方不得再按版本号复制选择逻辑。

## 1. 唯一事实源

`orbit-compatibility` 是 Orbit、字节码审计与 Launcher 共享的兼容性事实层。它不访问网络、
文件系统、ZIP 或字节码，只提供：

- 封闭的 `ModLoader` 身份；
- 可比较的数值版本边界与官方 Minecraft snapshot 选择器；
- `(Loader, Minecraft 范围, Loader 范围) -> capability` 的唯一选择；
- Loader 不随版本变化的规范包、约束体系、嵌套优先级及 Launcher 身份证据
  （Maven group/artifact、main class、component UID）；
- NeoForge 发布版本编码、artifact 与 Maven 路径的 Minecraft 范围布局。
- Mojang `world_version` 对应的 `pack_version` schema 与 `java_version` 声明策略。

范围是闭合且必须唯一命中的。零条命中表示未验证，多条命中表示表本身有歧义；两者都
直接报错。未知未来版本不会落入“最新规则”、按 major 猜测或 generic fallback。

`VersionSelector::Any` 不是“所有组合都验证过”的别名。它只允许用于能力确实由 Loader ABI
决定、Minecraft 版本不改变该能力的轴；调用方仍必须验证实际 JAR 的结构或 ABI。如果以后
发现 Minecraft 版本也改变该能力，必须把这一行拆成 Minecraft 范围，不能在调用方补 `if`。

## 2. 单一路径与真实适配器

“统一”必须满足以下两个条件：

1. 版本选择只发生一次，选中的是数据，不是另一个 Loader 专属编排对象；
2. 输入解析后进入同一个领域模型和同一条事务、求解或分析流水线。

当前边界如下：

| 领域 | 统一路径 | 允许存在的真实差异 |
|---|---|---|
| profile 探测 | 一个 `ProfileDetector` 注册表和一次扫描 | Maven group/artifact、main class、component UID |
| dedicated server 探测 | 全部格式归一化为 `ServerRuntimeSpec` | 官方 bootstrap manifest、shim list、argument file 的实际 schema |
| Launcher profile | 同一下载、校验、归一化和安装计划 | Fabric Meta 与 Quilt Meta 的 JSON schema |
| Forge family installer | 同一 staging/执行/检查/提交事务 | 官方 installer 输出格式；NeoForge 布局由范围记录选择 |
| resolver | 一个 `build_solver_graph` | metadata 与版本约束 parser |
| audit | 一个 `analyze` 流水线 | 范围记录选择的 namespace、Mixin 注册和 transformer capability |
| Runtime Agent | 一个 Agent/recorder | 范围记录选择的 code-source 与 Loader 原生身份能力 |

结构自描述的官方落盘格式不应被强行改成版本表。例如 server argument file 自己声明
Minecraft/Loader 版本和 classpath，Orbit 应解析并交叉校验这些事实；给它再套一层猜测性的
版本分支反而更弱。反过来，NeoForge artifact 名称和版本编码随 Minecraft 线变化，属于真正
的版本布局，必须由范围表选择。

## 3. 二次验证

范围表只决定“应当用哪种解释”。它不能替代实际工件验证：

- platform detection 最终读取真实 Minecraft/Loader JAR、版本元数据与 SHA-256；
- audit 在选择 namespace/Mixin/transformer capability 后，仍探测 Loader runtime 的实际
  Mixin、ModLauncher ITransformer 或 NeoForge ClassProcessor ABI；
- Legacy Forge/LaunchWrapper 因实际 ABI 不满足而给出明确 unsupported，不会仅凭范围宣称可用；
- Runtime Agent 只接受范围内 Loader/Java，并使用实际 code source/module identity；
- Minecraft Java feature 只读取真实 `version.json`，resolver、audit 和候选发现消费同一个
  数值，不按 Minecraft 版本猜 Java。

版本声称与实际 JAR 不一致时，以“不一致错误”终止，不能切换到另一条探测或兼容路径。

## 4. 当前范围事实

Runtime Agent 与 audit 当前注册的 Loader 线为：

| Loader | 已注册范围 | 关键边界 |
|---|---|---|
| Fabric | 0.4.x–0.19.x | Mixin/Fabric namespace；Agent `file:` source |
| Quilt | 0.12.x–0.30.x | 0.18 起可用 Quilt 原生 module identity |
| Forge | 14.x–64.x | 37 起 secure module/`union:`；14–36 audit 由实际 ABI 拒绝 LaunchWrapper |
| NeoForge | 47.1.x、20.2.x–21.11.x、26.1.x–26.2.x | 实际 ABI 在 ITransformer/ClassProcessor 间验证 |

NeoForge 分发布局另按 Minecraft 范围选择：

- Minecraft 1.20.1：legacy `net.neoforged:forge`，仅 47.1.x；
- Minecraft 1.20.2–1.x：短版本编码 `net.neoforged:neoforge`；
- Minecraft 26.x 及以后已登记数值线：完整 Minecraft 版本编码；缺省 patch 用 `0`，解析时
  还原为不带 `.0` 的 Minecraft 版本；
- 官方 snapshot：`0.<snapshot>.<build>` 编码。

Minecraft JAR 内部格式按不可变 `world_version` 范围选择：1913–2586 使用共享整数
`pack_version`，2681–4440 使用 resource/data 双整数，4534 起使用 major/minor 四字段；
空档不猜 JSON 形状。1913–2713 的官方 `version.json` 隐式 Java 8，2714 起必须显式声明
`java_version`。这些策略和 Loader 能力使用同一个“唯一范围命中，否则失败”的选择原则。

未登记的空档和未来线直接不支持。扩展范围必须同时加入边界内、边界外、歧义和真实工件
fixture；只修改 maximum 但没有证据与测试不算支持。

## 5. 禁止事项

- 在 core、audit 或 Launcher 调用方比较具体 Minecraft/Loader 版本字符串；
- 在多个模块重复去除 Minecraft 版本前缀或解码 NeoForge 发布版本；
- 把同一 `ModLoader` match 成另一个同构 Loader enum；
- 为四个 Loader 复制 resolver、audit orchestration 或 profile 扫描器；
- 在范围未命中、JAR 缺失、ABI 不符时选“最接近”规则；
- 用文件名或目录顺序替代官方 manifest/profile/argument file 中的事实。

Loader 专属 match 只有在读取真实不同的 wire/archive schema 时才成立；其输出类型必须相同，
且后续层不得再次知道这个 schema 来自哪个适配器。
