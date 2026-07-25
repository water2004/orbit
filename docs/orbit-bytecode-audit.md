# Orbit 字节码兼容风险分析

## 1. 证据边界

`orbit audit` 每次都重新打开当前硬盘上的文件，只使用：

- 当前实例实际 Minecraft JAR；
- launcher profile/组件指向的 Loader 和运行时依赖 JAR；
- `mods/` 下的顶层 JAR、其中经当前求解图选中的活动嵌套 JAR 的 `.class`；
- 同一顶层 Mod JAR（含嵌套内容）中的 Mixin refmap。

manifest、lockfile 只用于定位实例和生成展示名。Fabric/Quilt/Forge/NeoForge 元数据、
Modrinth、CurseForge、作者声明的 depends/breaks/conflicts 均不构成风险证据。分析不
下载 Yarn、Mojmap、SRG、Tiny 等 mapping；类和成员始终使用 ClassFile 的 internal
name 与 descriptor。

没有持久化分析缓存。每次命令都会重新读取、计算本次报告所需哈希并解析 JAR；内存中的
类索引和方法对象在进程退出后消失，Orbit 的下载缓存不存放分析结果。

## 2. 分层

依赖方向固定为：

```text
orbit CLI → orbit-core → orbit-bytecode-audit
```

core 重新探测 launcher 平台并组装精确路径。独立分析 crate 不认识 Orbit manifest、
lockfile、provider 或 CLI，只接收 Artifact 列表与实际 Loader 环境，返回结构化报告。
CLI 仅负责过滤、文本/JSON 展示和退出码。

对于 Loader 管理的多版本合并 JAR，core 先用与 install/sync 相同的求解图恢复本次实际
选择的 nested identity，再把允许展开的物理嵌套路径交给分析 crate。未选 Minecraft
版本的嵌套实现不会进入 Class Universe，也不会产生 Mixin 风险。

ClassFile 前端使用 `ristretto_classfile`，由自有 facade 隔离第三方类型。当前完整解析
范围覆盖 Java 旧版本至 Java 25；更高 major 以 Java 25 parser 能力尽力解析并降低
覆盖说明。类空间保留同名类的所有定义，不以最后一个覆盖前一个。目标方法指令同时保留
稳定顺序 ID 和原始 byte offset。

## 3. Readiness

probe 同时比较 Orbit 声明、fresh loader 探测和实际 classpath：

- `Ready`：Minecraft、Loader、至少一个 Mod 可解析，实际 Loader marker 与 ABI 完整；
- `Incomplete`：基础 JAR、类空间、Mixin 或现代 Forge/NeoForge 运行库不完整；
- `Ambiguous`：声明与 fresh 探测冲突，或 classpath 同时出现冲突 Loader；
- `Unsupported`：Loader 不支持，或实际 ModLauncher ABI 无法识别。

Fabric/Quilt 要求实际 Loader marker 与 Mixin annotation ABI。Forge/NeoForge 还验证
`ITransformer` 的 `targets/transform/getTargetType/castVote`、`Target` factory、
`TargetType` 和 `ITransformationService.transformers()` 签名。判断不依赖固定版本号。

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

不读取 Mixin config，因此 Mixin 一般是 `Candidate`；这与目标解析精度分开表达。

## 5. Mixin

分析器从 Annotation 发现 `@Mixin`，支持结构合并、`@Shadow`、`@Overwrite`、
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
ordinal 和 shift；NEW 严格要求 NEW opcode；CONSTANT 严格解析类型化键值，不使用
字符串后缀匹配。INVOKE_ASSIGN 与 MIXINEXTRAS:EXPRESSION 当前标记为
`known_but_unsupported` 并降级位置精度，不伪装成普通 INVOKE，也不丢失原 injector
的 mutation 语义。真正的自定义 InjectionPoint 单独分类。`@ModifyConstant` 不要求
伪造 `@At`，会直接解析 `@Constant` 的 literal、
ordinal 与 zero-condition discriminator；未显式给 discriminator 时按 handler 返回
类型匹配实际常量指令。

每个软引用分别得到 `direct_exact`、`refmap_exact`、`ambiguous` 或 `unresolved`
状态。原始引用在活动 Class Universe 唯一命中时不需要 refmap、不警告且不降低
confidence；无关 refmap 也不能抬高当前引用。只有当前引用真正歧义或无法解析时才产生
对应 warning。

slice 先按主 `@At.slice` 选择 ID，再解析 from/to（含 boundary ordinal/shift），随后
在区间内匹配 selector，最后应用主 ordinal 和 shift。边界或 ID 无法解析时整个
injector 降为方法精度，禁止退回全方法搜索后声称具体指令。require/allow 作用于整个
`InjectionQuery`，expect 仅作为调试预期保存；`@Group` 按 Mixin class + group name
聚合成员成功数。

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

