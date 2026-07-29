# Orbit 版本号与约束

> 实现位置：`orbit-core/src/versions/`。

## 1. 两种相等关系

Orbit 必须同时表达：

1. **候选表示相等**：完整 JAR 声明版本和内容候选都相同；用于精确选择。
2. **版本优先级相等**：数值核心相同；用于 upgrade/downgrade 与 Pareto 支配关系。

例如 `1.2.3-alpha` 与 `1.2.3-beta` 是不同表示，也可以来自不同内容哈希，但二者的数值
核心都是 `1.2.3`。它们是不同方案，互相不算升级或降级。求解器不得把其中一个当成另一个，
也不得仅因后缀文本排序而淘汰方案。

统一入口为：

```rust
pub enum Version {
    Fabric(SemanticVersion),
    Maven(MavenVersion),
    Generic(String),
}
```

`Version::parse()` 解析候选；`parse_constraint()` 生成 PubGrub range；
`cmp_precedence()` 只比较数值核心；普通 `Ord` 保持不同具体表示可区分。

## 2. Orbit 显式约束

对 Fabric/Quilt 与 Forge/NeoForge，Orbit TOML 和 CLI 的显式运算符遵循同一规则：

| 表达式 | 行为 |
|---|---|
| `*` | 允许所有版本 |
| `=1.2.3` | 允许所有数值核心为 `1.2.3` 的表示，包括 `-alpha`、`-beta` |
| `=1.2.3-alpha` | 只允许这个完整后缀表示 |
| `!=1.2.3` | 排除整个 `1.2.3` 数值核心类 |
| `!=1.2.3-alpha` | 只排除这个完整后缀表示 |
| `>1.2.3` | 数值核心必须高于 `1.2.3` |
| `>=1.2.3` | 数值核心不低于 `1.2.3` |
| `<`、`<=` | 对称地按数值核心比较 |

是否存在显式后缀由数值核心后的 `-suffix` 决定。Fabric build metadata `+...` 仍按其
Loader 语义保留用于展示，但不构成 `-suffix` 精确条件。

为了把一个数值核心类表示成 PubGrub range，两个版本实现都提供只供内部使用的
`Before(core)` 与 `After(core)` 边界。具体后缀表示严格位于这两个边界之间，因此
`=1.2.3` 可以覆盖整类而 `=1.2.3-alpha` 仍是 singleton。

## 3. Fabric / Quilt

Fabric SemanticVersion 支持任意长度数字组件、prerelease、build metadata、AND/OR、
`x`/`X`/`*` 通配、`~` 与 `^`。这些范围的开闭边界都使用数值核心边界：

```text
^1.2.3  → >=1.2.3 <2.0.0
^0.2.3  → >=0.2.3 <1.0.0
~26.1   → >=26.1 <26.2
0.8.x   → >=0.8 <0.9
```

Fabric caret 固定首个数值组件，不采用 npm 对 `0.x` 的特殊收窄。无法解析为
SemanticVersion 的字符串只能参与具体表示的精确匹配，不能假装拥有数值顺序。

## 4. Forge / NeoForge

Maven 版本实现保留 ComparableVersion 的 item-list、任意长度数字、qualifier、别名、
hyphen 子列表及与比较一致的 `Eq`/`Hash`。Loader 元数据中的 Maven range 仍按原生语法：

| 表达式 | 行为 |
|---|---|
| `1.2.3` | Maven recommendation：不形成硬约束 |
| `[1.2.3]` | Maven 原生精确表示 |
| `[1,2)` | 数值核心 `>=1` 且 `<2` |
| `(,2]` | 数值核心 `<=2` |
| `[1,2),[3,)` | 区间并集 |

Orbit 用户策略额外接受上一节的 `= != > >= < <=`。因此 `=x` 是“无后缀则核心类、
有后缀则精确表示”，而 Maven 原生 `[x]` 始终是具体表示 singleton。

## 5. 候选与 Pareto 枚举

内容哈希是候选主键，不参与版本高低。同一包可以同时拥有：

- 不同数值核心的候选；
- 相同数值核心、不同 `-suffix` 的候选；
- 完全相同版本字符串、不同内容哈希和不同依赖的候选。

Orbit 传给 PubGrub fork 三个彼此独立的关系：

- `same_version`：同一个具体候选身份；
- `same_precedence`：数值核心相同；
- `strictly_higher`：数值核心严格更高。

fork 原生保留同一 Pareto 优先级上的不同候选实现，并排除已经保留的具体组合，避免把
同版本候选合并或无限重复。多个可行实现作为不同方案交给用户；界面显示 JAR 版本、远端
和依赖差异，不显示内容哈希。

只有 `cmp_precedence() == Greater` 才产生 upgrade；小于产生 downgrade；优先级相同而
候选不同产生 replace。`upgrade` / `outdated` 的标准版本 Pareto 支配以及 `add` / `fix`
在固定极小变更集合内的次级版本支配，都只使用这个优先级关系。`add` / `fix` 的首要关系
不是版本高低，而是相对 lock 未能保留的逻辑包状态集合按包含关系 Pareto 极小。

## 6. 版本管理命令

```text
orbit versions <package>
orbit constraint show <package>
orbit constraint set <package> <requirement>
orbit constraint clear <package>
```

`versions` 从包在 TOML 中配置的全部远端枚举当前 Minecraft/Loader 可用工件，先统一下载
和读取 JAR 元数据，再按数值核心降序、具体表示稳定排序。provider 的 release name 或
project ID 不会被当成版本。内容哈希和候选主键不进入文本、JSON 或 GUI 展示模型。

`constraint set`/`clear` 只更新 TOML，不改 lock 和磁盘 JAR。命令会报告当前 lock 选择
是否符合新策略；运行 `orbit fix` 才真正求解并应用。

## 7. 测试契约

- `=1.2.3` 接受该核心的所有后缀，`=1.2.3-alpha` 只接受精确后缀；
- 有序运算符忽略后缀；
- 相同核心不同后缀是两个 Pareto 方案，切换属于 replace；
- 相同完整版本但不同内容身份仍是不同方案；
- Fabric build metadata 的 `Eq`/`Hash` 一致；
- Maven qualifier、原生精确范围、开闭范围和并集保持 Loader 语义；
- Forge/NeoForge、Java 与 Jar-in-Jar 使用同一个版本模型。
