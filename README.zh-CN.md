# Orbit 🪐

[English](README.md)

<p align="center">
  <img src="assets/orbit.svg" width="112" height="112" alt="Orbit 图标">
</p>

**面向 Minecraft Java Edition 的现代、非侵入式模组包管理器与原生工作区。**

Orbit 本体的目标没有改变：`orbit` 只管理模组包、真实 JAR 元数据、依赖图、可复现清单与
lock，不替代用户已有的启动器。它可以接管一个合法的现有实例，也可以管理 Launcher 新建的
隔离实例，并提供接近 `cargo` / `npm` 的明确同步、修复、升级和恢复语义。

同一仓库现在包含三个职责严格分离的程序：

- `orbit`：模组包管理器；只管理模组及依赖图。
- `orbit-launcher`：Minecraft 运行时管理器；负责官方 Minecraft/Loader 元数据、隔离实例、
  Mojang Java、Microsoft/离线/标准 Yggdrasil 账户、客户端启动、EULA 与服务端监督运行；
  不链接、不调用 Orbit，也不管理模组。
- `orbit-gui`：GPUI 原生桌面薄壳；不实现包管理或启动业务，只通过两个 CLI 的
  JSON/NDJSON/stdin 协议提供完整图形交互。

Launcher 已完整支持 Vanilla、Fabric、Quilt、Forge、NeoForge 的客户端与独立服务端安装，
不再是“首个基线”。项目自己的 Microsoft public-client ID 已内置，token 仍只进入操作系统
秘密存储。详细边界见 [Orbit 架构](docs/orbit-architecture.md)、
[Launcher 架构](docs/orbit-launcher-architecture.md) 和 [GUI 架构](docs/orbit-gui.md)。

---

## ✨ 核心特性

- **📂 非侵入式与多实例/服务器管理**：无需改变外部启动器结构。直接进入启动器实例或 Fabric、Quilt、Forge、NeoForge dedicated server 根目录即可初始化管理；Launcher 托管客户端位于 `instances/<实例>`，精确 `minecraft.jar` 属于实例，共享仓库只承载不可变 assets/libraries。
- **🔄 事实同步与显式修复**：`orbit sync` 联网识别本地 JAR 来源并如实重建 TOML/lock，但不选择版本或删包；需要改变包集合时由 `orbit fix` 展示完整方案并确认。
- **🧹 彻底的深度清理 (`purge`)**：卸载模组时一并清理 `config/` 目录下残留的配置文件，保持环境绝对纯净。
- **🌐 多来源**：支持 Modrinth、CurseForge 与本地 `file:` JAR；不同平台只负责候选发现，最终统一验证 JAR 并求解依赖。
- **🗃️ 按游戏版本隔离的本地版本库**：每个精确 Minecraft/Loader 分别保存远端快照与 JAR 分析数据库；批量检查 project 变更标记，未变化不重拉版本，变化时也只刷新当前游戏版本。全局 LRU JAR 缓存仍是独立的内容存储。
- **🧩 完整 Loader 语义**：Fabric、Quilt、Forge、NeoForge 先由各自适配器保真解析，再进入同一个规范化求解模型；支持端侧、软/硬依赖、`provides`、加载顺序、内嵌模组与 Jar-in-Jar。
- **🔎 可解释求解**：依赖原因直接参与 PubGrub 的真实传播和回溯；不会用第二次反事实求解或日志解析猜原因。
- **🧭 目标明确的完整方案选择**：`add` / `fix` 枚举标准 Pareto 极小包变更集合，`upgrade` / `outdated` 枚举标准版本 Pareto 极大 front；互不支配的方案全部请求选择。
- **📦 包级事务**：JAR 自声明的 `mod_id` 是包；同 ID 文件是版本候选。方案会同时展示升级、允许的依赖降级与未选包版本移除，确认后一次应用。
- **⏱️ 可观察长事务**：在线候选发现、JAR 下载/缓存校验/解析、离线求解、最终物化、audit 和便携包导出均提供强类型进度；Orbit ZIP 对已经压缩的 JAR 使用 Stored，并按真实字节显示与取消导出。
- **☕ 字节码下限检查**：根据目标 Minecraft 与 JAR class major 校验最低 Java；该检查不宣称能证明 API、Mixin 或运行时行为完全兼容。
- **🚀 联合跨版本迁移**：GUI 先分别冻结目标无关的 Orbit 模组包与 Launcher 游戏状态包；`orbit-launcher install --from` 创建目标运行时并恢复世界/设置，`migrate export --source-pack` 再针对真实目标运行时求完整依赖解，最后由 `orbit install` 精确物化模组。

