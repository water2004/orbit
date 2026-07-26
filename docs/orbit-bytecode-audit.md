# Orbit 字节码兼容风险分析

上游行为依据与跨版本不变量见
[orbit-bytecode-audit-runtime-model.md](orbit-bytecode-audit-runtime-model.md)。

## 1. 证据边界

`orbit audit` 每次都重新打开当前硬盘上的文件，只使用：

- `[platform]` 精确记录且通过路径、哈希和元数据校验的 Minecraft JAR；
- `[platform]` 精确记录且通过校验的 Loader 和运行时依赖 JAR；
- 与 install/sync 相同的 Loader 求解图为当前物理端选择的顶层包和活动嵌套 JAR；
- 上述活动内容中的 `.class`、Loader 模组元数据、Mixin config、refmap、manifest、
  NeoForge TOML 和 `META-INF/services`。

manifest、lockfile 和 JAR 内的 Loader 依赖用于恢复本次运行时内容，但依赖声明本身
不构成字节码风险证据。JAR 内的 Mixin/Transformer 注册资源用于判断代码是否实际进入
转换管线。Modrinth、CurseForge 和它们的兼容性声明不参与 audit。分析不会为了可读
名称下载 Yarn、Mojmap、社区 mapping 或平台数据；但会读取当前 Loader classpath
中实际存在的运行时 mapping 资源，因为它们决定 JVM 最终看到的符号空间。比较仍使用
ClassFile 的 internal name 与 descriptor，只是先把基础游戏投影到 Loader 的运行时
命名空间。

没有持久化分析缓存。每次命令都会重新读取、计算本次报告所需哈希并解析 JAR；内存中的
类索引和方法对象在进程退出后消失，Orbit 的下载缓存不存放分析结果。

## 2. 分层

依赖方向固定为：

```text
orbit CLI → orbit-core → orbit-bytecode-audit
```

core 严格消费 init/sync 写入的 platform snapshot，并复用其中物理端的 resolver
solution 组装 Loader 实际选择的顶层 JAR、嵌套 archive chain、活动 mod ID/provides
和运行时路径。独立分析
crate 不认识 Orbit manifest、lockfile、provider 或 CLI，只接收这份已选择的 Artifact
列表与实际 Loader 环境，返回结构化报告。CLI 仅负责过滤、文本/JSON 展示和退出码。

对于同一 mod ID 的多个顶层版本和 Loader 管理的多版本合并 JAR，core 不另写 audit
专用猜测规则，而是直接消费与 install/sync 相同的求解结果。未选的顶层包版本和嵌套
实现不会进入 Class Universe，也不会注册 Mixin/Transformer。未被 lockfile 识别的
顶层 JAR 仍会扫描并明确保留为未知输入，避免静默隐藏实例中的额外代码。

ClassFile 前端使用 `ristretto_classfile`，由自有 facade 隔离第三方类型。当前完整解析
范围覆盖 Java 旧版本至 Java 25；更高 major 以 Java 25 parser 能力尽力解析并降低
覆盖说明。类空间保留同名类的所有定义，不以最后一个覆盖前一个。目标方法指令同时保留
稳定顺序 ID 和原始 byte offset。

## 3. Runtime namespace 与 Readiness

probe 比较 Orbit 声明、已校验的 Loader JAR 和快照 classpath：

- `Ready`：Minecraft、Loader、至少一个 Mod 可解析，实际 Loader marker 与 ABI 完整；
- `Incomplete`：基础 JAR、类空间、Mixin 或现代 Forge/NeoForge 运行库不完整；
- `Ambiguous`：Loader 声明互相冲突，或 classpath 同时出现冲突 Loader；
- `Unsupported`：Loader 不支持，或实际 ModLauncher ABI 无法识别。

Fabric/Quilt 要求实际 Loader marker 与 Mixin annotation ABI。Forge/NeoForge 还验证
`ITransformer` 的 `targets/transform/getTargetType/castVote`、`Target` factory、
`TargetType` 和 `ITransformationService.transformers()` 签名。判断不依赖固定版本号。

ABI readiness 之后、任何 Mixin/Transformer finding 之前，audit 必须完成 runtime
namespace alignment：

