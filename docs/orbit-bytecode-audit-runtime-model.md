# Orbit audit 运行时模型研究记录

本文记录 `orbit-bytecode-audit` 的实现依据。它描述可跨版本观察的协议和生命周期，
不复制上游实现，也不把某个 Minecraft、Loader 或 Mod 版本写进判断表。

## 研究范围

本轮实际检查了以下上游源码：

| 项目 | revision |
|---|---|
| FabricMC/fabric-loader | `0.19.3` / `35b0b1c0268eb5f9d377322db491b0bb436541a8`，并比较 `0.16.14`、`0.17.3`、`0.18.6`、`0.19.1`、`0.19.2` |
| SpongePowered/Mixin | `releases/0.8.7` / `4053421aa10aaac6127d969028a29c94fe3054f6` |
| FabricMC/tiny-remapper | `7834504ce1be97df03e99723bef456e40241c607` |
| FabricMC/mapping-io | `06f4ec3f872d7e9b6643919c4c48059b911f97c2` |
| McModLauncher/modlauncher | `901c6ea849ae21ee7d464cd97113e77a6101a734` |
| neoforged/FancyModLoader | `b6b853518f4c04ac743b83d68606aac06bf72545` |
| MinecraftForge/MinecraftForge | `66d4d888eb9f560a35cd3cc8642f5d8f161fba3d` |

源码只用于确认行为。Orbit 的 Rust parser、投影器和抽象解释器均为独立实现。

## 稳定不变量

Fabric Loader 的 `MappingConfiguration` 依据 mapping 内容决定目标 namespace；生产
环境中有实际类映射时运行时游戏类进入 intermediary，没有类映射时保持 official。
`MinecraftGameProvider` 在把游戏类加入运行 classpath 前完成该转换。稳定边界是
mapping namespace/内容和转换发生顺序，不是缓存目录名、内部 Java 类名或版本号。

Tiny v1/v2 是可探测资源格式。Orbit 读取当前 classpath 已有的 mapping，按 Minecraft
类名对 namespace 做精确覆盖匹配，再投影完整内部 Class Universe。它不需要生成可运行
JAR，也不下载供人阅读的 mapping。无法唯一确定输入 namespace 时 readiness 失败。

Mixin 先由 Loader 注册 config，再按 config package/side 展开 Mixin，随后对每个
target/mixin 对调用 `IMixinConfigPlugin.shouldApplyMixin`。因此 plugin 结果必须逐项
求值，而且只在所有路径可证明时才可接受或拒绝。未知运行时状态不能解释成 false。
preApply/postApply 发生在类节点转换阶段，静态分析无法证明的修改属于 coverage。

Fabric/Quilt 只有 metadata 选中的 nested JAR 才进入 Loader 内容。进入同一个顶层
Mod 的成员共享加载单元中的 config、class、plugin 与 refmap 可见性；未声明成员不能
污染 Universe。Forge/NeoForge 同样先由 launcher/FML 选择运行时游戏内容和转换服务，
再注册 Mixin/Transformer，差异只存在于能力探测和注册资源，不存在于后续统一效果与
冲突模型。

现代 Forge 的 launch handler 明确暴露 runtime naming；NeoForge 的 GameLocator
选择 patched Minecraft 或 launcher 声明的 `srg` 分类运行时内容。Orbit 不模拟启动
过程，而是验证 init/sync 固定到 platform snapshot 的 classpath JAR 中内嵌
Minecraft 版本及实际类内容。
没有可靠 runtime game artifact 且磁盘基础游戏与转换目标不共享可观察类空间时，
ModLauncher provider 返回 readiness incomplete。

## Orbit 抽象

`RuntimeEnvironmentProvider` 将 Fabric/Quilt 与 Forge/NeoForge 的 capability probe
隔离；输出统一 `NamespaceReport`。`LoaderArtifactUnit` 表达 resolver 已选顶层内容及
活动 nested 成员。`ClassDefinitionId` 保留 loader unit、artifact、entry、原始/运行时
类名和内容哈希。所有 Mixin、Transformer、hard reference 与 duplicate-class 比较只
消费对齐后的 Class Universe。

`PluginDecision` 使用 Always/Never/Conditional/Unknown；config activation、Mixin
activation 与最终 finding activation 分层。Overwrite 合并先构造按优先级变化的
CandidateClassState；确定失败进入 unary risk，Plugin/namespace/定义歧义则分别进入
coverage 或 readiness。

这些抽象依赖资源格式、ABI 形状、实际 JAR 内容和生命周期顺序，未引用 Minecraft
版本阈值、Loader 版本前缀或 Mod 白名单。
