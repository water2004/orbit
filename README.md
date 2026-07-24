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
- **☕ 字节码下限检查**：根据目标 Minecraft 与 JAR class major 校验最低 Java；该检查不宣称能证明 API、Mixin 或运行时行为完全兼容。
- **🚀 跨版本升级预检**：想升级 MC 主版本？`orbit check <version>` 可查询在线模组是否已有兼容版本。

---

## 🚀 快速开始

### 安装
根据你的操作系统，在release页面下载安装

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
| `orbit init <name>` | 初始化当前目录为 Orbit 项目，生成 `orbit.toml`，并**自动扫描接管**现有 `mods/` 里的文件。 |
| `orbit instances list` | 列出所有被 Orbit 托管的 MC 实例及其路径（当前/默认实例会有 `*` 标记）。 |
| `orbit instances default <name>`| 将指定实例设为全局默认。在任意目录下执行命令都将默认作用于它。 |
| `orbit instances remove <name>` | 从 Orbit 全局列表中移除对该实例的追踪（**绝不会**删除硬盘上的文件）。 |

### 2. 同步与更新 (Sync & Update)

*Orbit 严格区分本地状态同步与网络更新操作。*

| 命令 | 描述 |
| :--- | :--- |
| `orbit sync` | **本地状态双向对齐**。扫描实际的 `mods/` 文件夹，识别用户手动增删的 `.jar` 文件并更新记录。不下载 JAR；来源识别可能查询 Modrinth SHA-512 或 CurseForge fingerprint。 |
| `orbit outdated` | **检查过时模组（只读）**。联网检查所有已安装模组是否有新版本，并输出过时报告。不修改任何文件。 |
| `orbit upgrade [mod]` | **执行更新**。下载并替换可更新的 `.jar` 文件，并更新 `orbit.lock`。如果不带参数，则升级所有可升级的模组。 |

### 3. 模组管理 (CRUD)

| 命令 | 描述 |
| :--- | :--- |
| `orbit search <query>` | 在已配置来源中搜索模组；支持 Modrinth 与 CurseForge。 |
| `orbit info <mod>` | 查看模组详细信息（描述、作者、版本历史、前置依赖、端侧支持等）。无需安装，直接请求平台 API。 |
| `orbit add <mod>` | 添加新模组。支持自动查找、`mr:name`、`cf:name` 或 `file:./my-mod.jar`。使用 `--env client\|server` 标记端侧。 |
| `orbit install` | 根据 `orbit.toml` 和 `orbit.lock`，下载并补齐所有缺失的 `.jar` 文件。默认全量安装；使用 `--target server`/`--target client` 过滤端侧；使用 `--locked` 严格按 lock 文件还原，不发起网络解析。 |
| `orbit remove <mod>` | 卸载模组。删除对应的 `.jar` 文件并移除 `orbit.toml` 中的记录。 |
| `orbit purge <mod>` | **深度清理**。在 `remove` 的基础上，启发式搜索并交互式询问以**彻底删除** `config/` 下的配置文件。 |
| `orbit list` | 列出当前实例记录的所有模组及版本；支持 `--tree` 和 `--target`。 |

### 4. 导入、导出与进阶工具 (IO & Utility)

| 命令 | 描述 |
| :--- | :--- |
| `orbit import <file>` | 合并 TOML、导入安全 ZIP，或按 index/overrides 导入 mrpack，随后触发 `sync`。 |
| `orbit export [file.zip]` | 将清单、锁文件和校验通过的 JAR 打包为 ZIP；也可输出 mrpack。 |
| `orbit check <version>`| **跨版本升级预检**。检查当前安装的模组集合是否已经针对目标 MC 版本（如 `1.21`）发布了对应文件。 |
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
# 文件路径和哈希记录在 orbit.lock
"my-local-mod" = "1.0.0"
```

> **提示**：强烈建议将 `orbit.toml` 和 `orbit.lock` 一同纳入 Git 版本控制！结合 `orbit install --target server`，你可以在任何机器上一键还原完整的模组环境。

### CurseForge API Key

CurseForge Core API 不提供匿名访问。使用 `cf:` 或把 `curseforge` 加入
`[resolver].platforms` 前，任选一种方式配置：

```toml
# %APPDATA%/orbit/config.toml（Windows）
# ~/.orbit/config.toml（Linux）
[auth]
curseforge_api_key = "YOUR_API_KEY"
```

或设置环境变量 `ORBIT_CURSEFORGE_API_KEY`。API Key 需要按
[CurseForge 官方说明](https://support.curseforge.com/support/solutions/articles/9000208346-about-the-curseforge-api-and-how-to-apply-for-a-key)
申请；Orbit 不内置共享 Key。

---

## 🤝 贡献与反馈

欢迎提交 Issue 报告 bug，或者发起 Pull Request 改进 Orbit！

## 📄 License

MIT License. 
