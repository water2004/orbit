# Orbit 实例环境检测

> 实现位置：`orbit-core/src/platform_detection.rs`、`orbit-core/src/launcher.rs`、
> `orbit-core/src/detection/` 与 `orbit-core/src/init.rs`

## 1. 职责边界

检测层只回答四个问题：

1. 当前目录是不是受支持的实际 Minecraft game directory；
2. launcher 当前选择了哪些 Minecraft/loader 候选；
3. 哪两个实际 Minecraft/loader JAR 对应该实例；
4. launcher 选择了哪些其余 runtime JAR，以及能否确定物理端。

生产调用边界是强制的：只有 `orbit init` 和 `orbit sync` 可以进入
`platform_detection`。其它命令只能消费 `orbit.toml [platform]` 的精确快照，不得
调用 `LauncherLayout`、loader detector 或目录候选扫描。

检测层不解析普通模组 JAR，不查询下载平台，也不替安装流程选择兼容版本。模组元数据由
`metadata/` 与 `jar/` 处理；平台版本由 provider 处理。

## 2. Loader detector

每种加载器实现同一个策略接口：

```rust
pub trait LoaderDetector: Send + Sync {
    fn name(&self) -> &'static str;
    fn loader_type(&self) -> ModLoader;
    fn detect(
        &self,
        instance_dir: &Path,
        mc_version: Option<&str>,
    ) -> Result<LoaderInfo, OrbitError>;
}
```

`LoaderDetectionService::new()` 当前注册 Fabric、Forge、NeoForge 和 Quilt 四个
detector。`LoaderInfo.versions` 保留全部候选，不在 detector 内提前取第一个。
`detect_all()` 执行全部策略，并按置信度降序返回；手动传入
`--modloader` 时，CLI 通过 `find_by_name()` 只运行对应 detector。

## 3. Launcher 布局与 profile 扫描

`LauncherLayout` 将常见启动器归一化成 profile、Minecraft JAR 搜索目录、共享
libraries 和组件列表：

- 标准/官方 launcher 的共享游戏根；
- HMCL 等使用的 `versions/<实例>` 隔离 game directory；
- Prism Launcher/MultiMC 的实例 `.minecraft` 或 `minecraft`，读取 `mmc-pack.json`；
- CurseForge profile（`minecraftinstance.json`）；
- GDLauncher 的 `instance/`（父目录 `instance.json`）；
- 带实际 version profile/JAR 的 standalone 目录和 dedicated server marker。

空目录、任意目录和只有 `mods/` 的目录不是合法实例。隔离目录只读取当前
`versions/<实例>` 的 profile，不扫描 sibling 实例。

四个 detector 复用 `profile::detect_profile_loader()`，只消费 `LauncherLayout`
给出的 profile/组件。若提供 Minecraft 版本，`inheritsFrom` 指向其它版本的 profile
不会进入候选。

JSON 按 Minecraft Launcher version profile 解析，主要读取 `libraries[].name` 和
`mainClass`：

| Loader | 确定性 Maven 坐标 | 弱证据 `mainClass` 标记 |
|--------|-------------------|-------------------------|
| Fabric | `net.fabricmc:fabric-loader` | `fabricmc` |
| Forge | `net.minecraftforge:forge` | `minecraftforge` |
| NeoForge | `net.neoforged:neoforge` 或 `net.neoforged:forge` | `neoforged` |
| Quilt | `org.quiltmc:quilt-loader` | `quiltmc` |

Prism/MultiMC 同时识别 component UID：
`net.fabricmc.fabric-loader`、`net.minecraftforge`、`net.neoforged` 和
`org.quiltmc.quilt-loader`。

找到 Maven 坐标时返回加载器版本和 `Confidence::Certain`。只命中 `mainClass` 时没有
足够信息确定版本，因此返回 `Confidence::Low`；没有证据时返回
`Confidence::None`。

