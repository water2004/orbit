# Orbit 模组元数据

> 实现位置：`orbit-core/src/metadata/` 与 `orbit-core/src/jar/`。

## 1. 唯一数据流

```text
JAR/ZIP I/O（jar）
  → loader 格式适配（metadata/fabric|quilt|forge|neoforge）
  → ModFileMetadata
  → 一个或多个 ModMetadata
  → JarModMetadata + bundled_mods
```

`init`、`sync`、`installer` 不直接打开 ZIP，也不直接调用具体 parser。loader 差异到
`ModFileMetadata` 为止；锁文件和 resolver 不再按 loader 复制业务路径。

## 2. 规范化模型

一个物理元数据文件由 `ModFileMetadata` 表示：

- `loader`
- `license`
- `language_loader`
- `mods: Vec<ModMetadata>`
- `embedded_jars`
- Forge-family `${file.<key>}` substitution properties

每个逻辑 `ModMetadata` 包含：

- `id`、`name`、`version`、`authors`、`description`
- `environment`
- `dependencies: Vec<DependencyExpression>`
- `provides`

依赖不再使用 `(id, version, required)` 元组。`ModDependency` 明确保留：

- `kind`
- `environment`
- `ordering`
- `reason`
- `unless`

Quilt 的嵌套 `any` / `all` 通过递归 `DependencyExpression` 保真。一个物理 JAR
声明的其他逻辑模组或嵌套模组写入 `bundled`，不伪装成独立顶层文件。

## 3. loader 适配

| Loader | 元数据文件 | 完整映射 |
|---|---|---|
| Fabric | `fabric.mod.json` | identity、environment、六类依赖、数组版本、provides、jars |
| Quilt | `quilt.mod.json` | identity、depends/breaks、any/all/unless、optional、provides version、jars；缺失时可读取 Fabric JAR |
| Forge | `META-INF/mods.toml` | 多 `[[mods]]`、mandatory、versionRange、ordering、side、reason、features、properties、language loader、JarJar |
| NeoForge | `META-INF/neoforge.mods.toml` | required/optional/incompatible/discouraged、ordering、side、features；兼容旧文件名 |

Fabric/Quilt 使用 Fabric predicate；Forge/NeoForge 使用 Maven ComparableVersion 与
Maven version range。版本约束的解释只发生在 `versions/`，parser 保留原始文本。

## 4. 严格解析

身份和结构字段错误会立即返回带文件名/字段名的错误，不再“尽量猜一个能用的结果”：

- 缺失或非法 mod ID；
- 缺失版本；
- 依赖值既不是合法字符串也不是合法数组/对象；
- Forge 缺失必需的 `modLoader`、`loaderVersion`、`license` 或旧格式
  `mandatory`；
- 未解析的 `${file.*}`；
- 声明的模组内嵌 JAR 不存在或其元数据损坏；
- Jar-in-Jar schema 字段为空、路径不存在、artifact version 不在声明 range。

普通内嵌库没有 loader 元数据时可以忽略；被明确声明为模组且元数据损坏时不能静默
吞掉。

## 5. Fabric 与 Quilt

Fabric 映射：

- `depends` → `required`
- `recommends` → `recommended`
- `suggests` → `suggested`
- `conflicts` → `discouraged`
- `breaks` → `incompatible`

`environment` 支持字符串和数组。`provides` 继承声明模组版本。

Quilt 递归保存依赖组。`unless` 是条件表达式而不是字符串标记；`breaks` 进入硬冲突。
带 group 前缀的依赖和 provides ID 在适配层归一化为实际 mod ID。

## 6. Forge 与 NeoForge

Forge-family parser 共享 TOML 骨架，但保留格式真实差异：

- Forge 旧依赖由 `mandatory` 判定 required/optional；
- NeoForge 优先使用 `type`，其中 incompatible 是硬冲突，discouraged 是 warning；
- `ordering = BEFORE/AFTER` 和 `side = CLIENT/SERVER/BOTH` 完整保留；
- `features.javaVersion` 变为正常的 `java` 依赖；
- 一个文件的多个 `[[mods]]` 全部保留，不只取第一个；
- `${file.<key>}` 在展示字段、版本、依赖、reason、license 和 language loader range
  中统一替换。

`META-INF/jarjar/metadata.json` 保存 Maven `group:artifact`、range、
artifactVersion、path 和 obfuscated。它与普通 bundled mod 是两个概念：
前者参与 artifact 版本求解，后者是同一物理文件内的逻辑模组。

## 7. 内嵌与字节码

`jar/mod.rs` 统一递归 loader 元数据声明的嵌套 JAR。结果进入
`JarModMetadata::bundled_mods`，再递归写入 lockfile 的 `bundled`。

同时扫描根目录 `.class` 文件头的 class major，并推导最低 Java 版本依赖。此检查对
四种 loader 一致；`META-INF/versions/` 不提高基础要求。

## 8. 扩展规则

新增 loader 必须完成：

1. 纯字符串 parser；
2. `jar` 分发与元数据文件选择；
3. 规范化模型映射；
4. 必要的版本语义；
5. detector；
6. parser、真实 JAR、嵌套、求解与错误测试。

不得在安装器、lockfile 或 resolver 新建 loader 专属分支；只有格式确实不同的适配点
允许分开。
