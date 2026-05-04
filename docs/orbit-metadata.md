# Orbit 文件元数据解析层设计

> 本文档定义 `orbit-core/src/metadata/` 的所有文件格式解析。
> 覆盖**模组**元数据（策略模式）和 **Mojang 游戏本体**版本信息（纯函数）。

---

## 目录

1. [设计概述](#1-设计概述)
2. [目录结构](#2-目录结构)
3. [模组元数据（策略模式）](#3-模组元数据策略模式)
   - [ModMetadata — 统一元数据](#modmetadata--统一元数据)
   - [MetadataParser trait](#metadataparser-trait)
   - [MetadataExtractor — 策略选择器](#metadataextractor--策略选择器)
4. [各加载器格式映射](#4-各加载器格式映射)
   - [Fabric — fabric.mod.json](#fabric--fabricmodjson)
   - [Forge — META-INF/mods.toml](#forge--meta-infmodstoml)
   - [NeoForge — META-INF/neoforge.mods.toml](#neoforge--meta-infneoforgemodstoml)
   - [Quilt — quilt.mod.json](#quilt--quiltmodjson)
5. [游戏本体版本（纯函数）](#5-游戏本体版本纯函数)
6. [模组元数据解析流程图](#6-模组元数据解析流程图)
7. [Rust 实现参考](#7-rust-实现参考)
8. [扩展指南](#8-扩展指南)

---

## 1. 设计概述

`metadata/` 模块处理两类文件格式：

| 类别 | 文件 | 方案 | 原因 |
|------|------|------|------|
| 模组元数据 | fabric.mod.json / mods.toml / ... | **策略模式** | 加载器多，格式各异，需可扩展 |
| 游戏本体 | version.json | **纯函数** | 只有一个格式，固定路径，无扩展需求 |

**模组元数据（策略模式）**：

| 加载器 | 元数据文件 | 格式 |
|--------|-----------|------|
| Fabric | `fabric.mod.json` | JSON |
| Forge | `META-INF/mods.toml` | TOML |
| NeoForge | `META-INF/neoforge.mods.toml` | TOML |
| Quilt | `quilt.mod.json` | JSON |

Orbit 需要从任意 JAR 中提取**统一的** `ModMetadata`，用于：
- `orbit init` 自动识别手动拖入的模组
- `orbit sync` 通过哈希反查失败时，回退读取元数据作为兜底
- 验证模组兼容性（MC 版本、加载器类型是否匹配当前实例）

**游戏本体版本（纯函数）**：

`instance/version.json` 是 Mojang 标准格式，位置固定、格式唯一。不需要策略模式，一个函数直接解析即可。用于 `orbit init` 自动检测当前实例的 MC 版本。

**核心设计原则**：加载器之间互不知晓。新增一个加载器只需加一个文件 + 一行注册代码。

---

## 2. 目录结构

```
orbit-core/src/metadata/
├── mod.rs               # MetadataParser trait + ModMetadata + MetadataExtractor
├── fabric.rs            # FabricParser — fabric.mod.json (JSON)
├── forge.rs             # ForgeParser — META-INF/mods.toml (TOML) [future]
├── neoforge.rs          # NeoForgeParser — META-INF/neoforge.mods.toml (TOML) [future]
├── quilt.rs             # QuiltParser — quilt.mod.json (JSON) [future]
└── mojang.rs            # McVersion — version.json (JSON, 纯函数，不走 trait)
```

`jar.rs` 调用 `MetadataExtractor`（模组）；`detection/` 模块调用 `mojang.rs`（游戏版本）。

---

## 3. 核心抽象

### ModMetadata — 统一元数据

所有加载器解析后都归一化为这个结构：

| 字段 | 类型 | 来源 |
|------|------|------|
| `id` | `String` | Fabric: `id`, Forge: `mods[].modId`, Quilt: `quilt_loader.id` |
| `name` | `String` | Fabric: `name`, Forge: `mods[].displayName` |
| `version` | `String` | Fabric: `version`, Forge: `mods[].version` |
| `authors` | `Vec<String>` | Fabric: `authors`, Forge: `mods[].authors` |
| `description` | `String` | Fabric: `description`, Forge: `mods[].description` |
| `license` | `Option<String>` | 许可证 ID |
| `environment` | `String` | 运行环境: "client" / "server" / "both" |
| `dependencies` | `IndexMap<String, String>` | 加载器相关的依赖映射，key=mod_id, value=version_constraint |
| `embedded_jars` | `Vec<String>` | META-INF/jars/ 下的内嵌 JAR 路径 |
| `loader` | `ModLoader` | 枚举: `Fabric`, `Forge`, `NeoForge`, `Quilt` |
| `sha256` | `String` | 由 `jar.rs` 在解析前计算填充 |

### MetadataParser trait

```rust
/// 每个加载器实现此 trait
trait MetadataParser: Send + Sync {
    /// JAR 内的目标文件名（用于 ZIP 条目匹配）
    fn target_file(&self) -> &str;

    /// 此解析器对应的加载器类型（解析成功时填入 ModMetadata.loader）
    fn loader_type(&self) -> ModLoader;

    /// 将文件内容解析为统一元数据
    fn parse(&self, content: &str) -> Result<ModMetadata, OrbitError>;
}
```

### MetadataExtractor — 策略选择器

```rust
struct MetadataExtractor {
    parsers: Vec<Box<dyn MetadataParser>>,
}
```

**行为**：遍历 JAR 的所有文件名，对每个文件遍历所有 parser——若文件名匹配 `target_file()`，调用该 parser 的 `parse()`。

**Builder 模式**：

```rust
let extractor = MetadataExtractor::builder()
    .with(FabricParser)
    .with(ForgeParser)
    .with(NeoForgeParser)
    .with(QuiltParser)
    .build();
```

---

## 4. 各加载器格式映射

### Fabric — fabric.mod.json

**文件名**：`fabric.mod.json`

**格式**：JSON

**字段映射**：

| JSON 字段 | ModMetadata 字段 | 备注 |
|-----------|-----------------|------|
| `id` | `id` | 如 `sodium` |
| `name` | `name` | 人类可读名称 |
| `version` | `version` | 如 `0.5.8` |
| `authors` | `authors` | 字符串数组 |
| `description` | `description` | |
| `depends` | `dependencies` | `{ "fabric-api": ">=0.92.0" }` |
| — | `loader` | `ModLoader::Fabric` |
| `depends.minecraft` | `mc_versions` | 从 `depends` 中提取 `minecraft` 键 |

**示例**（sodium 的 fabric.mod.json 简化）：
```json
{
  "schemaVersion": 1,
  "id": "sodium",
  "version": "0.5.8",
  "name": "Sodium",
  "description": "Modern rendering engine...",
  "authors": ["jellysquid3", "IMS212"],
  "depends": {
    "fabricloader": ">=0.15.0",
    "minecraft": "1.20.1",
    "fabric-api": "*"
  }
}
```

### Forge — META-INF/mods.toml

**文件名**：`META-INF/mods.toml`

**格式**：TOML

**结构**：
```toml
modLoader = "javafml"
loaderVersion = "[47,)"
license = "LGPL-2.1"

[[mods]]
modId = "jei"
version = "12.0.0"
displayName = "Just Enough Items"
description = "View Items and Recipes"
authors = ["mezz"]

[[dependencies.jei]]
modId = "forge"
mandatory = true
versionRange = "[47,)"
ordering = "NONE"
side = "BOTH"
```

**字段映射**：

| TOML 路径 | ModMetadata 字段 |
|-----------|-----------------|
| `mods[0].modId` | `id` |
| `mods[0].displayName` | `name` |
| `mods[0].version` | `version` |
| `mods[0].authors` | `authors` |
| `mods[0].description` | `description` |
| `dependencies.<id>.modId` + `dependencies.<id>.versionRange` | `dependencies` |
| `dependencies.minecraft.versionRange` | `mc_versions` | 从 `modId == "minecraft"` 的条目中提取 |
| — | `loader` = `ModLoader::Forge` |

> **注意**：Forge JAR 可能包含多个 `[[mods]]` 条目（极少见）。Orbit 取第一个作为主模组。

### NeoForge — META-INF/neoforge.mods.toml

**文件名**：`META-INF/neoforge.mods.toml`

**格式**：TOML（结构与 Forge 完全相同）

与 Forge 的唯一区别：文件名不同、`loader` 填入 `ModLoader::NeoForge`。字段映射复用 Forge 逻辑。

### Quilt — quilt.mod.json

**文件名**：`quilt.mod.json`

**格式**：JSON

**结构**：
```json
{
  "schema_version": 1,
  "quilt_loader": {
    "group": "com.example",
    "id": "example-mod",
    "version": "1.0.0",
    "metadata": {
      "name": "Example Mod",
      "description": "...",
      "contributors": { "Author1": "Developer" }
    },
    "depends": [
      { "id": "minecraft", "versions": ">=1.20" },
      { "id": "fabric-api" }
    ]
  }
}
```

**字段映射**：

| JSON 路径 | ModMetadata 字段 |
|-----------|-----------------|
| `quilt_loader.id` | `id` |
| `quilt_loader.metadata.name` | `name` |
| `quilt_loader.version` | `version` |
| `quilt_loader.depends[].id` + `versions` | `dependencies` |
| — | `loader` = `ModLoader::Quilt` |

---

## 5. 游戏本体版本（纯函数）

Mojang 的 `version.json` 存放在游戏 JAR 内部，格式唯一、无扩展需求——一个纯解析函数即可。

**入口**：`jar.rs` 从 JAR 中读出 `version.json` 的字符串内容 → 交给此模块解析。

**不碰文件**：此模块的输入是 JSON 字符串内容，不关心它来自哪个 JAR、什么路径。

### 示例

```json
{
    "id": "1.21.11",
    "name": "1.21.11",
    "world_version": 4671,
    "series_id": "main",
    "protocol_version": 774,
    "pack_version": {
        "resource_major": 75,
        "resource_minor": 0,
        "data_major": 94,
        "data_minor": 1
    },
    "build_time": "2025-12-09T12:20:42+00:00",
    "java_component": "java-runtime-delta",
    "java_version": 21,
    "stable": true,
    "use_editor": false
}
```

### 提取类型

```rust
/// Minecraft 游戏本体版本信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McVersion {
    /// 版本 ID，如 "1.21.11"
    pub id: String,
    /// 人类可读名称
    pub name: String,
    /// 世界数据版本（用于存档兼容性判断）
    pub world_version: u32,
    /// 网络协议版本
    pub protocol_version: u32,
    /// 资源包/数据包版本
    pub pack_version: PackVersion,
    /// 要求的 Java 版本
    pub java_version: u32,
    /// 是否为稳定版
    pub stable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackVersion {
    pub resource_major: u32,
    pub resource_minor: u32,
    pub data_major: u32,
    pub data_minor: u32,
}
```

### 解析函数

```rust
impl McVersion {
    /// 从 version.json 字符串内容解析
    pub fn from_json(content: &str) -> Result<Self, OrbitError> {
        serde_json::from_str(content)
            .map_err(|e| OrbitError::Other(anyhow::anyhow!("invalid version.json: {e}")))
    }
}
```

> **设计决策**：为什么不用 trait？Mojang 只有一个，不像加载器需要策略模式。`McVersion::from_json(str)` 只做一件事——把 JSON 字符串变成 struct。调用方（`detection/`）负责从 JAR 中提取这个字符串。

---

## 6. 模组元数据解析流程

**职责边界**：`jar.rs` 负责文件 I/O，`metadata/` 只做字符串解析。

```
jar.rs: get_mod_metadata(file, modloader_context)
  │
  ├─ 1. 计算 SHA-256（流式，避免将整个 JAR 加载到内存）
  │
  ├─ 2. 打开 ZIP 归档，用每个 parser 的 target_file() 做 O(1) 直接查找：
  │      entries = []
  │      for parser in extractor.parsers:
  │          if let Ok(file) = archive.by_name(parser.target_file()):
  │              entries.push((filename, read_to_string(file)))
  │      -- 若 entries 为空 → 返回 Err(UnrecognizedJar)
  │
  ├─ 3. 将 entries 传给 MetadataExtractor::extract(entries, modloader_context)
  │      -- extract() 只做文件名匹配 + 调用 parser.parse(str)
  │      -- 不碰任何文件 I/O
  │
  ├─ 4. 填充 sha256 → 返回 ModMetadata
  │
  └─ 5. 歧义消除（多个候选时按 modloader_context 选择）
```

> **分层职责**：`jar.rs` 负责"从哪里读"（ZIP 打开、`by_name` 查找、read_to_string），`metadata/` 负责"怎么解析"（字符串 → 结构体）。parser 根本不知道文件系统的存在。

---

## 7. Rust 实现参考

### mod.rs — trait + extractor

```rust
// orbit-core/src/metadata/mod.rs

use indexmap::IndexMap;
use crate::error::OrbitError;

// ── 统一类型 ──────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum ModLoader {
    Fabric,
    Forge,
    NeoForge,
    Quilt,
    Unknown,
}

impl ModLoader {
    pub fn as_str(&self) -> &'static str {
        match self {
            ModLoader::Fabric => "fabric",
            ModLoader::Forge => "forge",
            ModLoader::NeoForge => "neoforge",
            ModLoader::Quilt => "quilt",
            ModLoader::Unknown => "",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ModMetadata {
    pub id: String,
    pub name: String,
    pub version: String,
    pub authors: Vec<String>,
    pub description: String,
    pub license: Option<String>,
    pub environment: String,
    pub dependencies: IndexMap<String, String>,
    pub embedded_jars: Vec<String>,
    pub loader: ModLoader,
    pub sha256: String,
}

// ── Parser trait ───────────────────────────

pub trait MetadataParser: Send + Sync {
    fn target_file(&self) -> &str;
    fn loader_type(&self) -> ModLoader;
    fn parse(&self, content: &str) -> Result<ModMetadata, OrbitError>;
}

// ── Extractor ──────────────────────────────

pub struct MetadataExtractor {
    parsers: Vec<Box<dyn MetadataParser>>,
}

pub struct MetadataExtractorBuilder {
    parsers: Vec<Box<dyn MetadataParser>>,
}

impl MetadataExtractorBuilder {
    pub fn new() -> Self {
        Self { parsers: vec![] }
    }

    pub fn with(mut self, parser: impl MetadataParser + 'static) -> Self {
        self.parsers.push(Box::new(parser));
        self
    }

    pub fn build(self) -> MetadataExtractor {
        MetadataExtractor { parsers: self.parsers }
    }
}

impl MetadataExtractor {
    pub fn builder() -> MetadataExtractorBuilder {
        MetadataExtractorBuilder::new()
    }

    /// 从已提取的条目中解析模组元数据。
    /// `entries` 由 `jar.rs` 通过 `archive.by_name()` 提取后传入。
    /// 此方法纯内存操作，不碰文件 I/O。
    pub fn extract(
        &self,
        entries: &[(String, String)],
        modloader_context: Option<&str>,
    ) -> Result<ModMetadata, OrbitError> {
        let mut candidates: Vec<(&dyn MetadataParser, &str)> = vec![];
        for (filename, content) in entries {
            for parser in &self.parsers {
                if filename == parser.target_file() {
                    candidates.push((parser.as_ref(), content.as_str()));
                }
            }
        }

        match candidates.len() {
            0 => Err(OrbitError::Other(anyhow::anyhow!(
                "unrecognized JAR: no known metadata file"
            ))),
            1 => candidates[0].0.parse(candidates[0].1),
            _ => {
                if let Some(ctx) = modloader_context {
                    for (parser, content) in &candidates {
                        if parser.loader_type().as_str() == ctx.to_lowercase() {
                            return parser.parse(content);
                        }
                    }
                }
                Err(OrbitError::Other(anyhow::anyhow!(
                    "ambiguous JAR: contains multiple metadata files"
                )))
            }
        }
    }
}

// ── 辅助：默认 extractor 注册所有已知 parser ──

pub fn default_extractor() -> MetadataExtractor {
    MetadataExtractor::builder()
        .with(super::fabric::FabricParser)
        .with(super::forge::ForgeParser)
        .with(super::neoforge::NeoForgeParser)
        .with(super::quilt::QuiltParser)
        .build()
}
```

### fabric.rs 示例

```rust
// orbit-core/src/metadata/fabric.rs

use super::{ModLoader, ModMetadata, MetadataParser};
use crate::error::OrbitError;
use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Deserialize)]
struct FabricModJson {
    id: Option<String>,
    name: Option<String>,
    version: Option<String>,
    #[serde(default)]
    authors: Vec<String>,
    description: Option<String>,
    #[serde(default)]
    depends: IndexMap<String, String>,
}

pub struct FabricParser;

impl MetadataParser for FabricParser {
    fn target_file(&self) -> &str {
        "fabric.mod.json"
    }

    fn loader_type(&self) -> ModLoader {
        ModLoader::Fabric
    }

    fn parse(&self, content: &str) -> Result<ModMetadata, OrbitError> {
        let raw: FabricModJson = serde_json::from_str(content)
            .map_err(|e| OrbitError::Other(anyhow::anyhow!("invalid fabric.mod.json: {e}")))?;

        // 提取 minecraft 版本约束
        let mc_versions = raw.depends
            .get("minecraft")
            .map(|v| vec![v.clone()])
            .unwrap_or_default();

        Ok(ModMetadata {
            id: raw.id.unwrap_or_default(),
            name: raw.name.unwrap_or_default(),
            version: raw.version.unwrap_or_default(),
            authors: raw.authors,
            description: raw.description.unwrap_or_default(),
            dependencies: raw.depends,
            loader: ModLoader::Fabric,
            mc_versions,
            sha256: String::new(),
        })
    }
}
```

### forge.rs 示例

```rust
// orbit-core/src/metadata/forge.rs

use super::{ModLoader, ModMetadata, MetadataParser};
use crate::error::OrbitError;
use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Deserialize)]
struct ForgeModsToml {
    mods: Vec<ForgeModInfo>,
    #[serde(default)]
    dependencies: IndexMap<String, Vec<ForgeDependency>>,
}

#[derive(Deserialize)]
struct ForgeModInfo {
    #[serde(rename = "modId")]
    mod_id: String,
    version: Option<String>,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    description: Option<String>,
    #[serde(default)]
    authors: Vec<String>,
}

#[derive(Deserialize)]
struct ForgeDependency {
    #[serde(rename = "modId")]
    mod_id: String,
    #[serde(rename = "versionRange")]
    version_range: Option<String>,
}

pub struct ForgeParser;

impl MetadataParser for ForgeParser {
    fn target_file(&self) -> &str {
        "META-INF/mods.toml"
    }

    fn loader_type(&self) -> ModLoader {
        ModLoader::Forge
    }

    fn parse(&self, content: &str) -> Result<ModMetadata, OrbitError> {
        let raw: ForgeModsToml = toml::from_str(content)
            .map_err(|e| OrbitError::Other(anyhow::anyhow!("invalid mods.toml: {e}")))?;

        let primary = raw.mods.first().ok_or_else(|| {
            OrbitError::Other(anyhow::anyhow!("mods.toml has no [[mods]] entries"))
        })?;

        let deps: IndexMap<String, String> = raw.dependencies
            .iter()
            .flat_map(|(k, v)| v.iter().map(move |d| {
                (d.mod_id.clone(), d.version_range.clone().unwrap_or("*".into()))
            }))
            .collect();

        // Forge 中 Minecraft 版本作为普通依赖存在，modId = "minecraft"
        let mc_versions = deps.get("minecraft")
            .map(|v| vec![v.clone()])
            .unwrap_or_default();

        Ok(ModMetadata {
            id: primary.mod_id.clone(),
            name: primary.display_name.clone().unwrap_or_default(),
            version: primary.version.clone().unwrap_or_default(),
            authors: primary.authors.clone(),
            description: primary.description.clone().unwrap_or_default(),
            dependencies: deps,
            loader: ModLoader::Forge,
            mc_versions,
            sha256: String::new(),
        })
    }
}
```

---

## 8. 扩展指南

新增一个加载器只需三步，**不修改任何现有文件**：

**步骤 1**：创建 `orbit-core/src/metadata/newloader.rs`

```rust
pub struct NewLoaderParser;
impl MetadataParser for NewLoaderParser {
    fn target_file(&self) -> &str { "new-loader.mod.json" }
    fn loader_type(&self) -> ModLoader { ModLoader::Unknown }
    fn parse(&self, content: &str) -> Result<ModMetadata, OrbitError> { /* mapping */ }
}
```

**步骤 2**：在 `default_extractor()` 的 builder 链中加一行：

```rust
.with(super::newloader::NewLoaderParser)
```

**步骤 3**：如果加载器不属于已有枚举成员，在 `ModLoader` 枚举中新增变体。

> 无需修改 `jar.rs`、`manifest.rs`、`resolver.rs` 或任何其他模块。

---

> **关联文档**
> - [orbit-architecture.md](orbit-architecture.md) — metadata/ 模块的位置
> - [orbit-toml-spec.md](orbit-toml-spec.md) — orbit.toml 项目级配置
> - [orbit-global-config.md](orbit-global-config.md) — config.toml 全局配置