---

## 🚀 快速开始

### 安装

Windows x64 用户可以从 release 页面下载
`orbit-<version>-x86_64.msi`。MSI 会把 Orbit 安装到
`%ProgramFiles%\Orbit\bin`；其中包含相邻的 `orbit`、`orbit-launcher` 和原生
`orbit-gui`，并创建开始菜单入口。安装向导默认勾选加入系统 `PATH`，也可以取消。
安装需要管理员权限。重复运行同一安装包会进入修改/修复/卸载界面；同版本的新构建
也能覆盖旧构建。卸载时可选择是否删除默认 AppData 中的 Orbit 配置和缓存。

从源码构建 MSI 的方法见
[Windows MSI](docs/windows-msi.md)。

Debian/Ubuntu amd64 使用三个独立 deb，不提供 MSI 式的交互式功能选择：

```bash
# 无图形服务端：只安装 Launcher
sudo apt install ./orbit-launcher_0.2.0-1_amd64.deb

# 服务端还需要管理模组时再安装 Orbit
sudo apt install ./orbit_0.2.0-1_amd64.deb

# 桌面完整套件：GUI 精确依赖同版本的两个 CLI
sudo apt install ./orbit_0.2.0-1_amd64.deb \
  ./orbit-launcher_0.2.0-1_amd64.deb \
  ./orbit-gui_0.2.0-1_amd64.deb
```

在无图形环境中安装 GUI 并不会阻止服务端工作，但会额外引入图形运行库，而且没有图形会话
也无法使用，因此服务端应安装 `orbit-launcher`，需要模组管理时再加 `orbit`。

三个程序使用同一个套件版本。tag 必须指向 `main` 且与全部公开 crate 版本一致，GitHub
Actions 才会同时发布 Windows MSI、三个 deb、
`SHA256SUMS` 和自动分类的 Release notes。构建与发布规则见
[Release 流程](docs/release-process.md)，deb 细节见
[Linux deb](docs/linux-deb.md)。

### Launcher 快速开始

```bash
# 一条命令创建并安装隔离客户端
orbit-launcher install --new fabric-1.21.1 \
  --kind client --minecraft 1.21.1 --loader fabric

# 在明确目录安装服务端
orbit-launcher install --new survival-server \
  --kind server --server-directory /srv/minecraft/survival \
  --minecraft latest-release --loader fabric
```

服务端启动前必须通过专用命令完整展示并接受 Minecraft EULA；Launcher 不会代替用户默认
同意。完整命令见 [Orbit Launcher CLI](docs/orbit-launcher-cli.md)。

Launcher 状态导出不绑定目标版本，只能由安装命令恢复：

```text
orbit-launcher --instance old-client export state.zip
orbit-launcher install --new new-client --kind client \
  --minecraft 1.21.1 --loader fabric --loader-version stable \
  --from state.zip --consume-from
```

客户端世界取自隔离实例的 `saves/`。独立服务端世界按 `server.properties` 的 `level-name`
定位（缺省 `world`）；目标 Minecraft 先生成自己的属性字段，Launcher 只迁移目标仍存在的
同名值。EULA 接受永不迁移。

### 体验丝滑工作流

```bash
# 1. 进入你现有的、混乱的 Minecraft 实例目录
cd "D:/Games/HMCL/instances/MySurvival/.minecraft"

# 2. 让 Orbit 接管这个目录，并命名为 "survival"
orbit init survival

# 3. 搜索并添加模组 (自动匹配当前 MC 版本与 Loader)
orbit add sodium
orbit add cf:238222   # CurseForge 数值 project ID；需要先配置 API Key
orbit add file:./my-local-mod.jar

# 给现有包增加/移除候选远端；Modrinth 使用 project ID，CurseForge 使用数值 project ID
orbit remote add sodium modrinth AANobbMI
orbit remote add sodium curseforge 394468
orbit remote list sodium
orbit remote remove sodium --index 2

# 4. 添加客户端专用模组 (开服时自动跳过)
orbit add zoomify --env client

# 修改已有包的过滤策略；auto 恢复跟随 JAR 声明
orbit env sodium client
orbit env sodium auto

# 从全部配置远端下载并列出 JAR 实际版本，然后立即求解并应用策略
orbit versions sodium
orbit constraint set sodium exact 0.6.13
orbit constraint set sodium any --string 'all; intersect not contains(i"beta")'
# 也可使用 any、greater-than、at-least、less-than、at-most，
# 或 range <下界> <上界> [--lower-bound ...] [--upper-bound ...]

# 5. 按 Pareto 极小变更修复依赖图；多个互不支配方案会要求选择
orbit fix

# 6. 按 orbit.lock 一键还原精确环境 (新电脑 clone 后)
orbit install

# 7. 一键部署到服务器 (自动过滤客户端模组)
orbit install --target server

# 8. 删除模组
orbit remove voxelmap

# 9. 彻底扬了不再使用的模组及其配置文件
orbit purge voxelmap
```