- Fabric/Quilt 从当前 classpath 的 Tiny v1/v2 资源读取 namespace 和类/成员映射；
- mapping 含实际类记录时，以 Minecraft JAR 对各 namespace 的精确类名覆盖确定输入
  namespace，再把整个基础游戏 Class Universe 投影到 `intermediary`；
- Loader 提供空 mapping artifact 时，这是可探测的 official identity capability；
- mapping 缺失、多个来源冲突或输入 JAR 无法唯一匹配时返回
  `Incomplete`/`Ambiguous`，在生成具体风险和 warning 前停止；
- Forge/NeoForge 优先采用 platform snapshot classpath 中版本可验证、实际包含
  `net/minecraft` 类的 Loader runtime game JAR；否则只有基础游戏与已注册转换共享
  可观察的类空间时才允许 identity 分析。

投影覆盖类、父类、接口、字段/方法名和 descriptor、指令 member reference、
invokedynamic implementation 与 annotation class descriptor，并在
ClassDefinitionId 中同时保存 original/runtime 名称。部分 mapping 不会猜测：未覆盖
数量进入 `mapping_coverage`，依赖这些符号的结论不能提升为 definite risk。
Tiny 只在方法的声明 owner 上记录映射时，Orbit 按实际类/接口层次把唯一的继承映射
传播到 override 声明和继承 member reference；冲突的继承名称保持未映射，不能用
“精确 owner 查表失败”制造假的 `@Overwrite` 缺失。

每个顶层 artifact 同时形成一个 `LoaderArtifactUnit`。只有 resolver 按 Loader 元数据
选中的 nested archive 才成为成员；config、Mixin 类、plugin 和 refmap 在该单元内
共享可见性。重复 nested 内容按 SHA-256 在单元内扫描一次，未选择的 nested JAR 不会
并入全局类空间。

若存在 LaunchWrapper/IClassTransformer 且没有可识别的 ModLauncher，命令以以下信息
停止：

```text
当前实例使用 Legacy Forge/LaunchWrapper。
字节码风险分析仅支持 ModLauncher 体系的现代 Forge 和 NeoForge。
```

## 4. 统一效果

Mixin 与 Transformer 都转换为：

- `ShapeRequirement`：类/成员/指令、slice、cardinality、local layout、control flow；
- `InjectionQuery`：一个 injector 的完整候选/选中 join point、require/allow/expect、
  slice 区间和组级 min/max；cardinality 不复制到每个具体指令；
- `Mutation`：修改种类、位置精度与显式组合语义。独占 owner、破坏性替换、值/参数
  decorator、operation wrapper、相邻插入、局部值修改和结构变化不会再压缩成一个
  `exclusive` 布尔值；
- `Evidence`：来源 Artifact/ClassFile/方法/annotation/指令，以及结构化的 mechanism、
  injector、selector、slice、ordinal、shift、refmap 来源、解析状态和分析精度；
- `Precision`：instruction、pattern、method、class、unknown；
- `Activation`：definite、conditional、candidate、unknown。

每个效果还分别保留 config priority、Mixin priority 和 injector order；三者不会
压成一个不可解释的总优先级。只有由当前 Loader 注册、通过物理端和 required-mod
条件的 Mixin 才进入相应层次。plugin 明确接受时为 definite；plugin 结果为
conditional/unknown 时可以恢复条件效果用于交互分析，但不会产生 definite unary risk
或 unresolved warning。

## 5. Mixin

分析器先按 Loader 规则发现注册入口：

- Fabric metadata schema 0/1 的 `mixins` 与 environment；
- Quilt metadata 的 `mixin`；
- Forge manifest 的 `MixinConfigs` 和可静态恢复的 `Mixins.addConfiguration`；
- NeoForge `[[mixins]]` 的 config、requiredMods 和 behaviorVersion。

每个 config 独立解析 required、minVersion、compatibilityLevel、package、plugin、
refmap、priority、mixinPriority、mixins/client/server、defaultRequire、
defaultGroup 和 overwrite.requireAnnotations。physical side、当前实际活动的 mod
ID/provides 和 config 自身作用域共同决定激活；同名 refmap/config 不会跨顶层或嵌套
artifact 串用。未注册的 `@Mixin`、端侧不匹配和 requiredMods 不满足项只进入
`inactive_candidates`，不参与风险比较。

