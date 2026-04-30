# Orbit 实例环境检测层设计

> 本文档定义 `orbit-core/src/detection/` 的策略模式架构、
> MC 版本检测流程、以及 `orbit init` 的完整编排逻辑。

---

## 目录

1. [设计概述](#1-设计概述)
2. [目录结构](#2-目录结构)
3. [核心抽象](#3-核心抽象)
   - [DetectionResult — 检测结果](#detectionresult--检测结果)
   - [LoaderInfo — 加载器信息](#loaderinfo--加载器信息)
   - [Confidence — 置信度](#confidence--置信度)
   - [LoaderDetector trait](#loaderdetector-trait)
   - [LoaderDetectionService — 编排层](#loaderdetectionservice--编排层)
4. [MC 版本检测流程](#4-mc-版本检测流程)
5. [`init` 命令编排流程](#5-init-命令编排流程)
6. [Rust 实现参考](#6-rust-实现参考)
7. [Phase 策略](#7-phase-策略)

---

## 1. 设计概述

`orbit init` 需要自动识别目标目录的 Minecraft 环境——**MC 版本**是什么、**模组加载器**是什么。

| 检测目标 | 信息来源 | 方案 |
|---------|---------|------|
| MC 版本 | 游戏 JAR 内的 `version.json` | `jar.rs` 提取 → `metadata/mojang.rs` 解析 |
| 模组加载器 | 各加载器特有的痕迹文件 | **策略模式**，每个加载器一个 `LoaderDetector` |

**两层分离**：

```
jar.rs（负责打开 JAR / 遍历文件系统）
  ├─ 提取 version.json 字符串 → metadata/mojang.rs（纯解析）
  ├─ 遍历 mods/ 下 JAR → metadata/ mod.rs（策略：Extractor）
  └─ 加载器检测文件 → detection/（策略：LoaderDetector）
```

**Phase 1 策略**：检测框架完整实现，但所有 `LoaderDetector` 返回 `Confidence::None`——直接走交互式选择。后续逐步加入实际检测逻辑，每加一个不改框架。

---

## 2. 目录结构

```
orbit-core/src/detection/
├── mod.rs              # DetectionResult + LoaderInfo + LoaderDetector trait + Service
├── fabric.rs           # FabricDetector
├── forge.rs            # ForgeDetector
├── neoforge.rs         # NeoForgeDetector
└── quilt.rs            # QuiltDetector
```

与 `metadata/`、`providers/` 完全对称的策略模式结构。

---

## 3. 核心抽象

### DetectionResult — 检测结果

```rust
pub struct DetectionResult {
    /// MC 版本（从游戏 JAR 提取，总是有值）
    pub mc_version: McVersion,
    /// 加载器信息（Phase 1 可能为 None，由交互式选择填充）
    pub loader: Option<LoaderInfo>,
    /// 每个探测器的原始输出（用于日志/verbose）
    pub candidates: Vec<LoaderInfo>,
}
```

### LoaderInfo — 加载器信息

```rust
pub struct LoaderInfo {
    pub loader: ModLoader,
    pub version: Option<String>,
    pub confidence: Confidence,
    pub evidence: Vec<String>,
}
```

### Confidence — 置信度

```rust
pub enum Confidence {
    /// 确定 — 找到了加载器专属 JAR（如 fabric-loader-0.15.7.jar）
    Certain,
    /// 很可能 — mods/ 下全部是特定加载器的模组
    High,
    /// 猜测 — 目录名或版本名包含 "fabric"/"forge" 关键词
    Low,
    /// 无任何线索 — 走交互式选择
    None,
}
```

### LoaderDetector trait

```rust
/// 每个加载器实现此 trait
pub trait LoaderDetector: Send + Sync {
    /// 探测器名称
    fn name(&self) -> &'static str;

    /// 对应的加载器类型
    fn loader_type(&self) -> ModLoader;

    /// 检测目标目录，返回该加载器的存在证据和置信度
    fn detect(&self, instance_dir: &std::path::Path) -> Result<LoaderInfo, OrbitError>;
}
```

### LoaderDetectionService — 编排层

```rust
pub struct LoaderDetectionService {
    detectors: Vec<Box<dyn LoaderDetector>>,
}

impl LoaderDetectionService {
    pub fn new() -> Self { /* 注册所有 detector */ }

    /// 遍历所有 detector，收集结果，选置信度最高的
    pub fn detect_all(
        &self,
        instance_dir: &std::path::Path,
    ) -> Result<Vec<LoaderInfo>, OrbitError> {
        let results: Vec<LoaderInfo> = self.detectors
            .iter()
            .map(|d| d.detect(instance_dir))
            .collect::<Result<_, _>>()?;

        // 按置信度降序排列
        // 后续 init 逻辑取 results[0]，if confidence < Certain → 交互式
        Ok(results)
    }
}
```

---

## 4. MC 版本检测流程

MC 版本不经过策略模式——它只有一个来源，检测逻辑是固定的：

```
detect_mc_version(instance_dir)
  │
  ├─ 1. 定位游戏 JAR
  │      可能的路径（按顺序尝试）：
  │        - {instance_dir}/versions/{jar_dir}/{jar_dir}.jar  (MC Launcher 标准)
  │        - {instance_dir}/.minecraft/versions/{id}/{id}.jar  (HMCL 等启动器)
  │      如何知道版本号？
  │        - 列出 versions/ 下所有目录 → 取第一个 → 用目录名作为 jar_dir
  │        - 或读取启动器配置文件（如 HMCL 的 hmcl.json）获得 version id
  │
  ├─ 2. 打开 JAR → archive.by_name("version.json") (O(1))
  │
  ├─ 3. 读取字符串内容 → metadata::mojang::McVersion::from_json(content)
  │
  └─ 4. 返回 McVersion
```

> **Phase 1 简化**：用户必须通过 `--mc-version` 手动指定。自动探测逻辑后续实现。

---

## 5. `init` 命令编排流程

```
orbit init <name> [--mc-version <ver>] [--modloader <loader>]

  ┌─ 1. 获取 MC 版本 ───────────────────────────┐
  │   if --mc-version 指定                       │
  │     → 直接使用                               │
  │   else                                       │
  │     → detect_mc_version(dir)                  │
  │     → 失败则报错退出                          │
  └──────────────────────────────────────────────┘
                    │
  ┌─ 2. 检测加载器 ───────────────────────────────┐
  │   if --modloader 指定                         │
  │     → 直接使用                               │
  │   else                                       │
  │     → LoaderDetectionService::detect_all()    │
  │     → 选置信度最高的                          │
  │     → if confidence >= Certain → 自动使用     │
  │     → else → 交互式列表选择                   │
  └──────────────────────────────────────────────┘
                    │
  ┌─ 3. 生成 orbit.toml ──────────────────────────┐
  │   OrbitManifest {                             │
  │     project: ProjectMeta {                    │
  │       name, mc_version,                       │
  │       modloader, modloader_version,            │
  │     },                                        │
  │     resolver: default,                        │
  │     dependencies: {}                          │
  │   }                                           │
  │   → toml::to_string_pretty → 写入文件          │
  └──────────────────────────────────────────────┘
                    │
  ┌─ 4. 注册实例 ─────────────────────────────────┐
  │   InstancesRegistry::load()                   │
  │   → 添加当前目录的条目                        │
  │   → save()                                    │
  └──────────────────────────────────────────────┘
```

**交互式选择（Phase 1 的兜底）**：

CLI 输出一个选择列表让用户选加载器：

```
? Could not auto-detect modloader. Please select one:
  [1] Fabric
  [2] Forge
  [3] NeoForge
  [4] Quilt
  [5] None (vanilla / unknown)
```

---

## 6. Rust 实现参考

### mod.rs

```rust
// orbit-core/src/detection/mod.rs

use crate::error::OrbitError;
use crate::metadata::{ModLoader, mojang::McVersion};

// ── 类型 ───────────────────────────────────

pub struct DetectionResult {
    pub mc_version: Option<McVersion>,
    pub loader: Option<LoaderInfo>,
    pub candidates: Vec<LoaderInfo>,
}

pub struct LoaderInfo {
    pub loader: ModLoader,
    pub version: Option<String>,
    pub confidence: Confidence,
    pub evidence: Vec<String>,
}

pub enum Confidence {
    Certain,
    High,
    Low,
    None,
}

// ── trait ──────────────────────────────────

pub trait LoaderDetector: Send + Sync {
    fn name(&self) -> &'static str;
    fn loader_type(&self) -> ModLoader;
    fn detect(&self, instance_dir: &std::path::Path) -> Result<LoaderInfo, OrbitError>;
}

// ── 编排 ───────────────────────────────────

pub struct LoaderDetectionService {
    detectors: Vec<Box<dyn LoaderDetector>>,
}

impl LoaderDetectionService {
    pub fn new() -> Self {
        Self {
            detectors: vec![
                Box::new(super::fabric::FabricDetector),
                // Box::new(super::forge::ForgeDetector),
                // Box::new(super::neoforge::NeoForgeDetector),
                // Box::new(super::quilt::QuiltDetector),
            ],
        }
    }

    pub fn detect_all(
        &self,
        instance_dir: &std::path::Path,
    ) -> Result<Vec<LoaderInfo>, OrbitError> {
        let mut results: Vec<LoaderInfo> = self.detectors
            .iter()
            .map(|d| d.detect(instance_dir))
            .collect::<Result<_, _>>()?;

        // 按置信度降序
        results.sort_by(|a, b| confidence_rank(&b.confidence).cmp(&confidence_rank(&a.confidence)));
        Ok(results)
    }

    /// 返回已知加载器名称列表（用于交互式选择）
    pub fn known_loaders(&self) -> Vec<(ModLoader, &'static str)> {
        self.detectors.iter()
            .map(|d| (d.loader_type(), d.name()))
            .collect()
    }
}

fn confidence_rank(c: &Confidence) -> u8 {
    match c { Confidence::Certain => 3, Confidence::High => 2, Confidence::Low => 1, Confidence::None => 0 }
}
```

### fabric.rs (Phase 1 占位)

```rust
// orbit-core/src/detection/fabric.rs

use super::{Confidence, LoaderDetector, LoaderInfo};
use crate::error::OrbitError;
use crate::metadata::ModLoader;

pub struct FabricDetector;

impl LoaderDetector for FabricDetector {
    fn name(&self) -> &'static str { "Fabric" }
    fn loader_type(&self) -> ModLoader { ModLoader::Fabric }

    fn detect(&self, _dir: &std::path::Path) -> Result<LoaderInfo, OrbitError> {
        // TODO: Phase 2 — 实际检测逻辑：
        // - 检查 versions/ 下是否有 fabric-loader-*.jar
        // - 检查 mods/ 下 JAR 的 fabric.mod.json
        Ok(LoaderInfo {
            loader: ModLoader::Fabric,
            version: None,
            confidence: Confidence::None,
            evidence: vec![],
        })
    }
}
```

---

## 7. Phase 策略

| Phase | 内容 |
|-------|------|
| **Phase 1（当前）** | 框架完整实现。所有 detector 返回 `Confidence::None` → 交互式选择。MC 版本通过 `--mc-version` 手动指定 |
| **Phase 2** | 实现 `FabricDetector.detect()`：查找 fabric-loader jar、遍历 mods/ 下 JAR 读取 fabric.mod.json |
| **Phase 3** | 实现 Forge / NeoForge / Quilt detector |
| **Phase 4** | 自动探测 MC 版本：遍历 `versions/` 目录 → 读 JAR 中的 `version.json` |

---

> **关联文档**
> - [orbit-metadata.md](orbit-metadata.md) — 文件元数据解析（McVersion、ModMetadata、各加载器格式）
> - [orbit-architecture.md](orbit-architecture.md) — detection/ 模块在项目中的位置
> - [orbit-cli-commands.md](orbit-cli-commands.md) — `init` 命令的行为规格
