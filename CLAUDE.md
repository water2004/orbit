# Orbit 项目开发规范

> 由三轮 Code Review 总结。**每次修改代码前必须遵守。**

---

## 架构铁律

1. **CLI 不包含任何业务逻辑**。`orbit-cli` 只做：clap 参数解析 + 调 core API + 格式化输出。TOML 解析、文件 I/O、依赖图操作全部在 `orbit-core`。
2. **core 层不输出到 stderr/stdout**。调试用 `tracing`，用户可见进度通过返回值传递给 CLI 层展示。
3. **依赖方向**：`cli → core → wrapper`。wrapper 之间不互相依赖。core 不依赖 cli。
   每个平台的 HTTP client、请求参数、响应 DTO、分页和传输错误属于独立 wrapper；
   core 的 provider 只做 wrapper 类型到 Orbit 领域类型的适配。
4. **lockfile 是已安装依赖图查询的唯一数据源**。`find_entry`、`dependents`、`check_version_conflict` 集中在 `resolver/`；候选求解图则明确组合 manifest 根约束、lockfile 当前图和候选 JAR 元数据。CLI 不手工重建依赖图。
5. **主键使用 JAR `mod_id`，slug 只作平台查找别名**。manifest dependency key 和 `PackageEntry.mod_id` 是图中的键；平台子表的 slug 仅供 `find_entry` 等用户输入匹配。human-readable name 不可靠。

---

## 编码规范

6. **`todo!()` 和不可达的空壳函数禁止进入业务 crate**。外部服务边界（缺认证、无 API 下载许可等）必须返回包含恢复建议的显式 `OrbitError`，不能伪造成功。
7. **CLI handler 必须接入 core 的真实入口**。参数错误交给 clap；业务失败返回错误，不能以 `println! + Ok(())` 掩盖。
8. **写入 manifest/lockfile 时传 `mods_dir` 作为参数**——禁止硬编码 `Path::new("mods")`。
9. **`apply_to_lockfile` 使用每个 `InstalledMod.provider` 的真实来源**——禁止硬编码 `"modrinth"`；传递依赖只写 lockfile，不得自动提升为 manifest 顶级声明。
10. **每模组独立记录 provider 来源**——`InstalledMod.provider` 字段，不能假设所有 deps 来自同一平台。

---

## API 调用规范

11. **先调 API 确认返回值再编码**。不确定字段是否存在/什么格式时，用 curl 调一下看实际响应。
12. **优先使用批量 API**。Modrinth hash/project 与 CurseForge fingerprint/project 等批量端点将 N 次请求压缩为 1 次。公共识别入口是 `identify_artifacts`，不能退化成逐文件 N+1 查询。
13. **404 转 `ModNotFound`**。`map_api_error()` 统一处理，CLI 收到后触发搜索回退。
14. **错误响应保留 body**。`error_for_status()` 会丢弃 body。先读 body 再检查状态码。

---

## Provider 规范

15. **用 `create_providers()` 工厂**，不直接 `ModrinthProvider::new("orbit", 3)`。
16. **`RateLimiter::acquire()` 返回 `Result`**，调用方加 `?`。内部方法（如 `lookup_project_slugs`）不获取 permit。
17. **哈希字段必须名实一致**。`RemoteArtifact.sha512` 只存 SHA-512；Modrinth 优先用 SHA-512，CurseForge API 只提供 SHA-1 时用 SHA-1 校验，下载后再计算三种哈希。
18. **下载必须校验来源提供的强哈希**。Modrinth 校验 SHA-512，CurseForge 校验
    SHA-1；写入 lockfile 后，已存在的 JAR 优先比对本地计算的 SHA-256。

---

## 交互设计

19. **`--dry-run` 在下载前拦截**，不能在下载+写盘后才跳过 toml/lock 写入。
20. **`--yes` 只跳过写入前确认，不替用户做歧义选择**：多个真实包身份、多个 Pareto
    解、搜索候选和模糊包名都必须由交互层明确选择；stdin 关闭或协议响应无效必须取消，
    禁止静默选择第一个。唯一候选仍自动采用。
21. **找不到 slug 时搜索并交互式选择**，不能直接报错退出。

---

## 版本号

