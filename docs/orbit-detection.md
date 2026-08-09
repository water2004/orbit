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

四种加载器不再各有一个只转发参数的 detector 类型。`detection/mod.rs` 保存一张
`ProfileDetector` 证据注册表：

```rust
ProfileDetector {
    loader,
    display_name,
    ProfileSignature { group, artifacts, main_class_markers, component_uids },
    strip_minecraft_prefix,
}
```

每一行都调用同一个 `profile::detect_profile_loader()`，`LoaderInfo.versions` 保留全部候选，
不在 detector 内提前取第一个。`detect_all()` 遍历注册表并按置信度降序返回；手动传入
`--modloader` 时，CLI 在边界解析为 `LoaderKind`，再通过 `find_by_kind()` 只运行对应
detector。

注册表只保存输入证据和规范化事实，不保存版本行为。随版本变化的能力统一由
`orbit-compatibility` 范围表选择；官方服务端落盘格式则由 `server/formats.rs` 的结构化
适配器解析并全部归一化为 `ServerRuntimeSpec`。

## 3. Launcher 布局与 profile 扫描

由 Orbit Launcher 管理的客户端与独立服务端使用严格适配器，不进入下述通用扫描：当实例目录同时包含
`orbit-launcher.toml` 与 schema 6 `orbit-launcher.lock` 时，`init`/`sync` 只读取 lock 的准确
Minecraft、Loader、classpath、工件路径与 SHA-256，并验证实际 JAR 内元数据。只存在其中
一个文件、schema 不匹配、路径/hash/身份不一致都会直接报错，不回退到 profile、文件名或
相邻目录猜测。正常的 add/install/audit 等命令仍只消费 `orbit.toml` 中的平台快照。

Orbit Launcher 客户端的 `<minecraft-directory>/instances/<实例>` 是它选择的隔离 game
directory；实例自己的 `minecraft.jar` 与 launcher manifest/lock 位于该目录，共享 classpath
仍引用仓库根的 `libraries/`。独立服务端的 lock 路径则严格相对于用户选择的服务端目录解析，
`server.jar`、`libraries/` 与 launcher manifest/lock 都属于该目录；二者由 lock 的 `kind`
明确区分，不根据目录内容猜测。
它不是 Mojang 规定的实例 manifest 格式，也不包含派生 `<实例>.json`。通用 HMCL/官方
Launcher 探测规则仅适用于没有 Orbit Launcher 标记的外部实例。

Minecraft JAR 内 `version.json` 的 `pack_version` 也按 Mojang 不可变的 `world_version`
范围选择唯一解析器，而不按 JSON 形状试错：`18w47b`（1913）至 `1.16.5`（2586）是单整数，
`20w45a`（2681）至 `1.21.8`（4440）是 `resource`/`data`，`25w31a`（4534）起是
resource/data major/minor 四字段；首个对应正式版为 `1.21.9`。未注册的版本空档或与范围
不符的结构直接报错。`java_version` 在 `21w19a`（2714）以前未写入该文件，对应范围明确为
Java 8；从该版本起缺失字段同样报错。

`LauncherLayout` 将常见启动器归一化成 profile、Minecraft JAR 搜索目录、共享
libraries 和组件列表：

- 标准/官方 launcher 的共享游戏根；
- HMCL 等使用的 `versions/<实例>` 隔离 game directory；
- Prism Launcher/MultiMC 的实例 `.minecraft` 或 `minecraft`，读取 `mmc-pack.json`；
- CurseForge profile（`minecraftinstance.json`）；
- GDLauncher 的 `instance/`（父目录 `instance.json`）；
- 带实际 version profile/JAR 的 standalone 目录；
- 含 `eula.txt` 或 `server.properties` 的 dedicated server。服务端 marker 的优先级高于
  通用 `versions/` 和 standalone，避免 Mojang bundler 生成的 `versions/` 被误判为
  客户端共享游戏根。

实现不会在上述列表中“第一个匹配就返回”。每个 probe 先产生带 `LayoutEvidence` 和
`LayoutPrecedence` 的候选：启动器自有实例 metadata 高于 dedicated server marker，
server marker 高于 isolated/shared/standalone 通用布局；同一优先级出现多个候选直接
报告歧义。这样优先级是可测试的领域规则，而不是函数调用顺序。

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

## 4. Dedicated server 运行时规范

服务端不伪造 launcher version profile，也不把启动脚本当作 shell/batch 程序执行。
`detection/server.rs` 只负责收集、合并和消歧 `ServerRuntimeCandidate`；
`detection/server/formats.rs` 解析官方本地安装格式。候选携带
`ServerLaunchFormat`（Fabric installer bootstrap、direct loader launch JAR、Forge
bootstrap shim 或 ModLauncher argument file），最终归一化成同一个
`ServerRuntimeSpec`：

| Loader | 权威本地规格 | 实际路径来源 |
|---|---|---|
| Fabric | 官方 server bootstrap JAR 的 `install.properties`，以及生成 launch JAR 的 manifest `Class-Path` | `game-version`、`fabric-loader-version`、真实 loader 元数据 |
| Quilt | `quilt-server-launch.jar` 的 manifest `Main-Class` / `Class-Path` | classpath 中声明 `quilt_loader` 的实际 JAR，以及 `quilt-server-launcher.properties` |
| Forge（当前） | 根目录 bootstrap shim 的 manifest 与 `bootstrap-shim.list` | 清单中的 Maven 坐标、相对路径和 SHA-256 |
| Forge（ModLauncher） | 安装器生成的当前平台 `unix_args.txt` / `win_args.txt` | `--fml.*` 参数、module path、legacy classpath 与安装器坐标目录 |
| NeoForge | 安装器生成的当前平台 `unix_args.txt` / `win_args.txt` | `--fml.mcVersion`、`--fml.neoForgeVersion` 和精确 classpath |

