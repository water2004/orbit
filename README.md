# Orbit 🪐

**The Modern, Non-intrusive Package Manager for Minecraft Mods.**

Orbit 是一个专为 Minecraft 打造的现代化命令行模组包管理器。它不试图替代启动器（如 HMCL, Prism Launcher 或 CurseForge），而是作为一个强大的“智能管家”完美融入你的现有工作流。

无论你是跨目录管理数十个整合包的硬核玩家，还是需要严格进行版本控制的模组开发者，Orbit 都能为你带来类似 `npm` 或 `cargo` 般优雅的模组管理体验。

---

## ✨ 核心特性

- **📂 非侵入式与多实例管理**：无需改变原有启动器结构。直接 `cd` 进入任意 `.minecraft` 目录即可初始化管理。
- **🔄 拥抱混乱的双向同步**：手动往 `mods` 文件夹拖入了新 mod？启动器自动删除了文件？只需 `orbit sync`，Orbit 会识别变更并对齐状态。
- **🧹 彻底的深度清理 (`purge`)**：卸载模组时一并清理 `config/` 目录下残留的配置文件，保持环境绝对纯净。
- **🌐 多来源**：支持 Modrinth、CurseForge 与本地 `file:` JAR；不同平台只负责候选发现，最终统一验证 JAR 并求解依赖。
- **🧩 完整 Loader 语义**：Fabric、Quilt、Forge、NeoForge 共享同一解析与求解路径，支持端侧、软/硬依赖、`provides`、加载顺序、内嵌模组与 Jar-in-Jar。
- **🔎 可解释求解**：依赖原因直接参与 PubGrub 的真实传播和回溯；不会用第二次反事实求解或日志解析猜原因。
- **🧭 完整方案选择**：枚举完整 Pareto front；被全面更新方案支配的组合不会出现，真正的升降级权衡才请求选择。
- **📦 包级事务**：JAR 自声明的 `mod_id` 是包；同 ID 文件是版本候选。方案会同时展示升级、允许的依赖降级与未选包版本移除，确认后一次应用。
- **⏱️ 可观察长事务**：在线候选发现、JAR 下载/缓存校验/解析、离线求解和最终物化分阶段显示；下载阶段提供精确完成数，求解阶段按新发现的搜索 run/probe 动态扩展总量。
- **☕ 字节码下限检查**：根据目标 Minecraft 与 JAR class major 校验最低 Java；该检查不宣称能证明 API、Mixin 或运行时行为完全兼容。
- **🚀 跨版本升级预检**：想升级 MC 主版本？`orbit check <version>` 可查询在线模组是否已有兼容版本。

---

## 🚀 快速开始

### 安装

Windows x64 用户可以从 release 页面下载
`orbit-<version>-x86_64.msi`。MSI 会把 Orbit 安装到
`%ProgramFiles%\Orbit\bin`；安装向导默认勾选加入系统 `PATH`，也可以取消。
安装需要管理员权限。重复运行同一安装包会进入修改/修复/卸载界面；同版本的新构建
也能覆盖旧构建。卸载时可选择是否删除默认 AppData 中的 Orbit 配置和缓存。

从源码构建 MSI 的方法见
[Windows MSI](docs/windows-msi.md)。

### 体验丝滑工作流

```bash
# 1. 进入你现有的、混乱的 Minecraft 实例目录
cd "D:/Games/HMCL/instances/MySurvival/.minecraft"

# 2. 让 Orbit 接管这个目录，并命名为 "survival"
orbit init survival

# 3. 搜索并添加模组 (自动匹配当前 MC 版本与 Loader)
orbit add sodium
orbit add cf:jei   # 需要先配置 CurseForge API Key
orbit add file:./my-local-mod.jar

# 4. 添加客户端专用模组 (开服时自动跳过)
orbit add zoomify --env client

# 5. 一键还原依赖环境 (新电脑 clone 后)
orbit install

# 6. 一键部署到服务器 (自动剔除客户端模组)
orbit install --target server --locked

# 7. 删除模组
orbit remove voxelmap

# 8. 彻底扬了不再使用的模组及其配置文件
orbit purge voxelmap
```

---

## 📖 命令参考 (CLI Reference)

Orbit 采用**目录优先**的上下文逻辑。命令会默认作用于当前所在目录的 `orbit.toml`，如果你在非项目目录执行命令，它将作用于你设置的**全局默认实例**（或通过 `-i <实例名>` 显式指定）。

### 1. 实例管理 (Instance Management)

| 命令 | 描述 |
| :--- | :--- |
| `orbit init <name>` | 在合法游戏目录中初始化实例，扫描现有 `mods/`；同 ID 文件作为候选求解，确认后移除未选版本。 |
| `orbit instances list` | 列出所有被 Orbit 托管的 MC 实例及其路径（当前/默认实例会有 `*` 标记）。 |
| `orbit instances default <name>`| 将指定实例设为全局默认。在任意目录下执行命令都将默认作用于它。 |
| `orbit instances remove <name>` | 从 Orbit 全局列表中移除对该实例的追踪（**绝不会**删除硬盘上的文件）。 |

### 2. 同步与更新 (Sync & Update)

*Orbit 严格区分本地状态同步与网络更新操作。*

| 命令 | 描述 |
| :--- | :--- |
| `orbit sync` | **本地状态双向对齐**。重新探测 Minecraft/loader 工件并扫描 `mods/`，同 ID 文件统一求解并确认清理未选版本。不下载 JAR；来源识别可能查询平台哈希接口。 |
| `orbit outdated [mod]` | **检查过时模组（只读）**。显示可行更新；更高候选受阻或没有适用 JAR 时同时给出原因。 |
| `orbit upgrade [mod]` | **执行更新**。单包模式必须让该包变新；方案也可包含依赖降级/替换/删除，确认后更新文件与 lock。 |

