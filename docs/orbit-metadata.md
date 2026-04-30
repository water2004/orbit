# Orbit 模组元数据解析层设计

> 本文档定义 `orbit-core/src/metadata/` 的 trait 体系、各加载器格式映射、
> 以及 `MetadataExtractor` 策略选择器的完整规格。

---

## 目录

1. [设计动机](#1-设计动机)
2. [目录结构](#2-目录结构)
3. [核心抽象](#3-核心抽象)
   - [ModMetadata — 统一元数据](#modmetadata--统一元数据)
   - [MetadataParser trait](#metadataparser-trait)
   - [MetadataExtractor — 策略选择器](#metadataextractor--策略选择器)
4. [各加载器格式映射](#4-各加载器格式映射)
   - [Fabric — fabric.mod.json](#fabric--fabricmodjson)
   - [Forge — META-INF/mods.toml](#forge--meta-infmodstoml)
   - [NeoForge — META-INF/neoforge.mods.toml](#neoforge--meta-infneoforgemodstoml)
   - [Quilt — quilt.mod.json](#quilt--quiltmodjson)
5. [解析流程图](#5-解析流程图)
6. [Rust 实现参考](#6-rust-实现参考)
7. [扩展指南](#7-扩展指南)

---

## 1. 设计动机

Minecraft 有多个模组加载器（Fabric、Forge、NeoForge、Quilt），每个加载器的 JAR 内嵌元数据格式不同：

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

**核心设计原则**：加载器之间互不知晓。新增一个加载器只需加一个文件 + 一行注册代码。

---

## 2. 目录结构

```
orbit-core/src/metadata/
├── mod.rs               # MetadataParser trait + ModMetadata + MetadataExtractor
├── fabric.rs            # FabricParser — fabric.mod.json (JSON)
├── forge.rs             # ForgeParser — META-INF/mods.toml (TOML)
├── neoforge.rs          # NeoForgeParser — META-INF/neoforge.mods.toml (TOML)
└── quilt.rs             # QuiltParser — quilt.mod.json (JSON)
```

与 `providers/` 完全对称的策略模式结构。`jar.rs` 作为入口，调用 `MetadataExtractor`。

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
| `dependencies` | `IndexMap<String, String>` | 加载器相关的依赖映射，key=mod_id, value=version_constraint |
| `loader` | `ModLoader` | 枚举: `Fabric`, `Forge`, `NeoForge`, `Quilt` |
| `mc_versions` | `Vec<String>` | 兼容的 MC 版本范围或列表 |
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

## 5. 解析流程

```
jar.rs: get_mod_metadata(file, modloader_context)
  │
  ├─ 1. 计算 SHA-256（流式，避免将整个 JAR 加载到内存）
  │
  ├─ 2. 打开 ZIP 归档，对每个 parser 执行 O(1) 直接查找：
  │      candidates = []
  │      for parser in extractor.parsers:
  │          if archive.by_name(parser.target_file()).is_ok():
  │              candidates.push(parser)
  │      -- 若 candidates 为空 → 返回 Err(UnrecognizedJar)
  │
  ├─ 3. 多加载器消除歧义：
  │      if candidates.len() == 1:
  │          return candidates[0].parse(content)
  │      -- 存在多个元数据文件（Architectury 多端 JAR）
  │      -- 按 modloader_context 筛选匹配的 parser
  │      if 有匹配 → 使用匹配的
  │      else → 返回 AmbiguousJar 错误（要求用户手动指定）
  │
  └─ 4. 填充 sha256 → 返回 ModMetadata
```

> **性能说明**：`archive.by_name()` 使用 ZIP 中央目录索引，O(1) 直接查找，不遍历文件列表。扫描 200 个模组时，这比逐条目遍历快一个数量级。物理上 ZIP 中央目录在文件末尾，`by_name` 直接跳转读取。

---

## 6. Rust 实现参考

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
    pub dependencies: IndexMap<String, String>,
    pub loader: ModLoader,
    pub mc_versions: Vec<String>,
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

    /// 从 ZIP 归档中提取元数据。
    ///
    /// `modloader_context` 用于多加载器 JAR 的歧义消除：
    /// 当同一个 jar 同时包含 fabric.mod.json 和 mods.toml 时，
    /// 优先返回与当前实例 loader 匹配的解析结果。
    pub fn extract(
        &self,
        archive: &mut zip::ZipArchive<std::fs::File>,
        modloader_context: Option<&str>,
    ) -> Result<ModMetadata, OrbitError> {
        // 1. 收集所有能匹配的 parser（O(1) by_name 查找）
        let mut candidates: Vec<(&dyn MetadataParser, String)> = vec![];
        for parser in &self.parsers {
            if let Ok(mut file) = archive.by_name(parser.target_file()) {
                let mut content = String::new();
                std::io::Read::read_to_string(&mut file, &mut content)
                    .map_err(|e| OrbitError::Other(anyhow::anyhow!("failed to read {}: {e}", parser.target_file())))?;
                candidates.push((parser.as_ref(), content));
            }
        }

        // 2. 消除歧义
        match candidates.len() {
            0 => Err(OrbitError::Other(anyhow::anyhow!(
                "unrecognized JAR: no metadata file found for any known loader"
            ))),
            1 => {
                let (parser, content) = candidates.into_iter().next().unwrap();
                parser.parse(&content)
            }
            _ => {
                // 多加载器 JAR — 按 modloader_context 选择
                if let Some(ctx) = modloader_context {
                    let ctx_lower = ctx.to_lowercase();
                    for (parser, content) in &candidates {
                        let loader_name = match parser.loader_type() {
                            ModLoader::Fabric => "fabric",
                            ModLoader::Forge => "forge",
                            ModLoader::NeoForge => "neoforge",
                            ModLoader::Quilt => "quilt",
                            ModLoader::Unknown => "",
                        };
                        if loader_name == ctx_lower {
                            return parser.parse(content);
                        }
                    }
                }
                Err(OrbitError::Other(anyhow::anyhow!(
                    "ambiguous JAR: contains multiple metadata files ({:?}). Specify --modloader to disambiguate.",
                    candidates.iter().map(|(p, _)| p.target_file()).collect::<Vec<_>>()
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

## 7. 扩展指南

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
