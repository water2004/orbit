# Orbit 版本号与约束

> 实现位置：`orbit-core/src/versions/`。

## 1. 统一入口

```rust
pub enum Version {
    Fabric(SemanticVersion),
    Maven(MavenVersion),
    Generic(String),
}
```

| Loader | 实际版本 | 约束 |
|---|---|---|
| Fabric / Quilt | Fabric SemanticVersion | Fabric predicate |
| Forge / NeoForge | Maven ComparableVersion | Maven VersionRange |
| 内部选择包 | 原始稳定字符串 | 精确/全集 |

`Version::parse()` 解析候选；`Version::parse_constraint()` 生成 PubGrub
`Ranges<Version>`。`Version::zero()` 只用于内部根包。

## 2. Fabric / Quilt

实现保留：

- 任意长度数字组件；
- prerelease；
- build metadata（展示保留，不参与 `Eq`、`Hash`、排序）；
- 无法解析版本的原始精确匹配。

支持 predicate：

- `=`, `>`, `>=`, `<`, `<=`
- 空格 AND
- `||` OR
- `x` / `X` / `*` 末尾通配
- `~`
- `^`

边界转换严格保持 Fabric 语义：`>` 不包含下界，`<=` 包含上界；通配、tilde 和 caret
的上界使用目标版本的最低 prerelease 边界，避免错误包含下一段 prerelease。

Fabric 的 caret 固定首个数值组件，不采用 npm 对 `0.x` 的特殊收窄：

```text
^1.2.3 → >=1.2.3 <2.0.0-
^0.2.3 → >=0.2.3 <1.0.0-
```

无效 SemanticVersion 只允许精确相等，不能参与有序范围。

## 3. Forge / NeoForge

`maven.rs` 是 Apache Maven `ComparableVersion` 行为的 Rust 实现，包括：

- `.`、`-` 和数字/文本转换形成的嵌套 item list；
- 任意长度数字比较；
- `alpha < beta < milestone < rc < snapshot < release < sp`；
- `a`、`b`、`m`、`cr`、`ga`、`final`、`release` 别名；
- hyphen 后的子列表和 qualifier-number combination；
- 尾随零/空 qualifier 规范化；
- 与比较一致的 `Eq` / `Hash`。

Maven range：

| 表达式 | 行为 |
|---|---|
| `1.2.3` | Maven recommendation：允许任意版本 |
| `=1.2.3` | Orbit 显式精确写法 |
| `[1.2.3]` | Maven 精确范围 |
| `[1,2)` | `>=1` 且 `<2` |
| `(,2]` | `<=2` |
| `[1,2),[3,)` | 区间并集 |

裸版本不能误当作 Maven 精确依赖；Forge 官方格式把它解释为 recommended version。
需要精确锁定时使用 `[x]`，manifest 也可使用 Orbit 的 `=x`。

## 4. resolver 边界

- manifest、override、loader 元数据和 Jar-in-Jar range 使用同一个 loader 版本模型；
- 平台展示版本不参与求解，实际值来自下载 JAR 自声明版本；
- provider 候选顺序决定多个允许版本中的偏好；
- 不同 `Version` variant 的总序只服务容器和内部包，普通依赖不会跨 loader 混用。

## 5. 测试契约

- Fabric build metadata 的 `Eq` / `Hash` 一致；
- prerelease、严格/包含边界、AND/OR、wildcard、tilde、caret；
- Maven qualifier、hyphen、数字段、精确范围、开闭范围和并集；
- resolver 中的 Forge/NeoForge、Java 与 Jar-in-Jar 实际范围。