`IMixinConfigPlugin` 使用保守四态结果：
`AlwaysApply`、`NeverApply`、`Conditional`、`Unknown`。只有无分支且所有可见返回都能
证明的常量/当前 target 或 mixin 字符串谓词才能得到 true/false；分支返回不同常量是
conditional。配置/系统属性、Loader API、类加载、静态可变字段、未知 helper、异常路径、
预算耗尽或无法证明全部路径时一律 unknown，绝不默认 false。求值按每个真实
`(targetClassName, mixinClassName)` 单独执行。静态 `getMixins()` 列表可以恢复；
pre/postApply 修改 ClassNode 则进入 coverage gap。若一个 config 的全部 Mixin 都被
证明为 false，会额外记录 `all_mixins_rejected_by_plugin` 一致性检查。

类名归一化只解包完整的 `L...;` 对象 descriptor；`LocalPlayerMixin`、
`LivingEntityMixin`、`LevelMixin` 等普通相对类名不会因首字母 `L` 被截断，数组
descriptor 也不会被误当作普通 internal name。

在活动集合中，分析器支持结构合并、`@Shadow`、`@Overwrite`、
`@Unique`、`@Accessor`、`@Invoker`，以及 Inject、Redirect、ModifyArg(s)、
ModifyVariable、ModifyConstant。MixinExtras 支持 WrapOperation、WrapMethod、
ModifyExpressionValue、ModifyReturnValue 和 WrapWithCondition。

结构分析遵守实际 Mixin 预处理语义：injector handler、synthetic 方法和非 public 的
`@Unique` 方法会被 conform/重命名，private/protected 的 `@Unique` 字段也会重命名，
不把它们的源码签名伪造成成员碰撞；仍可能被丢弃或严格模式拒绝的 public unique 成员
保留为结构效果。

支持字符串 selector 与 `@Desc`、priority、ordinal、opcode、shift/by、
require/expect/allow、group min/max、locals、cancellable 和 slice 边界。内置
InjectionPoint 支持 HEAD、TAIL、RETURN、INVOKE、INVOKE_STRING、FIELD、NEW、
CONSTANT、JUMP、LOAD、STORE。INVOKE_STRING 同时验证 target、`ldc=`、slice、
ordinal 和 shift；NEW 严格要求 NEW opcode，并把类名、`<init>` selector 和“参数
descriptor + 对象返回类型”三种合法形式归一化为同一个分配类；CONSTANT 严格解析
类型化键值，不使用字符串后缀匹配。INVOKE_ASSIGN 与 MIXINEXTRAS:EXPRESSION 当前标记为
`known_but_unsupported` 并降级位置精度，不伪装成普通 INVOKE，也不丢失原 injector
的 mutation 语义。真正的自定义 InjectionPoint 单独分类。`@ModifyConstant` 不要求
伪造 `@At`，会直接解析 `@Constant` 的 literal、
ordinal 与 zero-condition discriminator；未显式给 discriminator 时按 handler 返回
类型匹配实际常量指令。

每个软引用分别得到 `direct_exact`、`refmap_exact`、`ambiguous` 或 `unresolved`
状态。原始引用在活动 Class Universe 唯一命中时不需要 refmap、不警告且不降低
confidence；无关 refmap 也不能抬高当前引用。只有当前引用真正歧义或无法解析时才产生
对应 warning。plugin-controlled、target 缺失、namespace 未对齐、unsupported selector、
可选 `require = 0` 和方法体/切片不可静态检查分别进入 plugin coverage、inactive、
readiness、optional 统计或独立的 `unavailable_method_body` / `unresolved_slice` coverage，
不伪装为 unresolved warning，也不把方法体缺失错误标成切片失败。

slice 先按主 `@At.slice` 选择 ID，再解析 from/to（含 boundary ordinal/shift），随后
在区间内匹配 selector，最后应用主 ordinal 和 shift。边界或 ID 无法解析时整个
injector 降为方法精度，禁止退回全方法搜索后声称具体指令。require/allow 作用于整个
`InjectionQuery`，expect 仅作为调试预期保存；`@Group` 按 Mixin class + group name
聚合成员成功数。

Mixin 以运行时优先级顺序合并进候选类状态。普通成员、接口、字段和 Overwrite 在
MAIN pass 改变候选类，后续 selector 再针对改变后的方法体求值；Accessor/Invoker
生成发生在所有 `INJECT_PREPARE` 之后，因此不会被提前放进 wildcard injector 的目标
集合。这样“推导时选中一条路径、证明时检查另一条路径”的分叉不会发生。相同
config/Mixin 优先级造成的有限顺序歧义会分别重算查询：全部顺序失败才是 definite
risk，只有部分顺序失败则是 conditional risk。

