# Orbit 版本号与约束

> 实现位置：`orbit-core/src/versions/`

## 1. 统一入口

resolver 使用同一个 `Version` 枚举承载不同 loader 的版本语义：

```rust
pub enum Version {
    Fabric(SemanticVersion),
    Maven(MavenVersion),
    Generic(String),
}
```

| Loader | 版本模型 | 约束模型 |
|--------|----------|----------|
| Fabric / Quilt | Fabric `SemanticVersion` | Fabric predicate |
| Forge / NeoForge | Maven 风格版本 | Maven range |
| 其它 | 原始字符串 | 精确匹配 |

`Version::parse(raw, loader)` 解析实际版本；`Version::parse_constraint(raw, loader)` 生成
PubGrub 的 `Ranges<Version>`。空约束和 `*` 都表示全集。

`Version::zero()` 只作为 Orbit 内部根包版本使用，不代表任何真实模组、loader 或 Java
版本。旧版 PubGrub 所需的 `Lowest` 哨兵已经删除。

## 2. Fabric / Quilt 版本

`fabric.rs` 实现 Fabric Loader 的数字组件、预发布和 build metadata 规则。

### 解析

```text
0.8.10+mc1.21
  core       = [0, 8, 10]
  prerelease = None
  build      = "mc1.21"
```

- `+` 后的 build metadata 保留用于展示，但不参与相等、哈希和排序；
- `-` 后的 prerelease 低于无 prerelease 的同版本；
- `x`、`X`、`*` 可作为末尾组件通配符；
- 缺少的数字组件按 `0` 比较；
- 非数字 core 解析失败时，统一入口回退为 `Version::Generic`。

Orbit 不会从 `mc1.20.1-0.5.8` 一类平台展示名中猜测版本尾部。传给版本模型的值必须是
JAR 自声明版本或 manifest 约束；平台的 `version_number` 另存于 provider 专属字段。

### 比较

| 比较 | 结果 |
|------|------|
| `0.5.10` 与 `0.5.8` | 前者更大 |
| `0.8.10+mc1.21` 与 `0.8.10` | 相等 |
| `1.0-alpha` 与 `1.0` | 前者更小 |
| `1.0-beta.2` 与 `1.0-beta.1` | 前者更大 |

预发布段按 `.` 拆分。两个数字段先按长度、再按字典序比较；数字段低于文本段；两个文本
段按字典序比较。

### 约束

空格表示 AND，`||` 表示 OR：

```text
>=0.8 <0.9
>=0.14 <0.15 || >=0.16
```

支持 `=`、`>`、`>=`、`<`、`<=`、`~`、`^` 和末尾通配符。通配符会展开为半开区间，
例如 `0.8.x` 等价于 `>=0.8 <0.9`。

`~` 固定前两个组件并允许后续更新；`^` 在首组件为非零时固定首组件，在首组件为零时
固定第二组件。这些规则同时用于直接 `satisfies()` 检查和 PubGrub range 构建。

## 3. Forge / NeoForge Maven 版本

`maven.rs` 处理 `mods.toml` / `neoforge.mods.toml` 中的 `versionRange`。

### 排序

版本按 `. - _ +` 以及数字/文本边界切分。数字段去掉前导零，并按数值位数和字典序比较，
因此 `47.10 > 47.2`，`47 == 47.0.0`。

常见 qualifier 顺序为：

```text
alpha < beta < milestone < rc < snapshot < release < sp < 未知 qualifier
```

别名会先归一化：

- `a` → `alpha`
- `b` → `beta`
- `m` → `milestone`
- `cr` → `rc`
- `ga`、`final`、`release` → 正式版

这是一套满足当前 Forge/NeoForge 元数据和 PubGrub 排序需要的 Maven 风格实现，并非对
Maven `ComparableVersion` 所有边角行为的逐行移植。

### 范围

| 表达式 | 含义 |
|--------|------|
| `47.2.0` | 精确版本 |
| `[47.2.0]` | 精确版本 |
| `[47,48)` | `>=47` 且 `<48` |
| `[21,)` | `>=21` |
| `(,20]` | `<=20` |
| `(,20],[21,)` | 两个区间的并集 |

格式错误或不是范围语法的输入按精确版本处理，不会偷偷放宽成任意版本。

## 4. 与 resolver 的边界

- manifest 根约束、传递依赖和 overrides 都通过同一 loader 版本模型解析；
- Forge/NeoForge loader 自身和模组依赖都可使用 Maven range；
- 候选顺序决定多个允许版本中的 provider 偏好，版本比较只决定范围是否允许；
- 无法解析的 Fabric/Quilt 版本会保留为 `Generic`，不会改写原始版本文本；
- 不同 `Version` variant 之间的稳定排序只是满足容器和 PubGrub 的总序要求，正常依赖
  图不应混用不同 loader 的版本模型。

## 5. 测试重点

当前测试覆盖：

- Fabric build metadata 的相等与哈希一致性；
- prerelease、通配符、AND/OR、`~` 与 `^`；
- Maven 数字排序、开放/闭合区间、精确区间和区间并集；
- Forge loader 依赖在实际 resolver 图中满足 Maven range。

新增版本语义时必须同时验证直接比较、`Eq`/`Hash` 一致性和 PubGrub range 行为。
