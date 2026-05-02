# Orbit 版本号解析与比较设计

> 本文档定义 `orbit-core/src/versions/` 的架构、Fabric 版本语义的 1:1 复刻逻辑及约束检查规则。

---

## 目录

1. [设计动机](#1-设计动机)
2. [模块结构](#2-模块结构)
3. [Fabric SemanticVersion](#3-fabric-semanticversion)
   - [解析规则](#解析规则)
   - [比较规则](#比较规则)
   - [约束检查](#约束检查)
4. [Rust 实现](#4-rust-实现)

---

## 1. 设计动机

Minecraft 模组版本号不遵循标准 SemVer——后缀（`+mc1.21`、`-hotfix`）语义依赖于具体加载器。Orbit 不能自己定义"正确"的比较方式，而应忠实复刻各加载器的官方实现。

```
versions/
├── mod.rs       # VersionScheme trait（各 loader 实现）
└── fabric.rs    # 1:1 复刻 Fabric Loader 的 SemanticVersionImpl
```

**核心原则**：不发明版本规则，只搬运官方实现。Fabric 怎么做，Orbit 就怎么做。

---

## 2. 模块结构

```
orbit-core/src/versions/
├── mod.rs          # VersionScheme trait + 通用工具
└── fabric.rs       # Fabric SemanticVersion（1:1 复刻）
```

`mod.rs` 定义 `VersionScheme` trait：

```rust
pub trait VersionScheme: Ord + Clone {
    fn parse(raw: &str) -> Self;
    fn satisfies(&self, constraint: &str) -> bool;
}
```

各加载器（Fabric、Forge、NeoForge）各自实现此 trait，`resolver` 按加载器类型选择对应实现。

---

## 3. Fabric SemanticVersion

对应 fabric-loader 源码：
- `SemanticVersionImpl.java` — 解析与比较
- `VersionPredicateParser.java` — 约束检查

### 解析规则

```
输入: "0.8.10+mc1.21.11"

1. 按 + 拆 → core="0.8.10", build="mc1.21.11"（build 忽略）
2. 按 - 拆 → core="0.8.10", prerelease=None
3. core 按 . 拆 → components=[0, 8, 10]

特殊: x/X/* 在末位表示通配符（COMPONENT_WILDCARD）
      "1.0.x" → components=[1, 0, WILDCARD]
      "1.x"   → components=[1, WILDCARD]
```

| 输入 | components | prerelease | build | 通配符 |
|------|-----------|------------|-------|--------|
| `0.5.8` | `[0,5,8]` | — | — | — |
| `0.8.10+mc1.21` | `[0,8,10]` | — | `mc1.21` | — |
| `1.0-alpha` | `[1,0]` | `alpha` | — | — |
| `0.5.8-hotfix` | `[0,5,8]` | `hotfix` | — | — |
| `0.8.x` | `[0,8,W]` | — | — | ✓ |
| `1.x` | `[1,W]` | — | — | ✓ |

> `W` = `COMPONENT_WILDCARD` = `i32::MIN`

### 比较规则

```
Ordering:
  for i in 0..max(len):
    if either component is WILDCARD → skip
    if a[i] > b[i] → Greater
    if a[i] < b[i] → Less
  if all components equal:
    if both have prerelease → tokenize by '.' compare each
      numeric part: compare by length then value
      text part: text > numeric
    if only one has prerelease → prerelease < no-prerelease
    if neither → Equal
```

| 比较 | 结果 | 原因 |
|------|------|------|
| `0.5.10` vs `0.5.8` | `>` | 10 > 8 |
| `0.8.10+mc1.21` vs `0.8.10` | `=` | build 忽略 |
| `1.0-alpha` vs `1.0` | `<` | prerelease < release |
| `0.5.8-hotfix` vs `0.5.8` | `<` | prerelease < release |
| `0.28.3` vs `0.28.3-` | N/A | 空的 `-` 后缀视为无 prerelease |

### 约束检查

对应 Fabric `VersionPredicateParser`：

```
输入: ">=1.0 <2.0"
1. 按空格拆 → [">=1.0", "<2.0"]
2. 各自解析 operator + version
3. 全部满足 → true

通配符处理:
  "0.8.x" (operator 必须是 =) 
    → 转换为 >=0.8 <0.9
  "1.x"
    → 转换为 >=1 <2
```

| 约束 | `0.8.10` 满足？ |
|------|:---:|
| `>=0.8` | ✓ |
| `<0.9` | ✓ |
| `>=0.8 <0.9` | ✓ |
| `0.8.x` | ✓（展开为 `>=0.8 <0.9`） |
| `>=6.7.1 <6.8` | ✗ |
| `*` | ✓ |

---

## 4. Rust 实现

### 类型

```rust
pub struct SemanticVersion {
    pub raw: String,
    components: Vec<i32>,        // WILDCARD = i32::MIN
    prerelease: Option<String>,  // - 之后
    build: Option<String>,       // + 之后，比较时忽略
    has_wildcard: bool,
}
```

### 入口

```rust
// 解析（启用 x 通配符）
let ver = SemanticVersion::parse("0.8.x", true)?;

// 约束检查
satisfies(&ver, ">=0.8 <0.9") → true
satisfies(&ver, "0.8.x") → true

// 直接比较
ver > SemanticVersion::parse("0.8.5", false)?
```

### 依赖

无需额外 crate。纯标准库实现。

---

> **关联文档**
> - [orbit-resolver.md](orbit-resolver.md) — resolver 调用 versions 进行版本约束校验
> - [orbit-architecture.md](orbit-architecture.md) — versions 模块在项目中的位置