MixinExtras 的 value decorator 与 Redirect 可以分层组合，value decorator 之间及
WrapOperation 之间可以链式组合。只有 Redirect×Redirect、Redirect×破坏性替换等真正
互斥组合才生成独占冲突；remove/set × decorator 仍报告锚点失效。
这些规则以运行时实现而不是版本号表为依据，维护时应对照
[MixinPreProcessorStandard](https://github.com/SpongePowered/Mixin/blob/master/src/main/java/org/spongepowered/asm/mixin/transformer/MixinPreProcessorStandard.java)
以及 MixinExtras 的
[WrapOperation](https://github.com/LlamaLad7/MixinExtras/wiki/WrapOperation)、
[WrapMethod](https://github.com/LlamaLad7/MixinExtras/wiki/WrapMethod) 和
[WrapWithCondition](https://github.com/LlamaLad7/MixinExtras/wiki/WrapWithCondition) 语义。

## 6. ModLauncher Transformer

先从 `META-INF/services/cpw.mods.modlauncher.api.ITransformationService` 找到实际
服务，再跟踪其 `transformers()`、helper/lambda、匿名内部类和间接实现。仅仅实现
`ITransformer` 但没有沿这条注册链返回的类是 inactive candidate，不进入效果管线。
静态 `targets()` 中的 Target factory 恢复类、方法或字段目标。

有界解释器跟踪 transform 输入节点的局部变量、字段访问、helper/lambda 路径、
字符串/整数/opcode 和常见 ASM tree/visitor 修改，包括 InsnList
add/insert/insertBefore/remove/set/clear、iterator remove、成员列表与结构字段写入。
只有接收者能追溯到 transform 输入时才生成 Mutation；内部临时 ASM 对象的写入会忽略并
记入 coverage。Target factory 的实参由保守的 JVM operand stack 与 descriptor
消费规则绑定，已被日志调用消费的字符串不能污染后续目标。解释器仍不声称具备完整堆
别名和路径证明；不能证明控制流对应关系的结果标成 heuristic/partial、Pattern/Method
精度且最高 Medium confidence，不能产生“精确指令修改”。无法证明 collection
provenance 的
`Iterator.remove()` 保留为 UnknownMethod；无法证明新旧值不同的 ClassVisitor/ClassNode
结构写也保留为 unknown。

多个 recovered target 无法与 mutation 分支一一关联时，不做 target×mutation
笛卡尔积，而是逐 target 生成 unknown effect。只有未来同时证明 target、输入节点来源、
写操作与控制流关联后，才允许提升精度。

动态 target 完全无法恢复时只记录 coverage 缺口，不与全部 Mod 制造风险边。目标已知但
效果未知时分别降级为 UnknownMethod 或 UnknownClass。JavaScript CoreMod、native/JVMTI
和不存在于 ClassFile/refmap 的转换逻辑不支持。

## 7. 冲突和风险值

报告把五类事实严格分开：

- `unary_risks`：单个 Mod 与当前已对齐环境之间的确定结构失效；
- `risks`：两个 artifact 的效果之间已找到具体互斥条件；
- `interactions`：顺序或组合会改变行为，但当前证据不证明结构不兼容；
- `inactive_candidates`：当前 Loader/端侧/插件选择不会应用的候选；
- `coverage_gaps` 与 `warnings`：静态分析没有覆盖或软引用无法唯一恢复。

冲突比较只在共享类/成员/指令身份桶中进行，包含：

- Overwrite、真正互斥的组合矩阵、同签名成员和结构写写冲突；
- 破坏性写操作使另一效果的指令/slice 锚点失效；
- 对完整 InjectionQuery 重算 require/allow/ordinal 与组级 min/max；
- 只有新增 RETURN 或有明确模式证据的 Transformer 插入才影响对应 query；
- ChangeLocalLayout 会与 locals capture 比较，ModifyVariable 的局部值写不会被当成
  布局变化；cancellable 不再泛化成“破坏所有 RETURN/TAIL”；
- 未知方法/类修改与精确效果重叠；
- 同名类的实际遮蔽使硬成员引用发生缺失、staticness 或访问级别失效。

同名类只有 private/helper/接口声明顺序等形状差异时不产生 blanket risk。继承层次与
接口 default method 会参与硬成员解析。每个独立失效原因单独生成一个 Risk；聚合不能
从其他原因借用更高 severity/confidence/activation。类定义、方法与指令身份均包含其
物理定义来源，两个不同 JAR 中相同 offset 的指令不会错误相撞。

`severity` 表示双方生效后的潜在后果，`confidence` 表示恢复证据精度，
`activation` 表示 Loader 激活确定性。三者在报告中独立显示；`risk_index` 使用乘法
门控形成 0–100 的启发式排序值，使 Critical/Low/Candidate 低于 High/Exact/Definite，
明确不是不兼容概率。同名类形状差异是确定事实，但在没有 ClassLoader 可见性证明时，
遮蔽风险的 activation 是 Conditional，不是 Definite。

`invalid_overwrite_target` 只在 config/side/plugin 确定、目标类唯一、namespace 与
descriptor 已对齐，并确认目标类在当前合并阶段没有直接声明该方法时生成。继承/接口
方法仍参与 selector 与 hard-reference 解析，但按 Mixin 的 `findTargetMethod` 语义，
不能充当 `@Overwrite` 目标；普通 Mixin 方法可以合法地在子类新增同签名实现。
`@Dynamic` 只补充运行时错误上下文，不会使缺失的 `@Overwrite` 目标合法。该结果属于
`unary_risks`，不会伪造成两个 Mod 的 pairwise conflict。
`UniqueRenamedMethod`/`UniqueDiscardableMethod` 已经消除成员竞争，不产生用户可见的
“priority selects one”交互。

## 8. 文本与详细报告

默认文本以自适应表格显示 runtime namespace/mapping/alignment、四类摘要计数、风险
总数和排序最高的前 20 项。coverage、inactive、Plugin 路径、warning/selector 明细只
进入 JSON，避免在正常终端中制造数百行诊断噪声。
每条风险使用“编号/详情”两列，
不会因 JVM descriptor 过长而把多个语义列压成不可读的窄列；TTY 服从当前终端宽度，
重定向输出无法探测宽度时限制为 120 列。`--limit` 可调整展示数；文本不会展开
`Evidence.detail`、selector 候选、refmap、stable ID 或每条 warning。

`--format json` 在 stdout 输出未截断的结构化细节。`--report <path>` 仅在用户显式
指定时额外写完整、未按文本 limit 或 stdout filter 截断的 JSON 报告；默认命令不创建
文件。当前 schema version 为数字 `4`，顶层固定包含 environment、readiness、namespace、
artifacts、registered_mixin_configs、registered_mixins、transformations、unary_risks、risks、
interactions、inactive_candidates、coverage_gaps、coverage 和 warnings。

`namespace` 给出 runtime namespace、每个 artifact 的符号空间、mapping source
路径/哈希/namespaces、Loader artifact units、alignment、类/方法/字段 mapping
覆盖率与探测证据。涉及投影的 transformation/risk evidence 同时记录 original symbol、
runtime symbol、mapping source 和 confidence。

## 9. Coverage 与安全预算

报告分别记录 JAR/ClassFile 成功失败、方法解析失败、方法预算降级、指令解析降级、
registered/inactive/plugin-controlled Mixin、各精度效果、Transformer target/effect
恢复、namespace/mapping、Plugin 四态、缺失 Mixin 类、nested plugin、optional
unresolved、unsupported selector/injection point、future classfile、unsupported
mechanism 和 budget exhaustion。单个坏 Mod JAR、类、方法、config、refmap 或解释
路径不会丢弃其他 Mod；Minecraft/Loader 等必需运行时失败则停止。

安全限制覆盖 JAR 与嵌套 JAR entry 数、单 entry/类大小、累计解压大小、嵌套深度、
类/方法/指令数量、annotation 深度、解释状态数和 helper 深度。预算耗尽会降级和警告，
不会无限分析。

分析过程另行发出不进入 `AuditReport` 的强类型进度事件：输入准备、顶层 artifact
扫描、readiness、Mixin、Transformer 和冲突比较。artifact/Mixin/Transformer 使用
扫描后可知的真实总数；不会按定时器伪造百分比，也不会把物理 JAR 文件名放进终端进度。
