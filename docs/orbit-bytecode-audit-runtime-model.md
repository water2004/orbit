# Orbit audit 运行时模型研究记录

本文记录 `orbit-bytecode-audit` 的实现依据。它描述可跨版本观察的协议和生命周期，
不复制上游实现，也不把某个 Minecraft、Loader 或 Mod 版本写进判断表。

## 研究范围

本轮实际检查了以下上游源码：

| 项目 | revision |
|---|---|
| FabricMC/fabric-loader | `b907c5b292fc062d75b6d8bf8255ac200109b992`，并比较 `0.16.14`、`0.19.3` |
| QuiltMC/quilt-loader | `c5c3b0f6e67bfa2f0744856b277b1d92884c3965`，并比较 `0.26.4`、`0.30.0` |
| SpongePowered/Mixin | `releases/0.8.7` / `4053421aa10aaac6127d969028a29c94fe3054f6` |
| FabricMC/tiny-remapper | `7834504ce1be97df03e99723bef456e40241c607` |
| FabricMC/mapping-io | `06f4ec3f872d7e9b6643919c4c48059b911f97c2` |
| McModLauncher/modlauncher | `901c6ea849ae21ee7d464cd97113e77a6101a734`，并比较 `9.0`、`10.0`、`11.0` |
| neoforged/FancyModLoader | `b6b853518f4c04ac743b83d68606aac06bf72545`，并比较 `4.0`、`10.0` |
| MinecraftForge/MinecraftForge | `d17cfd0b4bbfd192a9007f02240032f19b9b340d`，并检查 `1.20.x`、`1.21.x` 分支 |

源码只用于确认行为。Orbit 的 Rust parser、投影器和抽象解释器均为独立实现。

## 稳定不变量

Fabric Loader 的 `MappingConfiguration` 依据 mapping 内容决定目标 namespace；生产
环境中有实际类映射时运行时游戏类进入 intermediary，没有类映射时保持 official。
`MinecraftGameProvider` 在把游戏类加入运行 classpath 前完成该转换。稳定边界是
mapping namespace/内容和转换发生顺序，不是缓存目录名、内部 Java 类名或版本号。

Quilt 的旧实现由 `MappingConfiguration` 读取 Tiny 并选择 target namespace，新实现把
同一个边界拆成 `MappingConfigurationImpl` 与 `EmptyMappingConfiguration`。Minecraft
provider 当前如何选择两者属于 provider 内部策略，不是 audit 应复制的版本规则。
Orbit 因此只消费稳定、可观察的结果：classpath 有有效 Tiny 类映射就按实际 namespace
投影；没有有效类映射就按 identity 分析。若 Mod 已引用 `net/minecraft` 而基础游戏实际
处在另一符号空间，identity 会因结构不一致而 `Incomplete`，不会依据版本字符串猜测。

Tiny v1/v2 是可探测资源格式。Orbit 读取当前 classpath 已有的 mapping，按 Minecraft
类名对 namespace 做精确覆盖匹配，再投影完整内部 Class Universe。它不需要生成可运行
JAR，也不下载供人阅读的 mapping。无法唯一确定输入 namespace 时 readiness 失败。

Mixin 先由 Loader 注册 config，再按 config package/side 展开 Mixin，随后对每个
target/mixin 对调用 `IMixinConfigPlugin.shouldApplyMixin`。因此 plugin 结果必须逐项
求值，而且只在所有路径可证明时才可接受或拒绝。未知运行时状态不能解释成 false。
preApply/postApply 发生在类节点转换阶段，静态分析无法证明的修改属于 coverage。

Fabric 在 `0.16.14` 与 `0.19.3` 都以全局 config 名称表拒绝跨 Mod 重复；当前 NeoForge
也以 config 名称表报告 duplicate loading issue。Quilt 的 Fabric 兼容 metadata 先汇总
为全局名称集合，而 Quilt 原生 metadata 在注册前加 `#modid:` 前缀，所以只有 Quilt
原生 config 是按 Mod 隔离的。Orbit 的注册 identity 直接表达这一区别，不以 JAR 路径
一概隔离，也不把两份同名全局 config 都当成活动配置。