---

## 📖 命令参考 (CLI Reference)

Orbit 采用**目录优先**的上下文逻辑。命令会默认作用于当前所在目录的 `orbit.toml`，如果你在非项目目录执行命令，它将作用于你设置的**全局默认实例**（或通过 `-i <实例名>` 显式指定）。

`orbit` 与 `orbit-launcher` 均提供全局 `--language system|en|zh-CN`。Orbit 持久化的
`core.language` 默认为 `system`，显式参数优先；help、文本结果、进度、询问和错误会使用
所选语言。JSON/NDJSON/stdin 机器协议固定为严格 UTF-8，
schema、字段名、枚举码和错误码不随语言变化。Windows 控制台不要求切换 code page；管道中的
非法 UTF-8 会明确报协议错误，而不会被替换或静默忽略。

### 1. 实例管理 (Instance Management)

| 命令 | 描述 |
| :--- | :--- |
| `orbit init <name>` | 在合法游戏目录中初始化实例并记录本地事实；同 ID 存在多个实现时保留全部文件和远端，要求随后运行 `fix`。 |
| `orbit instances list` | 列出所有被 Orbit 托管的 MC 实例及其路径（当前/默认实例会有 `*` 标记）。 |
| `orbit instances register <name> <path>` | 注册已经具有一致 `orbit.toml` 与 `orbit.lock` 的实例；不探测、不补全状态。 |
| `orbit instances default <name>`| 将指定实例设为全局默认。在任意目录下执行命令都将默认作用于它。 |
| `orbit instances remove <name>` | 从 Orbit 全局列表中移除对该实例的追踪（**绝不会**删除硬盘上的文件）。 |

### 2. 同步与更新 (Sync & Update)

*Orbit 严格区分本地状态同步与网络更新操作。*

| 命令 | 描述 |
| :--- | :--- |
| `orbit sync` | **事实对账**。重新探测平台、扫描 `mods/`，并联网调用可用 provider 的批量哈希 API 恢复来源；重建 lock、补充 TOML，但不发现依赖候选、不求解、不删除 JAR。 |
| `orbit fix` | **依赖修复**。递归下载所有远端候选的 JAR 元数据，枚举相对当前 lock 的 Pareto 极小包变更方案；确认后安装入选包、删除未选包，并同步清理 TOML 与 lock。 |
| `orbit outdated [mod]` | **检查过时模组（只读）**。显示可行更新；更高候选受阻或没有适用 JAR 时同时给出原因。 |
| `orbit upgrade [mod]` | **执行更新**。单包模式必须让该包变新；方案也可包含依赖降级/替换/删除，确认后更新文件与 lock。 |

### 3. 模组管理 (CRUD)

