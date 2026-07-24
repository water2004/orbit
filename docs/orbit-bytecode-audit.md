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
- `Mutation`：替换方法、成员增删、指令插入/删除/替换、redirect/wrap、参数/局部变量/
  常量、访问标志/父类/接口/控制流或未知方法/类修改；
- `Evidence`：来源 Artifact/ClassFile/方法/annotation/指令、refmap 或解释路径；
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
InjectionPoint 支持 HEAD、TAIL、RETURN、INVOKE、INVOKE_ASSIGN、FIELD、NEW、
CONSTANT、JUMP、LOAD、STORE。自定义 InjectionPoint 保留方法级未知修改并发出
warning。`@ModifyConstant` 不要求伪造 `@At`，会直接解析 `@Constant` 的 literal、
ordinal 与 zero-condition discriminator；未显式给 discriminator 时按 handler 返回
类型匹配实际常量指令。

refmap 的 default 和所有 context 都作为候选，在实际类空间验证。唯一可解析候选可提高
可信度；多个可解析候选保持歧义；没有 refmap 时保留原始软引用并降低可信度。
MixinExtras 的 wrap/value/condition 注入按其链式组合语义处理；它们彼此不伪报为
`@Redirect` 式独占写，但与真正独占的 Redirect 或无法证明可组合的其他机制仍会比较。
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

专用有界解释器跟踪 transform 输入节点的局部变量别名、字段访问、helper/lambda 路径、
字符串/整数/opcode 和常见 ASM tree/visitor 修改，包括 InsnList
add/insert/insertBefore/remove/set/clear、iterator remove、成员列表与结构字段写入。
只有接收者能追溯到 transform 输入时才生成 Mutation；内部临时 ASM 对象的写入会忽略并
记入 warning。构造出的 MethodInsnNode、FieldInsnNode 或 LdcInsnNode 会回到实际目标
方法匹配，以恢复具体稳定指令 ID。

动态 target 完全无法恢复时只记录 coverage 缺口，不与全部 Mod 制造风险边。目标已知但
效果未知时分别降级为 UnknownMethod 或 UnknownClass。JavaScript CoreMod、native/JVMTI
和不存在于 ClassFile/refmap 的转换逻辑不支持。

## 7. 冲突和风险值

冲突比较只在共享类/成员/指令桶中进行，包含：

- Overwrite/独占指令写、remove×replace、同签名成员和结构写写冲突；
- 写操作破坏另一效果的指令、slice、cardinality、local 或 control-flow 要求；
- 插入导致 ordinal/cardinality 漂移；
- 未知方法/类修改与精确效果重叠；
- 多个 Mod 提供形状不同的同名类，以及遮蔽后硬成员引用失效。

`severity` 表示双方生效后的潜在后果，`confidence` 表示恢复证据精度，
`activation` 表示 Loader 激活确定性。`risk_index` 按 severity 55%、confidence 30%、
activation 15% 形成 0–100 的启发式排序值，明确不是不兼容概率。

## 8. Coverage 与安全预算

报告记录 JAR/ClassFile 成功失败、方法降级、Mixin 数、各精度效果数、Transformer/
target/effect 恢复数、unsupported mechanism 和 budget exhaustion。单个坏 Mod JAR、
类、方法、refmap 或解释路径不会丢弃其他 Mod；Minecraft/Loader 等必需运行时失败则
停止。

安全限制覆盖 JAR 与嵌套 JAR entry 数、单 entry/类大小、累计解压大小、嵌套深度、
类/方法/指令数量、annotation 深度、解释状态数和 helper 深度。预算耗尽会降级和警告，
不会无限分析。