分析实际 ClassFile 中直接/间接 `ITransformer`、匿名内部类、
`ITransformationService` 工厂和 invokedynamic implementation method。静态
`targets()` 中的 Target factory 恢复类、方法或字段目标。

有界解释器跟踪 transform 输入节点的局部变量、字段访问、helper/lambda 路径、
字符串/整数/opcode 和常见 ASM tree/visitor 修改，包括 InsnList
add/insert/insertBefore/remove/set/clear、iterator remove、成员列表与结构字段写入。
只有接收者能追溯到 transform 输入时才生成 Mutation；内部临时 ASM 对象的写入会忽略并
记入 warning。当前解释器没有完整 JVM operand stack、堆别名和路径证明，因此最近常量、
固定 taint 窗口和构造节点关联一律标成 heuristic/partial、Pattern/Method 精度与 Low
confidence，不能产生“精确指令修改”。无法证明 collection provenance 的
`Iterator.remove()` 保留为 UnknownMethod；无法证明新旧值不同的 ClassVisitor/ClassNode
结构写也保留为 unknown。

多个 recovered target 无法与 mutation 分支一一关联时，不做 target×mutation
笛卡尔积，而是逐 target 生成 unknown effect。只有未来同时证明 target、输入节点来源、
写操作与控制流关联后，才允许提升精度。

动态 target 完全无法恢复时只记录 coverage 缺口，不与全部 Mod 制造风险边。目标已知但
效果未知时分别降级为 UnknownMethod 或 UnknownClass。JavaScript CoreMod、native/JVMTI
和不存在于 ClassFile/refmap 的转换逻辑不支持。

## 7. 冲突和风险值

冲突比较只在共享类/成员/指令桶中进行，包含：

- Overwrite、真正互斥的组合矩阵、同签名成员和结构写写冲突；
- 破坏性写操作使另一效果的指令/slice 锚点失效；
- 对完整 InjectionQuery 重算 require/allow/ordinal 与组级 min/max；
- 只有新增 RETURN 或有明确模式证据的 Transformer 插入才影响对应 query；
- ChangeLocalLayout 会与 locals capture 比较，ModifyVariable 的局部值写不会被当成
  布局变化；cancellable 不再泛化成“破坏所有 RETURN/TAIL”；
- 未知方法/类修改与精确效果重叠；
- 多个 Mod 提供形状不同的同名类，以及遮蔽后硬成员引用失效。

`severity` 表示双方生效后的潜在后果，`confidence` 表示恢复证据精度，
`activation` 表示 Loader 激活确定性。三者在报告中独立显示；`risk_index` 使用乘法
门控形成 0–100 的启发式排序值，使 Critical/Low/Candidate 低于 High/Exact/Definite，
明确不是不兼容概率。同名类形状差异是确定事实，但在没有 ClassLoader 可见性证明时，
遮蔽风险的 activation 是 Conditional，不是 Definite。

## 8. 文本与详细报告

默认文本以自适应表格分别显示环境/readiness、coverage、覆盖缺口、风险等级分布、
排序最高的前 20 项、warning 分类计数和详细报告提示。每条风险使用“编号/详情”两列，
不会因 JVM descriptor 过长而把多个语义列压成不可读的窄列；TTY 服从当前终端宽度，
重定向输出无法探测宽度时限制为 120 列。`--limit` 可调整展示数；文本不会展开
`Evidence.detail`、selector 候选、refmap、stable ID 或每条 warning。

`--format json` 在 stdout 输出未截断的结构化细节。`--report <path>` 仅在用户显式
指定时额外写完整、未按文本 limit 截断的 JSON 报告；默认命令不创建文件。当前 schema
version 为 2。

## 9. Coverage 与安全预算

报告记录 JAR/ClassFile 成功失败、方法降级、Mixin 数、各精度效果数、Transformer/
target/effect 恢复数、unsupported mechanism 和 budget exhaustion。单个坏 Mod JAR、
类、方法、refmap 或解释路径不会丢弃其他 Mod；Minecraft/Loader 等必需运行时失败则
停止。

安全限制覆盖 JAR 与嵌套 JAR entry 数、单 entry/类大小、累计解压大小、嵌套深度、
类/方法/指令数量、annotation 深度、解释状态数和 helper 深度。预算耗尽会降级和警告，
不会无限分析。

分析过程另行发出不进入 `AuditReport` 的强类型进度事件：输入准备、readiness、顶层
artifact 扫描、Mixin、Transformer 和冲突比较。artifact/Mixin/Transformer 使用扫描后
可知的真实总数；不会按定时器伪造百分比，也不会把物理 JAR 文件名放进终端进度。