| 命令 | 描述 |
| :--- | :--- |
| `orbit search <query>` | 在已配置来源中搜索模组；支持 Modrinth 与 CurseForge。 |
| `orbit info <mod>` | 查看模组详细信息（描述、作者、版本历史、前置依赖、端侧支持等）。无需安装，直接请求平台 API。 |
| `orbit add <mod>` | 添加新模组，并 Pareto 极小化对现有逻辑包的变更。支持自动查找、`mr:<project-id-or-search>`、`cf:<numeric-project-id>` 或 `file:./my-mod.jar`。可用 `--env client\|server\|both` 覆盖 JAR 声明。 |
| `orbit env <package> <client\|server\|both\|auto>` | 修改包环境过滤；`auto` 跟随 lock 中精确 JAR 的声明。 |
| `orbit install` | 严格校验平台快照和 lock，仅物化 lock 已记录的精确 JAR；绝不求解、修复、删包或改写 TOML/lock。 |
| `orbit remove <mod>` | 按 JAR `mod_id` 卸载包。删除其选中内容并移除 `orbit.toml`/lock 中的记录。 |
| `orbit purge <mod>` | **深度清理**。在 `remove` 的基础上，启发式搜索并交互式询问以**彻底删除** `config/` 下的配置文件。 |
| `orbit list` | 列出当前实例记录的所有模组及版本；支持 `--tree` 和 `--target`。 |
| `orbit remote add/remove/list` | 管理一个逻辑包的多个 `file` / Modrinth / CurseForge 候选远端；不能删除最后一个远端。 |
| `orbit versions <package>` | 刷新当前 Minecraft/Loader 作用域内发生变化的 project，复用已分析内容，并按 JAR 实际声明版本降序列出候选。 |
| `orbit constraint show/set` | 查看或原子应用数字核心策略与完整版本字符串规则；应用时按 Pareto 极小包变更求解并提交。 |

### 4. 导入、导出与进阶工具 (IO & Utility)

| 命令 | 描述 |
| :--- | :--- |
| `orbit import <file>` | 合并 TOML、导入安全 ZIP，或按 index/overrides 导入 mrpack，随后触发 `sync`。 |
| `orbit export [file.zip]` | 将清单、锁文件、校验通过的 JAR 与可移植配置打包为 ZIP；JAR 不重复压缩并报告真实字节进度，也可输出 mrpack。 |
| `orbit migrate check <目标实例目录>` | 先要求保留全部源包并对真实目标运行时联合求解；严格解不存在时才询问是否搜索标准 Pareto 极小删包方案。 |
| `orbit migrate export <目标实例目录>` | 复用同一严格优先迁移规划器，将目标 `orbit.toml`、`orbit.lock` 和模组配置写入空白目标实例；随后在目标运行 `orbit install`。 |
| `orbit migrate export <目标实例目录> --source-pack source.zip --consume-source-pack` | 从新建目标前冻结的源包求解；确认写入成功后删除临时源包。GUI 的升级/迁移流程使用此模式。 |
| `orbit audit` | **字节码兼容风险分析（只读）**。复用 Loader 实际选择的顶层/嵌套运行时内容，由 Fabric/Quilt/Forge/NeoForge 后端确定注册与运行时规则，再进入共享 ClassFile/效果/冲突流水线；默认输出分类摘要，`--output-format json` 或显式 `--report <path>` 保留完整 schema 5 证据。不下载 mapping，也不把依赖声明本身当作风险证据。 |

`mods/` 缺失是合法的空模组集合。init、sync、检查、失败或取消的操作以及空 lock 的 install
都不会补建该目录；只有选中的 JAR 真正物化时才创建。Loader 版本更新始终由 Launcher 执行
`instance configure --loader-version` 后再 `install`；跨 Minecraft/Loader 类型迁移则创建新实例，
GUI 在写入目标 Orbit 状态前先展示 `migrate check` 的完整包级方案。

迁移界面不预先展示“严格/软”策略。Orbit 总是先求保留全部源包的严格解；只有严格图无解时，
才在同一个 CLI 进程中显示原因并询问是否搜索标准 Pareto 极小删包 front。自动化可用
`--allow-removals` 表达同一许可；若软解仍有多个互不支配方案，仍必须明确选择。
| `orbit cache clean` | 清理 Orbit 在后台全局保存的 `.jar` 下载缓存，释放磁盘空间。 |

---

## ⚙️ 工作原理：`orbit.toml` & `orbit.lock`

每一个被 Orbit 接管的 `.minecraft` 目录下都会生成两个文件。`orbit.toml`
声明完整的受管逻辑包集合和每个包的全部候选远端；所有选中顶层包都有地位相同的
`[packages.<mod_id>]`，不区分根包与传递包。`orbit.lock` 只锁定实际版本、内容校验值、
JAR 元数据和能够恢复该精确内容的工件来源。相同字节跨来源按哈希合并，同版本不同
字节保持为不同候选；哈希不会作为用户界面中的包名或选项名称。两者都应纳入版本控制。

