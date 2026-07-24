# Orbit 实例环境检测

> 实现位置：`orbit-core/src/detection/`、`orbit-core/src/metadata/version_profile.rs`
> 与 `orbit-core/src/init.rs`

## 1. 职责边界

检测层只回答两个问题：

1. 当前目录使用哪一种模组加载器，以及能否从 launcher profile 得到版本；
2. 当前实例的 Minecraft JAR 是否包含可解析的 `version.json`。

它不解析模组 JAR，不查询下载平台，也不替安装流程选择兼容版本。模组元数据由
`metadata/` 与 `jar/` 处理；平台版本由 provider 处理。

## 2. Loader detector

每种加载器实现同一个策略接口：

```rust
pub trait LoaderDetector: Send + Sync {
    fn name(&self) -> &'static str;
    fn loader_type(&self) -> ModLoader;
    fn detect(&self, instance_dir: &Path) -> Result<LoaderInfo, OrbitError>;
}
```

`LoaderDetectionService::new()` 当前注册 Fabric、Forge、NeoForge 和 Quilt 四个
detector。`detect_all()` 执行全部策略，并按置信度降序返回；手动传入
`--modloader` 时，CLI 通过 `find_by_name()` 只运行对应 detector。

## 3. Launcher profile 扫描

四个 detector 复用 `profile::detect_profile_loader()`。扫描范围为：

- 实例根目录中的 `*.json`；
- `versions/` 下每个直接子目录中的 `*.json`。

JSON 按 Minecraft Launcher version profile 解析，主要读取 `libraries[].name` 和
`mainClass`：

| Loader | 确定性 Maven 坐标 | 弱证据 `mainClass` 标记 |
|--------|-------------------|-------------------------|
| Fabric | `net.fabricmc:fabric-loader` | `fabricmc` |
| Forge | `net.minecraftforge:forge` | `minecraftforge` |
| NeoForge | `net.neoforged:neoforge` 或 `net.neoforged:forge` | `neoforged` |
| Quilt | `org.quiltmc:quilt-loader` | `quiltmc` |

找到 Maven 坐标时返回加载器版本和 `Confidence::Certain`。只命中 `mainClass` 时没有
足够信息确定版本，因此返回 `Confidence::Low`；没有证据时返回
`Confidence::None`。当前 detector 不产生 `Confidence::High`，该枚举值为其它检测
策略保留。

Forge/NeoForge profile 的坐标版本有时包含 Minecraft 前缀，例如
`1.21.1-52.0.0`。只有前半段确实是纯数字点分 Minecraft 版本时，检测层才将其归一化
为 `52.0.0`，避免破坏 `21.1.0-beta` 这样的正常预发布版本。

## 4. `orbit init` 的选择规则

加载器选择顺序如下：

1. 显式 `--modloader` 始终优先，并验证名称是否受支持；
2. 未显式指定时，只自动接受 `Confidence::Certain` 的最佳检测结果；
3. 没有确定结果时，交互模式要求用户选择加载器；
4. 加载器版本按“显式参数 → 检测版本 → 交互输入”选择；
5. 使用 `--yes` 且无法确定版本时，必须显式提供
   `--modloader-version`，不会伪造版本。

`LoaderInfo.evidence` 保留命中的坐标、profile 文件名或 `mainClass` 标记，CLI 在
自动检测成功时显示这些证据。检测失败与“检测到某个版本”是两种可区分的结果。

## 5. Minecraft 版本检测

`init::detect_mc_version(instance_dir)` 先扫描 `versions/` 的直接子目录，再回退扫描
实例根目录中的 JAR，读取其中的 `version.json`，并交给
`metadata::mojang::McVersion` 解析。返回值除 `id` 外还保留
world/protocol/pack/Java 版本和稳定版标志。

该检测只接受真实 `version.json`；无法检测时由 CLI 请求 `--mc-version` 或交互输入。
它不会从目录名猜版本，也不会把 loader profile 的 `inheritsFrom` 当作已经验证的游戏
JAR 版本。

## 6. 已知边界

- 只扫描根目录和 `versions/` 的一层子目录，不解析各启动器的私有配置数据库；
- launcher profile 的 `mainClass` 仅是弱证据，不足以自动确定 loader 版本；
- 多个加载器同时有确定证据时，当前按 detector 注册顺序稳定选择第一个结果，没有额外
  的冲突询问；
- 游戏 `version.json` 的 Java 信息用于检测展示；resolver 依据目标 Minecraft 版本
  注册 `java` 平台包，并用模组 feature 与 class major 校验最低 Java。它不探测用户
  当前 shell 的 Java，因为安装目标应由实例版本决定；
- CurseForge 是下载 provider 边界，与实例 loader 检测无关；启用它不会改变 loader
  profile 的检测规则。

## 7. 扩展检测策略

新增 loader 时需要同时完成：

1. 添加 `LoaderDetector` 实现；
2. 在 `LoaderDetectionService::new()` 注册；
3. 定义能够确定版本的强证据，并将猜测保留为低置信度；
4. 在 `metadata/`、`jar/` 和 `versions/` 接入对应格式；
5. 添加根目录、`versions/` 子目录、弱证据和版本归一化测试。

不能用固定版本、空字符串或默认 `0.0.0` 代替检测失败。无法获得可复现所需信息时，
应要求用户显式输入。
