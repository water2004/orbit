# Orbit 模组元数据解析

> 实现位置：`orbit-core/src/metadata/` 与 `orbit-core/src/jar/`

## 1. 分层

元数据处理分为两层：

```text
jar/       打开 ZIP、选择 loader reader、读取内嵌 JAR
  └─ metadata/   将 JSON/TOML 字符串解析为统一结构，不做文件 I/O
```

`metadata::MetadataParser` 是纯解析策略：

```rust
pub trait MetadataParser: Send + Sync {
    fn target_file(&self) -> &str;
    fn loader_type(&self) -> ModLoader;
    fn parse(&self, content: &str) -> Result<ModMetadata, OrbitError>;
}
```

`jar::read_mod_metadata(path, loader)` 是业务入口。它根据实例 loader 选择 reader，
读取对应元数据，并递归解析声明的内嵌 JAR。`init`、`sync`、`installer` 不直接打开
ZIP，也不直接调用具体 parser。

## 2. 支持格式

| Loader | 主元数据 | 兼容行为 |
|--------|----------|----------|
| Fabric | `fabric.mod.json` | 根目录优先，兼容一层子目录 |
| Forge | `META-INF/mods.toml` | 读取 `META-INF/jarjar/metadata.json`；`${file.jarVersion}` 从 MANIFEST 替换 |
| NeoForge | `META-INF/neoforge.mods.toml` | 兼容旧版 `META-INF/mods.toml` |
| Quilt | `quilt.mod.json` | Quilt JAR 缺少自身元数据时回退读取 Fabric 元数据 |

游戏本体 `version.json` 由 `metadata/mojang.rs` 解析；launcher 版本配置中的
`libraries` 与 `mainClass` 由 `metadata/version_profile.rs` 解析。它们不是模组
parser。

## 3. 统一结构

`ModMetadata` 保存纯元数据字段：

| 字段 | 含义 |
|------|------|
| `id` / `name` / `version` | 模组标识、展示名、自声明版本 |
| `authors` / `description` / `license` | 展示元数据 |
| `environment` | `client`、`server` 或 `both` |
| `dependencies` | `mod_id → 原始版本约束` |
| `embedded_jars` | 元数据声明的内嵌 JAR 路径 |
| `loader` | Fabric / Forge / NeoForge / Quilt |

哈希不是 metadata parser 的职责。SHA-1/SHA-256/SHA-512 由 `jar` 或安装编排层
针对真实字节计算，并写入 `orbit.lock`。

JAR reader 返回 `JarModMetadata`。该类型额外保留：

- 依赖是否 required；
- 已递归解析的 `implanted_mods`；
- loader 声明的内嵌 JAR 路径。

## 4. 格式映射

### Fabric

- `id`、`name`、`version`、`description` 直接映射；
- `authors` 兼容字符串、字符串数组和含 `name` 的对象；
- `environment = "*"` 归一化为 `both`；
- `depends` 的字符串或数组约束保持原义；
- `jars[].file` 进入 `embedded_jars`。

Fabric parser 采用逐字段容错：单个非关键字段格式异常不会丢失其它有效字段；整体
JSON 无法解析时才返回错误。

### Forge

主模组取第一个 `[[mods]]`：

- `modId` → `id`
- `displayName` → `name`
- `version` → `version`
- `authors` 同时接受官方常见字符串和字符串数组
- `description` → `description`
- 顶层 `license` → `license`

依赖只读取 `[[dependencies.<主模组 id>]]`，不会把同一个 JAR 中其它 mod 的依赖
错误并入主模组。`mandatory = false` 被标记为 optional。Forge 的 Maven
`versionRange` 原样进入依赖图，由 `versions/maven.rs` 解析。

若版本为 `${file.jarVersion}`，JAR reader 从 `META-INF/MANIFEST.MF` 的
`Implementation-Version` 取实际版本。

JarJar 内嵌路径来自 `META-INF/jarjar/metadata.json` 的 `jars[].path`。

### NeoForge

字段结构复用 Forge parser。现代格式使用 `META-INF/neoforge.mods.toml`；旧版
NeoForge 的 `META-INF/mods.toml` 仍可读取。

依赖优先使用 `type` 判断：

- `required` → required；
- `optional`、`incompatible`、`discouraged` → 不作为 required 依赖。

旧格式的 `mandatory` 仍受支持。

### Quilt

主字段位于 `quilt_loader`：

- `id`、`version` 直接映射；
- `metadata.name`、`description`、`contributors`、`license` 映射展示字段；
- `depends` 支持对象数组和旧式映射；
- `versions` 数组以 ` || ` 保留多个选择；
- `optional = true` 不作为 required 依赖；
- `jars` 支持字符串路径和 `{ file = ... }` 形式。

Quilt Loader 能加载 Fabric 模组，因此 Quilt reader 在没有 `quilt.mod.json` 时会调用
Fabric reader，而不是把 Quilt 元数据错误地伪装成 Fabric parser 输入。

## 5. 歧义处理

`MetadataExtractor::extract(entries, modloader_context)` 用于纯内存场景：

1. 收集所有命中目标文件的 parser；
2. 只有一个候选时直接解析；
3. 多个候选时按 `modloader_context` 选择；
4. 无上下文或上下文无法消歧时返回明确错误。

实际实例扫描已知 loader，通常直接走对应 `jar` reader。这样 Forge 与 NeoForge
共享 TOML 结构时仍能保留正确 loader 语义。

## 6. 内嵌 JAR

父 JAR reader 只声明内嵌路径。`jar/mod.rs` 统一完成递归：

1. 读取父元数据；
2. 按声明路径提取字节；
3. 使用同一实例 loader 解析子 JAR；
4. 成功结果写入 `implanted_mods`；
5. 非模组库不会导致父模组解析失败。

`init` 与安装流程将 implanted 模组写入父 `[[package.implanted]]`，不会把它们重复
加入 manifest 顶级依赖。

## 7. 扩展新 Loader

新增 loader 时需要显式完成以下边界，不能只添加 parser：

1. 在 `metadata/` 实现纯字符串 parser；
2. 在 `metadata::default_extractor()` 注册；
3. 在 `jar/` 实现 reader，并在 `read_mod_metadata_from_archive` 分发；
4. 如有独立版本约束语义，在 `versions/` 实现；
5. 如需自动检测，在 `detection/` 注册 detector；
6. 添加 parser、JAR 分发、内嵌和歧义测试。

通常无需修改 manifest 或 provider 数据模型；loader 特有的格式差异应留在
`metadata/`、`jar/`、`versions/` 和 `detection/` 边界内。