```toml
[project]
name = "survival"
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.15.7"

[platform]
minecraft_jar = { path = "../../1.20.1/1.20.1.jar", sha256 = "..." }
loader_jar = { path = "../../libraries/net/fabricmc/fabric-loader/0.15.7/fabric-loader-0.15.7.jar", sha256 = "..." }
runtime_jars = [
  { path = "../../libraries/net/fabricmc/intermediary/1.20.1/intermediary-1.20.1.jar", sha256 = "..." },
]
physical_environment = "client"

[resolver]
catalogs = ["modrinth"]

[packages]
# 一个包可同时声明多个远端；包身份和版本始终从下载后的 JAR 读取
sodium = { version = "^0.5", string = 'all; intersect not contains(i"beta")', remotes = [
  { type = "modrinth", project_id = "AANobbMI" },
  { type = "curseforge", project_id = 394468 },
] }
lithium = { version = ">=0.11 <0.14", remotes = [
  { type = "modrinth", project_id = "MODRINTH_PROJECT_ID" },
] }

# 客户端专用
"inventory-hud" = { version = "*", env = "client", remotes = [
  { type = "modrinth", project_id = "MODRINTH_PROJECT_ID" },
] }

# 本地文件也是同一远端模型
"my-local-mod" = { version = "1.0.0", remotes = [
  { type = "file", path = "../sources/my-local-mod.jar" },
] }
```

`version` 只描述数字核心，因此 `=1.2.3` 匹配数字核心为 `1.2.3` 的全部 Loader 合法表示；
`-alpha` 等作者文本必须由 `string` 表达，不能混入数字操作数。完整表示仍是不同求解方案，
但数字核心相同就具有相同的升级与 Pareto 优先级。

可选 `string` 从 `all` 或 `none` 开始，按顺序对 **完整 JAR 声明版本字符串** 执行
`intersect [not]`、`union [not]` 和整体 `complement`；前缀、数字、分隔符、限定词和构建文本
都不会被裁掉。引号字符串精确且区分大小写，`i"text"` 不区分大小写。Orbit 不给作者字符串
预设稳定版、测试版等含义。`orbit add` 只为新建请求包默认排除不区分大小写的 `beta` 与
`snapshot`，绝不改写已有项。数字核心允许任意段。Fabric/Quilt 的不透明 Loader 版本只
旁路 `version`，仍执行 `string` 并给出警告；Forge/NeoForge 则保持声明版本必须以数字开头
的 Loader 规则。

> **提示**：强烈建议将 `orbit.toml` 和 `orbit.lock` 一同纳入 Git 版本控制！结合 `orbit install --target server`，你可以在任何机器上一键还原完整的模组环境。

### CurseForge API Key

CurseForge provider 不支持匿名或降级运行。使用 `cf:`、把 `curseforge` 加入
`[resolver].catalogs`、为包添加 CurseForge 远端，或者操作含 CurseForge 远端的实例前，
必须任选一种方式
配置 API Key：

```toml
# Windows system 布局：%APPDATA%/orbit/config.toml
# Linux system 布局：$XDG_CONFIG_HOME/orbit/config.toml
#   （未设置 XDG_CONFIG_HOME 时为 $HOME/.config/orbit/config.toml）
[auth]
curseforge_api_key = "YOUR_API_KEY"
```

也可以不手工编辑 TOML：

```powershell
orbit config set auth.curseforge-api-key YOUR_API_KEY
orbit config set cache.capacity-mib 2048
orbit config set repository.dir D:/OrbitRepository
orbit config list
```

密钥在 `config get/list` 和 JSON 输出中只显示 `<redacted>`；由于命令行可能进入 shell
历史，无人值守环境仍建议使用环境变量。

或设置环境变量 `ORBIT_CURSEFORGE_API_KEY`。API Key 需要按
[CurseForge 官方说明](https://support.curseforge.com/support/solutions/articles/9000208346-about-the-curseforge-api-and-how-to-apply-for-a-key)
申请；Orbit 不内置共享 Key。Key 同时用于 Core API 与 CurseForge CDN 下载，只在
运行时保存，不写入 `orbit.toml` 或 `orbit.lock`。

也可以用全局 `--config <file>`、`--cache-dir <directory>` 与
`--repository-dir <directory>` 传入精确路径，或用
`--data-layout executable` 将 `config.toml`、`instances.toml` 和 `cache/` 放在
可执行文件旁。完整跨平台规则见
[全局配置与运行路径](docs/orbit-global-config.md)。

---

## 🤝 贡献与反馈

欢迎提交 Issue 报告 bug，或者发起 Pull Request 改进 Orbit！

## 📄 License

MIT License. 