22. **`||` 是 OR 分隔符**。`satisfies()` 先按 `||` 拆组，组内空格 AND。不能按空格拆分后把 `||` 当版本号解析。
23. **版本约束必须生效**。`resolve()` 中用 `SemanticVersion::parse` + `satisfies()` 过滤。
24. **`get_versions()` 必须传 loaders/game_versions 过滤参数**。

---

## 数据结构设计

25. **Provider 专属字段进子 struct**。公共类型（`RemoteArtifact`、`PackageEntry`）不扁平存放平台专属字段。Modrinth 的 project/version 数据放在 `modrinth`，CurseForge 的 project/file 数据放在 `curseforge`；公共编排通过统一 source helpers 读取。
26. **key 统一用 JAR loader 元数据声明的 ID**（即 `mod_id`）。slug 只在 `find_entry` 中作为备选匹配键，不用作主键。

## JAR 模块

27. **所有 JAR 元数据读取走 `jar` 模块**。`init.rs`、`installer.rs` 不直接打开 ZIP、不直接调用具体 parser。调用 `jar::read_mod_metadata(path, loader)`，由 jar 模块按 Fabric、Forge、NeoForge、Quilt 分发到对应 reader。
28. **`loader` 参数由调用者传入，禁止 auto-detect**。一个 JAR 可能同时兼容多个 loader（同时含 fabric.mod.json 和 META-INF/mods.toml），auto-detect 会选错。

## 文件 I/O

29. **manifest/lockfile 文件读写统一走 `ManifestFile` / `Lockfile` 封装**。其他模块不直接调 `std::fs::write` 操作 orbit.toml / orbit.lock。初始化用 `ManifestFile::new(dir, manifest)` + `save()`，运行时用 `ManifestFile::open(dir)` / `Lockfile::open(dir)`。
30. **`Lockfile::open_or_default(dir, meta)` 处理锁文件不存在**。不需要每个调用方手写 `if path.exists() { from_path } else { default }`。

## Resolver

31. **候选求解统一通过 `graph::build_solver_graph()` 构图**。平台包、lockfile、完整离线候选、root 和未知引用包的注册不得重新散落到求解循环。resolver 禁止持有 provider 或动态补抓候选。
32. **lockfile 条目必须携带真实依赖进入候选求解图**。否则升级求解无法解释当前版本和候选版本之间的传递依赖冲突。`check_local_graph()` 是另一条纯本地输入路径，共享平台包注册规则，但不伪造或注入 lockfile。
33. **成功求解中的候选淘汰原因只来自同一次求解的类型化 `SolverEvent`**。禁止解析 debug 日志，禁止用第二次反事实求解替代实际的传播、decision 和 backtrack 路径。

## 代码卫生

34. **写完功能立即检查是否有死代码**：未用的函数、struct、trait、import、依赖项全部删除。
35. **字段命名必须准确**：存 SHA-512 就叫 `sha512`，不叫 `sha256`。
36. **`expect()` / `unwrap()` 只在不可能失败时使用**。library crate 优先返回 `Result`。
37. **修复一个问题时检查所有同类问题**（如一个 stub 改 exit(2) 就要全部改）。

## core 层输出

38. **`orbit-core/src` 当前不包含 `println!` / `eprintln!` / `eprint!`**。继续保持：用户可见结果和依赖诊断通过结构化返回值传递给 CLI；纯调试信息使用 `tracing`。

---

## 文档同步

