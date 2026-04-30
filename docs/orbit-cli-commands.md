# Orbit CLI 命令设计文档

> 本文档是 Orbit CLI 开发的**唯一参照标准**——每个命令的行为、参数、错误条件必须与此文档一致。

---

## 目录

1. [全局约定](#1-全局约定)
2. [实例管理](#2-实例管理)
   - [orbit init](#orbit-init)
   - [orbit instances list](#orbit-instances-list)
   - [orbit instances default](#orbit-instances-default)
   - [orbit instances remove](#orbit-instances-remove)
3. [模组增删](#3-模组增删)
   - [orbit add](#orbit-add)
   - [orbit install](#orbit-install)
   - [orbit remove](#orbit-remove)
   - [orbit purge](#orbit-purge)
4. [同步与更新](#4-同步与更新)
   - [orbit sync](#orbit-sync)
   - [orbit outdated](#orbit-outdated)
   - [orbit upgrade](#orbit-upgrade)
5. [查询与搜索](#5-查询与搜索)
   - [orbit search](#orbit-search)
   - [orbit info](#orbit-info)
   - [orbit list](#orbit-list)
6. [导入、导出与工具](#6-导入导出与工具)
   - [orbit import](#orbit-import)
   - [orbit export](#orbit-export)
   - [orbit check](#orbit-check)
   - [orbit cache clean](#orbit-cache-clean)
7. [核心动词速查](#7-核心动词速查)

---

## 1. 全局约定

### 1.1 项目上下文解析

Orbit 所有命令遵循三级上下文优先级：

1. 当前工作目录下存在 `orbit.toml` → 使用当前项目
2. 不存在 → 使用 `~/.orbit/instances.toml` 中记录的**全局默认实例**
3. 用户通过 `-i <name>` / `--instance <name>` 显式指定 → 覆盖以上两者

**全局回退安全规则（Global Fallback Safety）**：

当命令因当前目录无 `orbit.toml` 而回退到全局默认实例时，按命令类型区分行为：

| 命令类型 | 命令 | 回退行为 |
|---------|------|---------|
| **只读** | `search`, `list`, `check`, `outdated`, `info` | 静默回退，正常执行 |
| **修改状态** | `add`, `remove`, `purge`, `upgrade`, `sync` | **阻断**：输出黄色警告，要求用户显式传入 `-i <name>` 或 `cd` 到项目目录。若已传 `-i`，正常执行 |

```
⚠ orbit: not in an Orbit project directory, and '<name>' is the global default instance.
  Destructive operations require an explicit target. Use:
    orbit <command> -i <name>
  or cd into the project directory.
```

> **设计意图**：防止用户在桌面或其他无关目录执行 `orbit purge sodium --yes` 时意外破坏全局默认实例。

### 1.2 全局标志

| 标志 | 简写 | 类型 | 说明 |
|------|------|------|------|
| `--instance` | `-i` | `String` | 指定操作的实例名称（而非当前目录或默认实例） |
| `--verbose` | `-v` | `bool` | 输出详细日志（API 请求、文件操作、解析过程） |
| `--quiet` | `-q` | `bool` | 静默模式，仅输出错误信息 |
| `--yes` | `-y` | `bool` | 跳过所有交互式确认（危险操作需要） |
| `--dry-run` | — | `bool` | 仅模拟执行，不修改任何文件 |

### 1.3 输出约定

- **正常输出**到 stdout
- **错误/警告**到 stderr
- **交互式提示**到 stderr（这样 `orbit list --tree > mods.txt` 不会被污染）
- 破坏性操作（`remove`、`purge`、`upgrade`）默认要求用户确认，除非指定 `--yes`

### 1.4 退出码

| 退出码 | 含义 |
|--------|------|
| `0` | 成功 |
| `1` | 一般错误（文件不存在、网络错误、版本冲突等） |
| `2` | 参数错误（非法标志、缺少必填参数） |
| `3` | 用户取消（交互式确认选择了 No） |

---

## 2. 实例管理

### orbit init

```
orbit init <name> [--mc-version <ver>] [--modloader <loader>] [--modloader-version <ver>]
```

初始化当前目录为 Orbit 项目。

**行为**：

1. 检查当前目录下是否已存在 `orbit.toml` → 若存在，报错退出（错误码 2）
2. 探测当前目录环境：
   - 检查 `mods/` 是否存在 → 若存在，扫描所有 `.jar` 文件
   - 对每个 jar 计算 SHA-256，调用各平台适配器的 `get_version_by_hash()` 尝试识别
   - 识别成功的模组写入 `[dependencies]`（platform + slug + 检测到的版本），无法识别的以 `type = "file"` 形式记录
3. 检测 MC 版本和 Modloader（优先级：用户显式指定 → 从 fabric.mod.json / forge 相关文件探测 → 要求用户手动输入）
4. 生成 `orbit.toml`（使用探测到的信息 + 用户指定的参数）
5. 将实例注册到 `~/.orbit/instances.toml` 全局实例列表
6. 输出创建摘要：实例名、MC 版本、Loader、自动识别/未识别的模组数量

**错误条件**：

| 条件 | 错误信息 |
|------|---------|
| `orbit.toml` 已存在 | `Error: orbit.toml already exists in this directory. Use 'orbit sync' to reconcile.` |
| 无法探测 MC 版本且用户未指定 | `Error: Could not detect Minecraft version. Specify with --mc-version.` |
| 目录不是 `.minecraft` 结构（无 `mods/` 且无 `versions/`） | `Warning: This doesn't look like a .minecraft directory. Continue? [y/N]` |

**示例**：

```bash
orbit init my-survival
orbit init my-pack --mc-version 1.20.1 --modloader fabric --modloader-version 0.15.7
```

---

### orbit instances list

```
orbit instances list
```

列出所有被 Orbit 托管的实例。

**行为**：

1. 读取 `~/.orbit/instances.toml`
2. 逐行输出，格式：`[当前/默认标记] <name>  <path>  <mc_version>  <modloader>`
3. 标记规则：
   - 当前目录对应的实例 → `*` （当前）
   - 全局默认实例 → `(default)` 
   - 两者重合 → `* (default)`

**输出格式**：

```
  current  name             path                              mc        loader
*          my-survival      D:/Games/HMCL/.../.minecraft     1.20.1    fabric
  (default) creative-pack   D:/Games/HMCL/.../.minecraft     1.21      forge
```

**错误条件**：无。列表为空时输出 `No instances registered. Use 'orbit init' to get started.`

---

### orbit instances default

```
orbit instances default <name>
```

设置全局默认实例。

**行为**：

1. 在 `~/.orbit/instances.toml` 中查找 `<name>`
2. 若找到，将其标记为默认（仅允许一个默认实例）
3. 输出 `Default instance set to '<name>'`

**错误条件**：

| 条件 | 错误信息 |
|------|---------|
| 实例名不存在 | `Error: Instance '<name>' not found. Use 'orbit instances list' to see registered instances.` |

---

### orbit instances remove

```
orbit instances remove <name>
```

从 Orbit 全局追踪中移除实例。**绝不删除硬盘上的文件。**

**行为**：

1. 在 `~/.orbit/instances.toml` 中查找并移除 `<name>`
2. 如果该实例是默认实例，清除默认标记
3. 如果该实例的路径是当前工作目录，输出警告但不阻止操作
4. 输出确认：`Removed '<name>' from Orbit tracking. Files on disk were NOT deleted.`

**错误条件**：

| 条件 | 错误信息 |
|------|---------|
| 实例名不存在 | `Error: Instance '<name>' not found.` |

---

## 3. 模组增删

### orbit add

```
orbit add <mod> [--platform <p>] [--version <constraint>] [--env client|server|both] 
               [--optional] [--no-deps]
```

添加新模组。**修改 `orbit.toml` 和 `orbit.lock`**，并下载 jar 到 `mods/`。

**行为**：

1. 解析 `<mod>` 的前缀语法：
   - `mr:sodium` → platform=modrinth, slug=sodium
   - `cf:jei` → platform=curseforge, slug=jei
   - `file:./path.jar` → type=file, path=...
   - `sodium`（无前缀） → 按 `[resolver].platforms` 自动搜索
2. 查询平台 API，找到满足版本约束的最新兼容版本
3. 若该模组名已存在于 `orbit.toml` 的 `[dependencies]` → 报错（用 `orbit upgrade <mod>` 升级已有依赖）
4. 下载/定位 jar 文件，计算 SHA-256
5. 将依赖写入 `orbit.toml` 的 `[dependencies]` 表
6. 更新/生成 `orbit.lock` 条目
7. 除非指定 `--no-deps`，否则递归解析并安装传递依赖
8. 输出：`Added <name> <version> [platform] [env]`

**错误条件**：

| 条件 | 错误信息 |
|------|---------|
| 模组已存在于 orbit.toml | `Error: '<mod>' already exists. Use 'orbit upgrade <mod>' to update it.` |
| 模组在全部平台上都找不到 | `Error: Could not find '<mod>' on any platform.` |
| 版本约束无匹配 | `Error: No version of '<mod>' satisfies constraint '<c>'. Available: ...` |
| URL 下载失败 | `Error: Failed to download '<mod>' from <url>: <http_error>` |
| SHA-256 校验失败 | `Error: Checksum mismatch for '<mod>': expected <...>, got <...>` |
| 传递依赖冲突 | `Error: Dependency conflict: <mod-a> requires <dep> >= X, but <mod-b> requires <dep> < X.` |

**示例**：

```bash
orbit add sodium                                           # 自动匹配平台
orbit add cf:jei                                           # 显式 CurseForge
orbit add mr:sodium --version "^0.5"                       # 版本约束
orbit add zoomify --env client                             # 客户端专用
orbit add file:./my-mod.jar                                # 本地文件
orbit add sodium --no-deps                                 # 不装传递依赖
```

**设计语义**：`add` = 修改声明文件（toml）+ 下载 jar + 更新 lock。对标 `cargo add` / `yarn add`。

---

### orbit install

```
orbit install [--target client|server|both] [--group <group>] [--no-optional]
              [--locked] [--frozen]
```

根据 `orbit.toml` 和 `orbit.lock` 还原完整的模组环境。**不接受任何模组名称参数。**

**行为**：

1. 读取 `orbit.toml`，解析依赖树
2. 若 `orbit.lock` 存在，优先使用 lock 中锁定的版本（而非重新解析）
3. 按 `--target` 过滤：`client` → 安装 `env=client` + `env=both`；`server` → 安装 `env=server` + `env=both`；默认 `both` → 全部
4. 若指定 `--group`，取该分组与过滤结果的交集
5. 若指定 `--no-optional`，跳过 `optional = true` 的依赖
6. 对满足过滤条件的每个依赖：
   - 若 `orbit.lock` 有该条目且 `sha256` 匹配磁盘上的 jar → 跳过
   - 若不在 lock 中 → 解析版本、下载 jar、计算 SHA-256、写入 `orbit.lock`
   - 若在 lock 中但磁盘缺失 → 使用 lock 中的 URL 重新下载
7. 对每个在线依赖，解析其传递依赖并递归安装（除非被 `exclude` 排除）
8. 安装完成后输出摘要：`Installed X mods, skipped Y (already up to date), failed Z`

**`--locked` 标志**：

启用时，Orbit **仅使用 `orbit.lock`** 中的精确版本和 URL，不发起任何新的元数据解析。如果：

- `orbit.lock` 不存在 → 致命错误：`Error: --locked requires orbit.lock, but it doesn't exist. Run without --locked first.`
- `orbit.toml` 中的依赖在 lock 中没有对应条目 → 致命错误：`Error: --locked: orbit.toml has '<mod>' which is missing from orbit.lock. Run without --locked to resolve.`

**`--frozen` 标志**：

`--frozen` 是 `--locked` 的别名，行为完全一致。提供此别名是为了兼容 npm/pnpm 用户习惯。

**错误条件**：

| 条件 | 错误信息 |
|------|---------|
| `--locked` 但 `orbit.lock` 不存在 | `Error: --locked requires orbit.lock, but it doesn't exist.` |
| `--locked` 但 toml 与 lock 不一致 | `Error: --locked: orbit.toml has '<mod>' which is missing from orbit.lock.` |
| URL 下载失败 | `Error: Failed to download '<mod>' from <url>: <http_error>` |
| SHA-256 校验失败 | `Error: Checksum mismatch for '<mod>': expected <...>, got <...>` |
| 传递依赖冲突 | `Error: Dependency conflict: <mod-a> requires <dep> >= X, but <mod-b> requires <dep> < X.` |

**示例**：

```bash
orbit install                                              # 全量安装
orbit install --target server                               # 仅服务端依赖
orbit install --target client --no-optional                 # 轻量客户端
orbit install --locked --target server                      # 生产环境严格还原
```

**设计语义**：`install` = 状态还原，不修改声明文件。对标 `npm ci` / `yarn install --frozen-lockfile`。

---

### orbit remove

```
orbit remove <mod> [--yes]
```

卸载模组。

**行为**：

1. 在 `orbit.toml` 的 `[dependencies]` 中查找 `<mod>`
2. 若找到：
   - 删除 `mods/` 下对应的 jar 文件（从 `orbit.lock` 中查找文件名）
   - 从 `orbit.toml` 的 `[dependencies]` 中移除该条目
   - 从 `orbit.lock` 中移除该条目
   - 输出：`Removed <mod> <version>`
3. 若未找到：报错

**错误条件**：

| 条件 | 错误信息 |
|------|---------|
| 依赖不存于 orbit.toml | `Error: '<mod>' is not in orbit.toml.` |
| 用户未确认 | `Aborted.` (退出码 3) |

---

### orbit purge

```
orbit purge <mod> [--yes]
```

深度清理。在 `remove` 的基础上，启发式搜索 `config/` 目录并交互式删除关联配置文件。

**行为**：

1. 执行 `orbit remove <mod>` 的全部步骤
2. 启发式扫描 `config/` 目录：
   - 按模组名称匹配（大小写不敏感、连字符/下划线模糊匹配）
   - 按模组 slug 匹配
   - 列出所有候选配置文件及其路径
3. 交互式逐个询问用户是否删除每个候选文件
4. 若指定 `--yes`，直接删除所有候选文件
5. 输出清理摘要：`Purged <mod>: removed 1 jar and N config files.`

**错误条件**：同 `remove`。

**示例**：

```bash
orbit purge voxelmap
# Found 3 candidate config files:
#   config/voxelmap.properties         [y/N]? y
#   config/voxelmap/waypoints.db       [y/N]? y
#   config/voxelmap-settings.json      [y/N]? n
# Purged voxelmap: removed 1 jar and 2 config files.
```

---

## 4. 同步与更新

### orbit sync

```
orbit sync [--yes]
```

本地状态双向对齐。不产生网络下载。

**行为**：

1. 扫描 `mods/` 目录下所有 `.jar` 文件，计算 SHA-256
2. 读取 `orbit.toml` 和 `orbit.lock`
3. 三方比对，识别差异：

   | 状态 | 处理 |
   |------|------|
   | toml 有，mods/ 没有 | 标记为 MISSING（等待 `orbit install` 修复） |
   | toml 没有，mods/ 有 | 标记为 NEW：尝试 SHA-256 匹配已知模组；成功则添加到 toml + lock；失败则以 `type = "file"` 添加 |
   | toml 有，mods/ 有，SHA-256 匹配 lock | 无变化 |
   | toml 有，mods/ 有，SHA-256 不同于 lock | 标记为 CHANGED：更新 lock 中的 SHA-256 和版本号 |
   | toml 有，lock 没有 | 标记为 UNLOCKED：需要 `orbit install` 下载并生成 lock |

4. 将变更写入 `orbit.toml` 和 `orbit.lock`
5. 输出同步报告

**输出格式**：

```
Syncing...
  + added       journeymap (modrinth, 5.9.8)    ← 手动拖入，自动识别
  + added       unknown-mod (file)               ← 手动拖入，无法识别
  ~ changed     my-tweaks (file, SHA-256 updated) ← 本地文件已更新
  - missing     sodium (expected in mods/)        ← toml 声明了但文件丢了
  ? unlocked    lithium                           ← toml 有但 lock 无，需 install

Sync complete: 2 added, 1 changed, 1 missing, 1 unlocked.
Run 'orbit install' to restore missing mods.
```

**错误条件**：

| 条件 | 错误信息 |
|------|---------|
| `mods/` 目录不存在 | `Warning: mods/ directory not found. Nothing to sync.` |

---

### orbit outdated

```
orbit outdated [<mod>]
```

检查过时模组（只读）。联网比对已安装版本与平台最新版本，**不修改任何文件**。

**行为**：

1. 若指定 `<mod>`，仅检查该模组；否则遍历 `orbit.lock` 中的所有在线依赖
2. 对每个模组，查询其平台 API 获取最新兼容版本
3. 比对当前版本与最新版本
4. 输出过时报告

**输出格式**：

```
Checking for outdated mods...
  sodium          0.5.8   → 0.5.11  (modrinth)
  lithium         0.12.1  → 0.13.0  (modrinth)  ⚠ breaking change
  jei             12.0.0  ✓ up to date
  journeymap      5.9.18  → 5.9.20  (curseforge)

3 outdated mods. Run 'orbit upgrade' to apply.
⚠ lithium 0.13.0 is a major update. Review changelog before upgrading.
```

**错误条件**：

| 条件 | 错误信息 |
|------|---------|
| 模组不在 lock 中 | `Error: '<mod>' not found in orbit.lock.` |
| 网络不可达 | `Error: Could not reach <platform> API. Check your connection.` |
| 平台 API 返回错误 | `Error: <platform> API error: <message>` |

**设计语义**：对标 `npm outdated` / `cargo outdated`。只读检查，不改文件。

---

### orbit upgrade

```
orbit upgrade [<mod>] [--yes] [--dry-run]
```

执行更新。下载并替换可更新的 jar 文件。

**行为**：

1. 若指定 `<mod>`，仅升级该模组；否则升级所有可升级的依赖
2. 执行与 `orbit outdated` 相同的查询逻辑
3. 对每个有更新的模组：
   - 下载新版本 jar
   - 删除旧 jar
   - 更新 `orbit.lock` 条目（版本、URL、SHA-256、依赖树）
   - 若 `orbit.toml` 中指定的是精确版本 `=X.Y.Z`，跳过该模组并警告
4. 若 `orbit.toml` 中使用的是约束（非 `=`），保留约束表达式不变（如 `^0.5`），lock 指向新版本
5. 输出升级摘要

**错误条件**：同 `outdated`，外加 `remove` 的文件操作错误。

**与 `outdated` 的区别**：`outdated` 只看不改；`upgrade` 实际下载替换。

---

## 5. 查询与搜索

### orbit search

```
orbit search <query> [--platform <p>] [--limit <n>] [--mc-version <ver>] [--modloader <loader>]
```

搜索模组。

**行为**：

1. 若 `--platform` 指定，仅搜索该平台；否则搜索 `[resolver].platforms` 中全部平台，合并结果
2. 为每个搜索结果标注：模组名、平台、简介（截断到一行）、最新版本、下载量
3. 兼容当前 MC 版本的结果高亮显示（绿色 `✓`）
4. 按相关度排序，`--limit` 默认 20

**输出格式**：

```
Searching for "sodium" on modrinth, curseforge...

  ✓ sodium (modrinth)          ⬇ 12.3M   v0.5.8    mc1.20.1
    Sodium is a free and open-source rendering engine...
  ✓ sodium-extra (modrinth)    ⬇ 2.1M    v0.4.0    mc1.20.1
    Extra features for Sodium.
    sodium (curseforge)        ⬇ 8.7M    v0.5.8    mc1.20.1
    ...
```

**错误条件**：

| 条件 | 错误信息 |
|------|---------|
| 所有平台均无结果 | `No results found for '<query>'.` |

---

### orbit info

```
orbit info <mod> [--platform <p>] [--mc-version <ver>] [--modloader <loader>]
```

查看模组详细信息（无需安装）。直接请求平台 API，打印该模组的完整元数据。

**行为**：

1. 若 `--platform` 指定，仅查询该平台；否则按 `[resolver].platforms` 顺序搜索
2. 请求平台 API 获取模组详情
3. 输出详细信息

**输出格式**：

```
sodium (modrinth)
  id: AANobbMI
  slug: sodium
  description: Sodium is a free and open-source rendering engine designed
               to improve frame rates and reduce micro-stutter in Minecraft.
  authors: jellysquid3, IMS
  latest version: 0.5.11 (mc 1.20.1, fabric)
  client side: required   server side: unsupported
  license: LGPL-3.0
  downloads: 12,340,000
  categories: graphics, optimization

  Recent versions:
    0.5.11   mc 1.20.1, 1.20.4   fabric   released 2026-03-15
    0.5.8    mc 1.20.1           fabric   released 2025-12-01
    0.5.3    mc 1.20.1           fabric   released 2025-09-10

  Dependencies:
    (none)
```

**错误条件**：

| 条件 | 错误信息 |
|------|---------|
| 模组在全部平台上都找不到 | `Error: Could not find '<mod>' on any platform.` |
| 网络不可达 | `Error: Could not reach <platform> API.` |

**设计语义**：对标 `npm view` / `cargo search <pkg> --limit 1`。用于在 `add` 之前了解模组详情。

---

### orbit list

```
orbit list [--tree] [--target client|server|both]
```

列出当前实例已安装的模组。

**行为**：

1. 读取 `orbit.lock`（若不存在则报错）
2. 默认输出为扁平表格：名称、版本、平台、env
3. `--tree` 模式：以树状结构展示，每个模组下方缩进显示其传递依赖

**扁平输出**：

```
  name            version    platform      env
  sodium          0.5.8      modrinth      both
  lithium         0.12.1     modrinth      server
  journeymap      5.9.18     curseforge    client
  jei             12.0.0     curseforge    both
  zoomify         2.11.1     modrinth      client (optional)
  my-tweaks       1.0        file          both

6 mods installed (2 client-only, 1 server-only, 1 optional)
```

**树状输出** (`--tree`)：

```
  sodium 0.5.8 (modrinth, both)
  lithium 0.12.1 (modrinth, server)
  journeymap 5.9.18 (curseforge, client)
  ├── journeymap-api 1.0.2 (modrinth, both)
  └── journeymap-icons 1.0.0 (modrinth, both)
  jei 12.0.0 (curseforge, both)
  zoomify 2.11.1 (modrinth, client, optional)
  my-tweaks 1.0 (file, both)
```

**错误条件**：

| 条件 | 错误信息 |
|------|---------|
| `orbit.lock` 不存在 | `Error: No orbit.lock found. Run 'orbit install' first.` |

---

## 6. 导入、导出与工具

### orbit import

```
orbit import <file> [--merge-strategy prefer-existing|prefer-import|interactive]
```

合并外部模组清单。

**支持格式**：

| 扩展名 | 处理方式 |
|--------|---------|
| `.toml` | 解析为 orbit.toml，合并 `[dependencies]` |
| `.zip` / `.mrpack` | 提取 `mods/` 目录中的 jar，隐式触发 `orbit sync` |

**TOML 合并行为**：

1. 解析导入文件
2. 逐条比对 `<file>` 的 `[dependencies]` 与当前 `orbit.toml`：
   - 若键名仅存在于导入文件中 → 添加
   - 若键名同时存在，版本约束不同 → 按 `--merge-strategy` 决定
3. 写入合并后的 `orbit.toml`
4. 输出合并摘要

**ZIP 导入行为**：

1. 解压到临时目录
2. 提取所有 `.jar` 文件到 `mods/`
3. 触发 `orbit sync` 识别新 jar
4. 清理临时目录

**错误条件**：

| 条件 | 错误信息 |
|------|---------|
| 文件不存在 | `Error: '<file>' not found.` |
| 格式不支持 | `Error: Unsupported file format. Expected .toml, .zip, or .mrpack.` |
| TOML 解析失败 | `Error: Failed to parse '<file>': <parse_error>` |

---

### orbit export

```
orbit export [<output>] [--target client|server|both] [--format zip|mrpack]
```

打包导出整合包。

**行为**：

1. 若未指定 `<output>`，默认文件名为 `<project.name>-<project.version>.zip`
2. 按 `--target` 过滤依赖（默认 `both` = 全量）
3. 将过滤后的 jar 文件 + `orbit.toml` + `orbit.lock` 打包为 zip
4. 若 `--format mrpack`，输出为 Modrinth 整合包格式（含 `modrinth.index.json`）

**错误条件**：

| 条件 | 错误信息 |
|------|---------|
| 无写入权限 | `Error: Permission denied: cannot write to '<output>'` |

---

### orbit check

```
orbit check <mc_version> [--modloader <loader>]
```

跨版本升级预检。检查当前模组集合是否已有目标 MC 版本的兼容版本。

**行为**：

1. 遍历 `orbit.lock` 中所有在线平台依赖
2. 对每个模组，查询其平台 API：是否存在兼容 `<mc_version>` + 当前 `modloader` 的版本
3. 输出兼容性矩阵

**输出格式**：

```
Checking compatibility with Minecraft 1.21 (fabric)...

  sodium          0.5.8     ✓ 0.6.0 available on modrinth
  lithium         0.12.1    ✓ 0.14.0 available on modrinth
  journeymap      5.9.18    ✗ no compatible version yet
  jei             12.0.0    ✓ 14.0.0 available on curseforge

3 of 4 mods are ready for Minecraft 1.21.
journeymap is blocking the upgrade.
```

**错误条件**：

| 条件 | 错误信息 |
|------|---------|
| `orbit.lock` 不存在 | `Error: No orbit.lock found. Run 'orbit install' first.` |

---

### orbit cache clean

```
orbit cache clean [--yes]
```

清理全局下载缓存。

**行为**：

1. 列出 `~/.orbit/cache/` 的内容及大小
2. 交互式确认（除非 `--yes`）
3. 删除缓存目录下的所有文件
4. 输出：`Cleaned cache: freed <size>.`

**错误条件**：无。缓存为空时输出 `Cache is already empty.`

---

## 7. 核心动词速查

| 意图 | 命令 | 对标 | 修改文件？ |
|:---|:---|:---|:---:|
| 找模组 | `orbit search <query>` | `npm search` | 否 |
| 看详情 | `orbit info <mod>` | `npm view` / `cargo search --limit 1` | 否 |
| 加模组 | `orbit add <mod>` | `yarn add` / `cargo add` | **是** |
| 删模组 | `orbit remove <mod>` | `yarn remove` | **是** |
| 深度清理 | `orbit purge <mod>` | — | **是** |
| 看列表 | `orbit list` | `npm list` | 否 |
| 查过时 | `orbit outdated` | `npm outdated` | 否 |
| 做更新 | `orbit upgrade [<mod>]` | `yarn upgrade` | **是** |
| 按清单还原 | `orbit install [--locked]` | `npm ci` / `yarn install --frozen-lockfile` | 否* |

> \* `install` 仅写入 `orbit.lock`（若无），不修改 `orbit.toml`。

---

> 本文档与 `orbit-toml-spec.md` 共同构成 Orbit CLI 的完整开发规范。两文档冲突时，以本文档为准（命令行为 > 数据格式）。