Fabric/Quilt 的 `server.jar` 若是 Mojang bundler，Orbit 读取
`META-INF/versions.list` 和 `META-INF/libraries.list`，验证清单哈希，并选择 loader
运行时实际展开的 `versions/<id>/<jar>`，不会把只负责解包的外层 server JAR 当成游戏
类空间。Forge bootstrap shim 的每一项同样按其清单 SHA-256 验证。

四种格式最终只输出 Minecraft 版本、loader/version、实际 Minecraft JAR、loader JAR
和完整 runtime classpath。`init`、loader 自动识别与 `sync` 都消费这一个结果；快照
之后的命令仍不接触探测规则。

下列情况直接报错，不按目录顺序或文件名猜测：

- 官方 launch spec 引用的 JAR 缺失、越出实例目录或 hash 不匹配；
- classpath 中没有唯一的 Fabric/Quilt loader 元数据；
- Forge/NeoForge 参数版本与安装坐标不一致；
- 同一服务端目录中存在多个不同的有效 loader/runtime；
- server 安装尚未生成实际 loader/game/runtime JAR。

## 5. `orbit init` 的选择规则

加载器选择顺序如下：

1. 显式 `--modloader` 始终优先，并验证名称是否受支持；
2. 未显式指定时，只自动接受唯一的 `Confidence::Certain` loader；
3. 没有确定结果时，交互模式要求用户选择加载器；
4. 加载器版本按“显式参数筛选实际候选 → 唯一检测版本 → 多候选交互”选择；
5. 使用 `--yes` 且无法确定版本时，必须显式提供
   `--modloader-version`，不会伪造版本。

`LoaderInfo.evidence` 保留命中的坐标、profile 文件名或 `mainClass` 标记，CLI 在
自动检测成功时显示这些证据。检测失败与“检测到某个版本”是两种可区分的结果。

## 6. Minecraft 版本检测

Dedicated server 先按上一节的运行时规范确定实际 game JAR；其它布局由
`platform_detection::detect_mc_versions(instance_dir)`（经 `init` API 转出）扫描布局声明的游戏 JAR 目录和
`libraries/com/mojang/minecraft`。两者都读取 `version.json` 并交给
`metadata::mojang::McVersion` 解析。返回值除 `id` 外还保留
world/protocol/pack/Java 版本和稳定版标志。

profile 的 `inheritsFrom` 或 Prism component 只用于筛选；最终仍必须找到并解析对应
真实 JAR。多个版本不会按扫描顺序取第一个。

`platform_detection::discover_platform_for_init()` 只把 init 已选择的版本作为消歧条件；
`platform_detection::rediscover_current_platform()` 不接受任何旧版本或旧路径参数，
供 sync 对账以及 migration planner 读取用户明确指定的目标实例。两者都会定位 loader Maven 目录并枚举其中的实际 JAR，Minecraft JAR
则以 JAR 内的 `version.json` 识别，均不假设文件名与版本相同。Fabric/Quilt loader
元数据必须可解析；所有能解析的 loader bundled 模块进入平台候选图。最终 Minecraft、
Loader、runtime JAR 的实际路径和 SHA-256，以及物理端才整体写入 `[platform]`。

`platform.rs` 不含上述规则。它只解析快照路径、校验 SHA-256、读取精确 JAR 元数据；
任一事实不成立都要求运行 `orbit sync`，不会按文件名、相邻目录、launcher profile
或旧值寻找替代项。

## 7. 已知边界

- 不解析启动器的私有数据库；只读取实例内稳定的 profile/组件/marker 文件和现有
  libraries。没有这些信息时明确报错；
- launcher profile 的 `mainClass` 仅是弱证据，不足以自动确定 loader 版本；
- dedicated server 只接受上述官方落盘规格；不会执行 `launch.sh`、`run.sh`、
  `run.bat` 或用户自定义 wrapper，也不会从进程列表猜测实际启动项；
- 多个 Minecraft、loader、loader version 或同级 Loader JAR 候选均视为歧义；
  交互 init 可让用户选择版本，sync/migrate 不猜测；
- 游戏 `version.json` 的 Java 信息既用于检测展示，也作为 resolver 注册 `java` 平台包的
  精确 feature；模组 feature 与 class major 再校验最低 Java。它不按 Minecraft 版本猜测，也不探测用户
  当前 shell 的 Java，因为安装目标应由实例版本决定；
- CurseForge 下载 provider 与 CurseForge launcher 布局是两个独立边界；前者需要 API
  Key，后者只读取本地实例 marker/profile，不需要网络。

## 8. 扩展检测策略

新增 loader 时需要同时完成：

1. 在 `ProfileDetector` 注册表添加唯一一行证据；
2. 若其官方 profile 或服务端落盘格式确实不同，增加只负责归一化输入的格式适配器；
3. 定义能够确定版本的强证据，并将猜测保留为低置信度；
4. 在 `metadata/`、`jar/` 和 `versions/` 接入对应格式；
5. 在 `orbit-compatibility` 添加有证据的能力范围；
6. 添加根目录、`versions/` 子目录、弱证据、范围边界和版本归一化测试。

不能用固定版本、空字符串或默认 `0.0.0` 代替检测失败。无法获得可复现所需信息时，
应要求用户显式输入。