39. **代码改动后同步更新 docs/**：`orbit-resolver.md`、`orbit-status.md`、`orbit-architecture.md`。
40. **modrinth-docs 是 API wrapper 的规格来源**，模型字段变更时同步更新。
41. **CurseForge provider 没有 API Key 就不能创建或使用**。Core API 与 artifact
    下载都使用用户配置的 Key；Key 仅保存在运行时 downloader 中，不进入
    `RemoteArtifact`、lockfile、日志或错误正文，并且只能通过 HTTPS 发往
    `forgecdn.net` 及其子域。重定向必须逐跳重新校验。
42. **Provider 只提供远端 artifact 定位信息**。slug、project ID、展示版本、远端
    dependency relation 都不能成为包身份或版本约束；`mod_id`、版本、依赖、环境、
    provides 和 bundled 只接受下载后 JAR loader metadata。
43. **下载与求解严格分层**。下载层先按 provider project relation 递归枚举目标
    Minecraft/loader 的完整 artifact 闭包，再统一从缓存/网络取得并解析；resolver
    纯离线消费候选。禁止从 JAR `mod_id` 反查 slug，禁止 resolver 动态联网补抓。
44. **全局路径必须注入**。配置、实例注册表和 JAR 缓存通过 `RuntimeContext` /
    `RuntimePaths` 传递；业务模块禁止读取 `APPDATA`、`HOME`、XDG 或
    `current_exe()`。平台目录发现只存在于 `RuntimeEnvironment` 实现。
45. **求解包恒为 JAR 声明的 `mod_id`**。文件名、provider slug/project ID、下载
    URL、owner/path 都只能区分候选，禁止重新引入“一个物理 JAR 一个求解包”的轴。
    同一 `mod_id` 的多个顶层 JAR 是同一个包的候选版本；同声明版本不同来源仍是不同
    候选，但不是升级。
46. **包内容与操作单元必须分开**。一个顶层包可以包含多个同文件模块、嵌套模组 JAR
    和普通库；不是每个 JAR 都是包。contained 模块必须绑定 owner 候选，普通库不单独
    求解。安装和删除只作用于顶层 `mods/*.jar` 包文件，禁止独立删除嵌套内容。
47. **loader 加载条件必须进入共享图**。Fabric nested 使用 `if_possible`；Quilt 保留
    `always` / `if_possible` / `if_required`；Forge-family JarJar 按 artifact range
    选择。相同 ID 的多版本嵌套候选选择一个兼容项，不能要求所有候选同时成立。
48. **所有求解型包集合变更共享 portfolio 与事务报告**。add、本地 add、fix、结构化
    constraint set、upgrade、migrate 等不得各写选择规则。唯一 Pareto 解自动选择，多解交给交互层；
    降级、替换和未选包删除即使在唯一解中也必须展示精确逻辑包版本动作并在写盘前确认。
    物理 JAR 文件名仅供事务层定位载体，不进入方案选择或包操作 UI。
49. **upgrade 的定义是至少一个包相对当前安装版本变新**。允许同一方案中的其他包
    降级、换源或删除；不得要求所有包都不降级，也不得把同版本不同候选算作升级。
50. **优先使用 fork 的通用求解抽象**。`P = mod_id`、不透明复合 `V`、调用方定义
    `same_version` / `strictly_higher` 和 maximal-solution enumeration 已由 fork 原生
    支持。来源身份可以属于 `V`，但枚举排除和固定其它包时必须使用 JAR 声明版本的
    `same_version` 等价类，不能把同声明版本的不同来源扩成用户多解。fork 必须拒绝不含
    当前值或与严格更高范围重叠的等价类。完整方案集采用标准 Pareto 支配：不存在另一
    可行方案让全部已选投影包保持等价或变高，且至少一个严格变高；每个 Pareto 点必须
    一次排除其支配的完整区域，不能退回逐个局部组合阻塞。若新的依赖语义无法由通用
    constraint/observer/enumeration 表达，应在 fork 增加通用能力并测试，禁止在 Orbit
    侧反事实求解、黑盒探测或结果后修补。
51. **一个 provider locator 可以对应多个真实包身份**。catalog 必须按每个 artifact
    的 JAR `mod_id` 分区；add 分别求可行解并在多个可行身份间交互选择，upgrade 固定
    lockfile 身份。禁止重新引入“一 locator 一 mod_id”的校验。
52. **loader 本身也服从真实 JAR 元数据规则**。从 launcher profile 定位实际 loader
    library JAR，把其依赖、provides 和 contained 模块注册到共享平台图。不得把 loader
    永久建成无依赖叶子，也不得硬编码某个 loader 自带模块作为例外。
53. **平台路径是快照，不是发现索引**。`sync` 必须从当前游戏目录、launcher
    profile/组件和 libraries 重新探测 Minecraft/loader JAR，再刷新 `[platform]`
    的路径与 SHA-256；不能依赖旧 TOML 文件名。
54. **平台版本变化分级处理**。`install` 发现实际 Minecraft 版本与 manifest 不一致时
    拒绝并要求先 sync；loader 版本变化本身不拒绝，必须用实际 loader JAR 进入共享图，
    由真实依赖约束决定是否兼容。
55. **init 只接受合法游戏目录**。支持标准共享根、`versions/<实例>` 隔离目录、
    Prism/MultiMC、CurseForge 和 GDLauncher 的实际 game directory；空目录或任意
    `mods/` 目录不得仅凭命令行参数初始化。多候选必须消歧，不按扫描顺序取第一个。
56. **长事务进度必须是 core 强类型事件**。core 不直接渲染，CLI 负责终端/文本展示；
    并发下载报告稳定队列的完成数，求解进度使用 fork observer 的 enumeration
    run/maximality probe start/finish 动态扩展总量。禁止解析 solver 日志或用定时假进度。
    成功的 Pareto 提升 probe 属于保留解的真实路径；失败 probe 必须通过边界结果回滚
    observer 状态。动态进度只证明当前工作仍在推进，不是剩余时间上界；Pareto/co-Pareto
    front 本身仍可能很大。
57. **字节码 mutation 的行为和位置精度必须正交建模**。禁止恢复 `exclusive: bool`
    或把未知 InjectionPoint 改写成 UnknownMethod；Redirect、破坏性替换、value/argument
    decorator、operation wrapper、相邻插入、局部值和结构变化使用显式组合语义。双方都是
    Instruction 精度时只以 stable ID 判同；至少一方为 Pattern 时才比较 opcode/member/
    constant。
58. **Mixin cardinality 属于完整 InjectionQuery**。slice ID 与 boundary 先解析并限制
    搜索区间，再应用 selector、ordinal、shift；无法解析不得回退成全方法精确匹配。
    require/allow 按 query 总匹配数计算，expect 只保存为调试预期，Group 按 Mixin class +
    group name 聚合。cancellable 与 ModifyVariable 不得泛化为控制流/局部布局 High。
59. **软引用 confidence 逐引用决定**。DirectExact 与 RefmapExact 不警告；只有
    Ambiguous/Unresolved 警告。Artifact 是否含任意 refmap 不能改变无关引用的 confidence。
    Transformer 启发式必须为 Pattern/Method + Low，未知 target-branch 关联不得做笛卡尔积。
60. **audit 默认文本与详细报告分离**。默认终端只显示摘要、有限高排名风险和 warning
    分类，不展开 Evidence.detail。文本必须通过统一 output 层渲染环境、coverage、
    warning、风险分布和两列风险详情表；TTY 服从终端宽度，非 TTY 最大 120 列。完整
    evidence 只进入 JSON stdout 或用户显式指定的 `--report`；默认运行不得创建报告文件。
61. **升级失败诊断不能随方案筛选丢失**。outdated、批量 upgrade 和单包 upgrade 共享
    upgrade resolution selector；单包模式必须升级指定 `mod_id`，不能由无关包升级满足。
    没有可行升级时保留所有 Pareto 解的候选排除推导；没有适用 JAR 时报告
    NoCompatibleCandidate，禁止表述成“已是最新”。多解 UI 只显示一次共同动作，并以
    `◆` 和终端样式标记每个选项的差异，不能只靠颜色传达语义。
62. **audit 进度也必须来自 core 强类型事实**。依次报告输入准备、readiness、顶层
    artifact、Mixin、Transformer 和冲突比较；已知总量使用真实计数，plain 输出节流。
    禁止定时假进度、解析日志或在进度中打印物理 JAR 文件名。
63. **GUI 是严格的原生进程薄壳，不是第二业务实现**。`orbit-gui` 只能调用同目录或用户
    明确选择的 `orbit` / `orbit-launcher`，使用共享 schema 的 stdout/stderr/stdin；禁止链接
    core、扫描 PATH、直读业务文件、增加 GUI 专用接口或在失败后走兼容路径。wgpu 是原生
    D3D12/Vulkan renderer，不代表 WebView。
64. **运行时版本选择必须来自 Launcher 官方目录**。Minecraft、Loader 和 Java 要求由
    `versions minecraft|loader|java` 返回，并与 install 共用 metadata adapter。新建实例走
    `install --new`；版本升级/迁移必须创建独立目标实例，禁止原地 configure 源实例。GUI
    不得以自由文本、版本字符串猜测或安装失败重试代替目录与兼容关系。
65. **GUI 必须渲染领域任务而不是命令表单**。Mods 更新显示可行升级和未升级诊断，方案
    选择突出差异并展示包级删除；Runtime 显示当前/目标/Java；长任务显示真实进度和取消。
    CLI 参数只属于进程桥，不得作为主要用户交互层级。
66. **主题不能代替界面设计**。GUI 必须把跟随系统、浅色、深色与强调色作为独立、持久的
    展示策略；任一主题下都要保持同一信息层级、可读对比度和非颜色差异提示。页面实现放在
    `app/pages/`，`app.rs` 只保留共享状态、进程调度与动作控制，禁止重新堆成单文件命令表单。
67. **CLI 与 GUI 共用一种语言模型**。`orbit`、`orbit-launcher` 和 `orbit-gui` 只支持
    `system`（缺省）、`en`、`zh-CN`；CLI help、文本结果、进度、询问与错误在展示边界翻译，
    core/domain 类型和 JSON 字段、枚举码、错误码不得本地化。GUI 每次调用 CLI 都显式传递
    当前语言，不得维护第二套命令语义。
68. **机器协议编码固定为严格 UTF-8**。不得依赖或猜测 Windows ACP/OEM code page；真实
    控制台交给 Rust 标准库 Unicode 输出，管道/重定向使用 UTF-8。GUI 对 stdout/stderr 严格
    解码，非法字节必须产生 protocol error，不得使用 lossy replacement 或静默丢行。
69. **安装、对账和修复是三种不同职责**。`install` 只物化现有 lock 的精确内容，禁止发现
    候选、求解、删除包或改写 TOML/lock；`sync` 可以联网做批量哈希来源识别，但只根据当前
    平台与本地 JAR 重建事实状态和补充 TOML，禁止求解或删包；`fix` 是完整图修复入口。
    结构化 `constraint set` 是唯一有界例外：先在内存 manifest 修改一个已有包的版本策略，
    再复用 fix 的 Pareto 极小 portfolio、选择、确认和同一文件事务；无解、取消或应用失败不得
    写入策略。`init` 与 sync 一样不选择重复实现；存在重复时保留文件和来源，交给 fix。
70. **迁移必须针对真实目标运行时规划**。`migrate check` 与 `migrate export` 共用一个联合
    依赖图规划器；目标是 Launcher 已安装的准确实例目录，平台 JAR、路径、哈希和 Loader
    元数据从该目录探测。禁止按目标版本逐包探测、伪造 platform snapshot 或在 export 后
    重新走另一条求解路径。export 只写目标 TOML/lock 与配置，JAR 物化仍只由目标中的
    `orbit install` 完成。`--source-pack` 只冻结源实例事实，不能替代真实目标探测。
71. **GUI 的整合包和迁移动作只能编排现有 CLI**。Orbit ZIP/TOML 与 Modrinth mrpack 导入
    走 `orbit import -> orbit fix`；两种导出分别走 `orbit export --format zip|mrpack`；迁移走
    `源 orbit export -> Launcher install --new -> orbit migrate export --source-pack -> 目标
    orbit install`。源导出失败时禁止创建目标；取消任一确认后不得偷偷继续后续阶段，GUI
    禁止自行解析归档或生成 TOML/lock。
72. **GUI 动画只描述界面状态变化**。页面、向导步骤、模态框、抽屉和提示可以使用 GPUI
    原生短过渡；任务进度只能来自 CLI 强类型事件，禁止用定时动画制造下载、求解或安装进度。
73. **便携包只有一个导出事实管线**。普通 Orbit ZIP 和迁移源快照必须共用同一个 core
    exporter，包含校验后的包、TOML/lock 与允许的配置根；已经压缩的 JAR 使用 ZIP Stored，
    不能再次 Deflate。校验和写入按真实字节发强类型进度，失败输出必须由事务临时文件清理。
    `migrate export --source-pack` 必须从该快照规划，成功确认后才可消费临时源包。