Fabric/Quilt 只有 metadata 选中的 nested JAR 才进入 Loader 内容。Quilt 同时实现
Fabric Loader 兼容 API，因此 Quilt JAR 内出现 `FabricLoaderImpl` 是 Quilt 的兼容能力，
不是第二个活动 Loader。进入同一个顶层
Mod 的成员共享加载单元中的 config、class、plugin 与 refmap 可见性；未声明成员不能
污染 Universe。Forge/NeoForge 同样先由 launcher/FML 选择运行时游戏内容和转换服务，
再注册 Mixin/Transformer，差异只存在于能力探测和注册资源，不存在于后续统一效果与
冲突模型。

现代 Forge 的 launch handler 明确暴露 runtime naming；NeoForge 的 GameLocator
选择 patched Minecraft 或 launcher 声明的 `srg` 分类运行时内容。Orbit 不模拟启动
过程，而是验证 init/sync 固定到 platform snapshot 的 classpath JAR 中内嵌
Minecraft 版本及实际类内容。
没有可靠 runtime game artifact 且磁盘基础游戏与转换目标不共享可观察类空间时，
FML provider 返回 readiness incomplete。

ModLauncher 9/10 的稳定转换入口是 Java `ServiceLoader` 注册
`ITransformationService`，由 `transformers()` 返回 `ITransformer`；目标来自
`ITransformer.Target`。ModLauncher 11 改变了部分 transformer ABI，因而 Orbit 探测
实际方法形状，不用版本号推断。Java 服务声明既可能来自 `META-INF/services`，也可能
来自 `module-info.class` 的 `Module.provides`，两者必须合并为同一个注册图。

NeoForge/FML 10 及当前源码已经改用
`ServiceLoader<ClassProcessor / ClassProcessorProvider>`。`ClassProcessor` 通过
`handlesClass` 选择类、`processClass` 修改类；官方 `SimpleClassProcessor`、
`SimpleMethodProcessor`、`SimpleFieldProcessor` 则以 `targets()` 暴露静态目标。
这是与旧 ModLauncher transformer 不同的 SPI，而不是同一逻辑的版本特判。Orbit 根据
运行时实际存在的 SPI 选择解释器：旧 Forge/NeoForge 走 ITransformer，当前 NeoForge
走 ClassProcessor；两者最终都归一化为相同 Target/Mutation/Effect 模型。

## Orbit 抽象

唯一 `AuditPolicy` 由共享 `(Loader, Minecraft 范围, Loader 范围)` 表选择 runtime ABI
profile、namespace alignment、Mixin 注册来源与转换 SPI 能力；`analyze` 只执行一遍扫描、
readiness、对齐、Mixin、Transformer 与冲突合成。Fabric 和 Quilt 共享 mapping-resource parser 与结构化
namespace 校验；Forge 和 NeoForge 共享 FML 能力分派，但由实际 runtime ABI 选择
ITransformer 或 ClassProcessor。后续统一输出
`NamespaceReport`、效果和冲突模型。`LoaderArtifactUnit` 表达 resolver 已选顶层内容及
活动 nested 成员。`ClassDefinitionId` 保留 loader unit、artifact、entry、原始/运行时
类名和内容哈希。所有 Mixin、Transformer、hard reference 与 duplicate-class 比较只
消费对齐后的 Class Universe。

`PluginDecision` 使用 Always/Never/Conditional/Unknown；config activation、Mixin
activation 与最终 finding activation 分层。Overwrite 合并先构造按优先级变化的
CandidateClassState；确定失败进入 unary risk，Plugin/namespace/定义歧义则分别进入
coverage 或 readiness。

这些抽象依赖资源格式、ABI 形状、实际 JAR 内容和生命周期顺序。版本边界只存在于
`orbit-compatibility`；audit 不复制边界、不另造 Loader 版本前缀、Mod 白名单或推测性
兜底。范围只选择解释能力，实际 Loader JAR 的 ABI 仍必须通过 readiness probe；Loader
改变内部选择策略但维持同一可观察能力时不需要复制一条分析路径。