Forge/NeoForge profile 的坐标版本有时包含 Minecraft 前缀，例如
`1.21.1-52.0.0`。只有前半段确实是纯数字点分 Minecraft 版本时，检测层才将其归一化
为 `52.0.0`，避免破坏 `21.1.0-beta` 这样的正常预发布版本。

## 4. `orbit init` 的选择规则

加载器选择顺序如下：

1. 显式 `--modloader` 始终优先，并验证名称是否受支持；
2. 未显式指定时，只自动接受唯一的 `Confidence::Certain` loader；
3. 没有确定结果时，交互模式要求用户选择加载器；
4. 加载器版本按“显式参数筛选实际候选 → 唯一检测版本 → 多候选交互”选择；
5. 使用 `--yes` 且无法确定版本时，必须显式提供
   `--modloader-version`，不会伪造版本。

`LoaderInfo.evidence` 保留命中的坐标、profile 文件名或 `mainClass` 标记，CLI 在
自动检测成功时显示这些证据。检测失败与“检测到某个版本”是两种可区分的结果。

## 5. Minecraft 版本检测

`platform_detection::detect_mc_versions(instance_dir)`（由 `init` API 转出）只扫描布局声明的游戏 JAR 目录和
`libraries/com/mojang/minecraft`，读取 `version.json` 并交给
`metadata::mojang::McVersion` 解析。返回值除 `id` 外还保留
world/protocol/pack/Java 版本和稳定版标志。

profile 的 `inheritsFrom` 或 Prism component 只用于筛选；最终仍必须找到并解析对应
真实 JAR。多个版本不会按扫描顺序取第一个。

`platform_detection::discover_platform_for_init()` 只把 init 已选择的版本作为消歧条件；
`platform_detection::rediscover_current_platform()` 不接受任何旧版本或旧路径参数，
只供 sync 对账使用。两者都会定位 loader Maven 目录并枚举其中的实际 JAR，Minecraft JAR
则以 JAR 内的 `version.json` 识别，均不假设文件名与版本相同。Fabric/Quilt loader
元数据必须可解析；所有能解析的 loader bundled 模块进入平台候选图。最终 Minecraft、
Loader、runtime JAR 的实际路径和 SHA-256，以及物理端才整体写入 `[platform]`。

`platform.rs` 不含上述规则。它只解析快照路径、校验 SHA-256、读取精确 JAR 元数据；
任一事实不成立都要求运行 `orbit sync`，不会按文件名、相邻目录、launcher profile
或旧值寻找替代项。

## 6. 已知边界

- 不解析启动器的私有数据库；只读取实例内稳定的 profile/组件/marker 文件和现有
  libraries。没有这些信息时明确报错；
- launcher profile 的 `mainClass` 仅是弱证据，不足以自动确定 loader 版本；
- 多个 Minecraft、loader、loader version 或同级 Loader JAR 候选均视为歧义；
  交互 init 可让用户选择版本，sync 不猜测；
- 游戏 `version.json` 的 Java 信息用于检测展示；resolver 依据目标 Minecraft 版本
  注册 `java` 平台包，并用模组 feature 与 class major 校验最低 Java。它不探测用户
  当前 shell 的 Java，因为安装目标应由实例版本决定；
- CurseForge 下载 provider 与 CurseForge launcher 布局是两个独立边界；前者需要 API
  Key，后者只读取本地实例 marker/profile，不需要网络。

## 7. 扩展检测策略

新增 loader 时需要同时完成：

1. 添加 `LoaderDetector` 实现；
2. 在 `LoaderDetectionService::new()` 注册；
3. 定义能够确定版本的强证据，并将猜测保留为低置信度；
4. 在 `metadata/`、`jar/` 和 `versions/` 接入对应格式；
5. 添加根目录、`versions/` 子目录、弱证据和版本归一化测试。

不能用固定版本、空字符串或默认 `0.0.0` 代替检测失败。无法获得可复现所需信息时，
应要求用户显式输入。