### 3. 模组管理 (CRUD)

| 命令 | 描述 |
| :--- | :--- |
| `orbit search <query>` | 在已配置来源中搜索模组；支持 Modrinth 与 CurseForge。 |
| `orbit info <mod>` | 查看模组详细信息（描述、作者、版本历史、前置依赖、端侧支持等）。无需安装，直接请求平台 API。 |
| `orbit add <mod>` | 添加新模组。支持自动查找、`mr:name`、`cf:name` 或 `file:./my-mod.jar`。使用 `--env client\|server` 标记端侧。 |
| `orbit install` | 重新探测实际平台后按 `orbit.toml`/lock 补齐缺失 JAR。Minecraft 版本变化时要求先 sync；loader 版本变化交给真实依赖分析。 |
| `orbit remove <mod>` | 卸载模组。删除对应的 `.jar` 文件并移除 `orbit.toml` 中的记录。 |
| `orbit purge <mod>` | **深度清理**。在 `remove` 的基础上，启发式搜索并交互式询问以**彻底删除** `config/` 下的配置文件。 |
| `orbit list` | 列出当前实例记录的所有模组及版本；支持 `--tree` 和 `--target`。 |

### 4. 导入、导出与进阶工具 (IO & Utility)

| 命令 | 描述 |
| :--- | :--- |
| `orbit import <file>` | 合并 TOML、导入安全 ZIP，或按 index/overrides 导入 mrpack，随后触发 `sync`。 |
| `orbit export [file.zip]` | 将清单、锁文件和校验通过的 JAR 打包为 ZIP；也可输出 mrpack。 |
| `orbit check <version>`| **跨版本升级预检**。检查当前安装的模组集合是否已经针对目标 MC 版本（如 `1.21`）发布了对应文件。 |
| `orbit audit` | **字节码兼容风险分析（只读）**。默认输出有界摘要；`--format json` 或显式 `--report <path>` 保留完整证据。直接分析当前实例的 Mod、Minecraft、Loader 与运行时 JAR，不下载 mapping，也不把依赖声明当作风险证据。 |
| `orbit cache clean` | 清理 Orbit 在后台全局保存的 `.jar` 下载缓存，释放磁盘空间。 |

---

## ⚙️ 工作原理：`orbit.toml` & `orbit.lock`

每一个被 Orbit 接管的 `.minecraft` 目录下都会生成两个文件。`orbit.toml`
声明期望状态；`orbit.lock` 锁定实际版本、来源、校验值和依赖树。在线依赖可按锁文件
还原，本地 `file:` 依赖则需要保留对应 JAR，或通过 `orbit export` 一并分发。两者都应
纳入版本控制。

```toml
[project]
name = "survival"
mc_version = "1.20.1"
modloader = "fabric"
modloader_version = "0.15.7"

[platform]
minecraft_jar = { path = "../../1.20.1/1.20.1.jar", sha256 = "..." }
loader_jar = { path = "../../libraries/net/fabricmc/fabric-loader/0.15.7/fabric-loader-0.15.7.jar", sha256 = "..." }

[resolver]
platforms = ["modrinth"]
prerelease = false

[dependencies]
# 平台托管模组
sodium = "^0.5"
lithium = ">=0.11 <0.14"

# 客户端专用
"inventory-hud" = { version = "*", env = "client" }

# `orbit add file:./my-local-mod.jar` 后仍只声明 mod_id 与版本；
# 模组文件路径和哈希记录在 orbit.lock；上面的 platform 路径只描述游戏/loader 工件
"my-local-mod" = "1.0.0"
```

> **提示**：强烈建议将 `orbit.toml` 和 `orbit.lock` 一同纳入 Git 版本控制！结合 `orbit install --target server`，你可以在任何机器上一键还原完整的模组环境。

### CurseForge API Key

CurseForge provider 不支持匿名或降级运行。使用 `cf:`、把 `curseforge` 加入
`[resolver].platforms`，或者操作含 CurseForge 锁定包的实例前，必须任选一种方式
配置 API Key：

```toml
# Windows system 布局：%APPDATA%/orbit/config.toml
# Linux system 布局：$XDG_CONFIG_HOME/orbit/config.toml
#   （未设置 XDG_CONFIG_HOME 时为 $HOME/.config/orbit/config.toml）
[auth]
curseforge_api_key = "YOUR_API_KEY"
```

或设置环境变量 `ORBIT_CURSEFORGE_API_KEY`。API Key 需要按
[CurseForge 官方说明](https://support.curseforge.com/support/solutions/articles/9000208346-about-the-curseforge-api-and-how-to-apply-for-a-key)
申请；Orbit 不内置共享 Key。Key 同时用于 Core API 与 CurseForge CDN 下载，只在
运行时保存，不写入 `orbit.toml` 或 `orbit.lock`。

也可以用全局 `--config <file>` 与 `--cache-dir <directory>` 传入精确路径，或用
`--data-layout executable` 将 `config.toml`、`instances.toml` 和 `cache/` 放在
可执行文件旁。完整跨平台规则见
[全局配置与运行路径](docs/orbit-global-config.md)。

---

## 🤝 贡献与反馈

欢迎提交 Issue 报告 bug，或者发起 Pull Request 改进 Orbit！

## 📄 License

MIT License. 
