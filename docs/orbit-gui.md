# Orbit GUI

`orbit-gui` 是 Orbit 套件的跨平台 Rust 原生桌面前端。它不链接 `orbit-core` 或
`orbit-launcher-core`，也不实现下载、求解、安装、账户或启动业务；唯一边界是启动安装在
同一目录的 `orbit` / `orbit-launcher` 进程，并使用两者已有的 JSON/NDJSON/stdin 协议。
Launcher 的安装、账户、Java、停止和状态由 GUI 直接调用 Launcher；客户端/服务端启动统一
调用 `orbit launch`，由 Orbit 单向包装 Launcher 并注入 Runtime Agent。GUI 不注入 Agent、
不合并归属账本，也不自行组合删除路径。

## 原生技术边界

界面使用 Zed Industries 的 Apache-2.0 `gpui` 与 Longbridge 的 Apache-2.0
`gpui-component` 控件库；后者不是 Zed 官方控件集。窗口、文本、输入、滚动和动画均为原生
GPUI 元素，不包含 WebView、浏览器或 HTML/JavaScript 运行时。滚动直接消费平台连续滚轮
事件，触控板的惯性阶段不会被离散成固定行数。远端项目图片由原生 HTTP 图片客户端读取
CLI 返回的展示 URL；已安装模组与账户头像只读取 CLI 返回的本地规范化图片路径。图片从不
参与包身份、版本或依赖判断，GUI 也不打开 JAR、解析皮肤或猜测缓存位置。

GUI 默认只接受与自身相邻的 `orbit(.exe)` 和 `orbit-launcher(.exe)`，也允许用户在设置页
明确选择准确路径。它不扫描 `PATH`，不链接 core，不在 CLI 失败后改走文件直读或兼容 API。
Windows MSI 的完整安装档位把三个程序安装在同一目录；Linux 的三个独立 deb 都把各自程序
安装到 `/usr/bin`，其中 GUI 包精确依赖同版本的两个 CLI，并独占 desktop entry 与 scalable
图标。Windows MSI 也允许只安装 Orbit，或安装 Orbit + Launcher；没有 GUI 时不会创建桌面
入口。无图形 Linux 服务器只需安装 Launcher，需要模组管理时再安装 Orbit。

界面语言、主题和强调色都是独立的展示策略。语言可选跟随系统（默认）、English 或简体中文；
GUI 将同一个显式 `--language system|en|zh-CN` 传给每次 CLI 调用，因此窗口文本、CLI 提示、
进度与结构化错误保持一致。中文模式从系统字体数据库选择微软雅黑、Noto Sans CJK SC、思源
黑体等真实 CJK 字体，不硬编码某个平台的字体路径；系统缺少可用中文字体时明确报错。偏好由
GUI 自己的展示偏好保存在独立 `preferences.json`。页面的信息架构、空状态和任务流不依赖
某个主题或语言才能成立。

源码同样按这一边界分层：`app/mod.rs` 保存共享状态与 GPUI shell，`app/controller.rs` 是唯一
命令动作和结果归并入口；`app/pages/` 分别实现 Home、Library、Discover、Audit、Runtime、
Accounts、Server、Settings 与 Activity；`process.rs`/`wire.rs` 是唯一进程协议入口，
`theme.rs` 只处理共享展示 token，`model.rs` 只承载稳定 view model。页面不能自行读取 TOML、
lock、JAR、账户或 Launcher 存储。

## 单一进程协议

每次操作启动一个 CLI 子进程：

- stdout：一个 schema 2 success envelope；
- stderr：schema 2 progress、interaction、error envelope 或受限日志；
- stdin：同一进程上的 schema 2 `interaction_response`；
- 取消：终止整个子进程组/Windows Job，包含 installer 或 Java 子进程。

Windows 上 `orbit-gui.exe` 始终使用 GUI 子系统，GUI 创建的 CLI 进程组始终带
`CREATE_NO_WINDOW`；交互完全走上述管道，不会为 GUI 本身或每次 CLI 调用创建控制台窗口。

这三条协议流都明确规定为 UTF-8，而不是“当前代码页”。Orbit 的 Rust CLI 在 Windows 真实
控制台上通过标准库的 Unicode 控制台实现输出，重定向/管道则写入 UTF-8 字节；GUI 对管道
执行严格 UTF-8 解码，遇到无效字节会显示带字节偏移的 protocol error，不按 ACP/OEM code
page 猜测，也不以替换字符掩盖损坏。JSON 的字段名、枚举码和 schema 不随语言改变；只有
面向用户的 message、prompt、choice label 与文本模式输出会本地化。

GUI 对 schema 严格匹配；旧 schema 直接显示 protocol error，不猜字段。Orbit 的包身份、
Pareto 方案和写盘确认都在 CLI/core 的同一执行路径中产生，GUI 只将暂停点渲染为可读卡片。
独立的迁移删包取舍会由同一个 CLI 进程依次发出多个 `resolution` interaction；prompt 明确
标注当前因子序号、因子总数和所代表的完整组合数，GUI 不在本地展开组合。
方案中共同动作只显示一次，`different: true` 的选项差异用 `◆` 与文字同时突出。物理 JAR
文件名和候选哈希不作为包名或方案标题展示。resolution 选项只按包显示安装、升级、降级、
替换、移除或保持以及版本变化，不显示 `warnings=0`、`diagnostics=0` 或 JSON 字段计数。
交互选项卡必须使用内容自适应高度，方案列表在唯一的独立滚动区内滚动；模态遮罩截断滚轮
传播，不能带动后层页面。标题、选项与取消按钮不能重叠。

## 交互模型

### Runtime

Runtime 页使用 Launcher 的官方只读目录，而不是自由输入后试错：

1. `versions minecraft` 返回 Mojang 完整版本清单、类型、时间和 latest 标记；
2. 选择精确 Minecraft 后，`versions loader` 返回 Fabric、Quilt、Forge 或 NeoForge 官方
   来源声明的兼容版本；
3. `versions java` 返回官方 version JSON 要求的 Java component/major；
4. `instance show` 同时给出 desired intent 与 installed lock 摘要，界面突出当前/目标差异；
5. 新建实例直接调用 Launcher 的 `install --new` 事务；跨版本升级不改写源实例，而是进入下述
   新实例迁移流程；同 Minecraft、同 Loader 类型的 Loader 版本更新调用
   `instance configure --loader-version` 后执行 Launcher `install`，若实例已由 Orbit 接管，
   最后调用 `orbit sync` 记录更新后的真实 Loader 工件。

版本清单默认显示正式版，并将 `release`、`snapshot`、`old_alpha/old_beta` 分成正式版、
快照、历史版本三个互斥频道；“全部”是显式选择，不能再用“非正式版”冒充快照。

客户端只使用 Launcher 托管的 Minecraft 仓库，并把完整实例目录固定为
`<minecraft-directory>/instances/<instance-name>`；该目录拥有精确 `minecraft.jar`，并在实际
使用时按需产生 `mods`、`config`、`saves` 等可变数据，共享仓库根只保留 assets/libraries
等不可变内容。
该布局是 Launcher 的实例隔离策略，不是自称 Mojang 标准实例格式，也不
生成 `<instance-name>.json`。服务端仍选择一个明确目录。设置页通过 `orbit-launcher minecraft directory`
显示仓库，并通过 `orbit-launcher minecraft move` 迁移整个仓库；GUI 不自行移动文件。

Java 不单独猜版本。安装事务根据目标 Minecraft 自动下载并验证 Mojang managed runtime。
Java 设置页列出、完整校验和清理未使用 Java；任一注册实例 lock 仍引用的 runtime 不能删除。
Runtime 页只负责创建、导入、更新和启动实例，避免同一管理动作出现两个入口。

跨版本迁移也由 Runtime 页编排领域流程：先从源实例调用同一个 `orbit export` 管线生成并
校验便携模组源包，再调用 `orbit-launcher export` 生成与目标无关、包含世界的游戏状态包；
两个导出都成功后才用 `orbit-launcher install --new ... --from <状态包> --consume-from`
创建并安装真实目标实例。客户端世界来自隔离 game directory 的 `saves/`；服务端世界来自
工作目录下 `server.properties` 的 `level-name`（缺省 `world`）。服务端属性先由目标
Minecraft 生成字段集合，再只迁移目标仍支持的同名值，跳过字段作为结构化结果显示；EULA
不迁移。GUI 随后先
调用 `orbit migrate check <目标目录> --source-pack <源包>`，把目标 Minecraft/Loader 上的
联合求解结果、升级/降级/替换/删除与诊断呈现为专用审阅页；用户接受后才调用
`orbit migrate export <目标目录> --source-pack <源包> --consume-source-pack`，最后在目标调用
`orbit install`。GUI
不自己拼目标 TOML、不逐包检查兼容性，也不链接 Orbit core；迁移联合求解完全属于
Orbit CLI/core。用户取消迁移方案或写盘确认时，GUI 不会继续调用目标 `install`，源实例始终
保持不变。目标状态成功写入后 GUI 调用 `orbit instances register <name> <path>`，由 Orbit CLI
校验两份状态文件并写入自己的全局实例注册表，再执行精确 `install`；因此即使下载中断，已经
成立的 Orbit 工作区仍可从全局列表继续恢复。GUI 不直接改注册表。目标的 `mods/`、`config/`
等可变目录不由 Launcher 预先制造；和 HMCL 的隔离
运行目录语义一样，领域命令在第一次真正物化对应内容时创建它们。

已选择实例的 Runtime 操作区提供“打开实例目录”，只使用 Launcher `instance show/list` 返回的
精确目录并交给系统文件管理器；目录不存在时明确报错，不扫描共享仓库或猜测旧布局。

迁移页不提供常驻的“严格/软”策略控件。`migrate check` 总是先要求保留全部源包；严格图
无解时，CLI 在 stderr 发出 schema 2 confirmation interaction 并暂停读取 stdin，GUI 将
冲突和“搜索 Pareto 极小删包方案”动作渲染为模态确认，再把 choice id 写回同一子进程。
软解按约束图的独立因子用通用 resolution interaction 逐组选择，随后只对合并后的删包赋值
枚举版本方案；版本仍有多个互不支配解时再发出一次通用 resolution interaction。审阅结果确有删除时，GUI
给随后的 export 增加 `--allow-removals`，表示复用用户刚刚作出的许可，避免重复询问；GUI
不自行计算删除集合或选择方案。

Runtime 页也把整合包作为领域动作呈现：安装 Orbit ZIP/TOML 或 Modrinth mrpack 时调用
`orbit import`，用户明确确认覆盖后再调用 `orbit fix` 求解并展示准确方案；导出分别调用
`orbit export --format zip` 与 `orbit export --format mrpack`。GUI 不解析归档、不改写清单，
也不根据扩展名实现第二套导入规则。

普通 Orbit 导出、迁移源快照与 Launcher 状态包都显示 CLI 的真实字节进度并可取消。JAR 已是压缩容器，core
以 ZIP Stored 写入；配置和小型元数据才使用 Deflate。被取消或失败的导出不会被当作可用包，
下一次对同一路径导出会先清理精确的事务临时文件。

### Mods

Mods 页以 lock 中逻辑包为单位显示当前版本、TOML 版本策略、环境、依赖、contained 模块数量和
多远端。contained 内容仍由 CLI 的结构化 lock 事实提供，但界面只显示总数，不展开内置模块名或
物理 JAR 明细；方案选择额外显示将安装的顶层 JAR basename，以区分同版本不同内容候选，
但不显示路径或 contained JAR 名。版本候选只显示 provider、依赖约束数量与内置模块数量，
绝不展示 hash、provider project/file id 或一整行内部模块约束。TOML 的 `[packages]` 是完整包集合，界面不制造根/传递两类身份。首次
接管由已安装 Launcher lock 的精确 Minecraft/Loader 版本调用 `orbit init`，不在 GUI 中
重复探测；`init` 在文本和 JSON 模式下执行同一个全局实例注册事务。搜索、添加、sync、fix、
install、outdated、单包/全部 upgrade、环境与远端管理都调用
现有 Orbit 命令。添加表单只暴露可选环境过滤和 optional，默认使用 `add` 的通用候选策略并
让同一进程展示互不支配方案；它不提供原始 `--version` 文本框。需要进一步限定版本时，包管理面板调用
`orbit versions` 展示全部远端 JAR 候选。版本策略不是命令行文本框：用户先选择“任意版本”、
“单边界”或“版本区间”，再从真实候选列表选择边界；单边界提供等于、大于、大于等于、
小于和小于等于，区间分别控制上下界是否包含。包管理面板必须把数字版本、完整字符串过滤和
包设置分成三个独立工作区；字符串操作表不得插入数字边界控件与候选版本列表之间。数字与字符串
工作区共享一个固定的策略摘要和应用动作，因此仍会原子提交同一条策略。字符串工作区选择
初始全部/空集合，每行选择交或并、可对该原子取反，并选择空、存在、精确、包含、开头或
结尾条件；另可插入对当前整体取补的步骤。行支持上移、下移、编辑和删除。大小写属于字符串
值，候选 JAR 的完整声明版本和其中的文本片段作为快捷选项，同时允许输入其它原始字符串。GUI 解析并序列化
CLI 的同一条顺序规则，但不执行匹配或求解。GUI 只把这些动作映射为结构化
`orbit constraint set` 参数，Loader 约束编码、Pareto 极小求解、多方案选择、确认和原子提交
全部由 CLI/core 完成。面板同时区分普通 remove 与 purge：purge 直接启动同一个 Orbit
进程，由 `data_deletion` interaction 展示运行时观测到的准确文件/目录树及逻辑包；GUI 不先
弹一层泛化确认，也不显示配置名称猜测。目录树条目还要列出其中将保留的其它当前所有者
节点。用户确认后仍由 Orbit 先清理 JAR/TOML/lock，再按递归所有权计划删除数据。
GUI 不复制 CLI 的项目详情报告；Discover 只提供搜索所需的名称、摘要、
来源、兼容标签和直接添加动作，需要完整项目数据时使用 `orbit info`。

Mods 页把三个职责分别呈现：Sync 只重新探测并重建本地事实，Fix 才求解和修复包集合，
Install 只按 lock 精确恢复缺失文件。GUI 不根据一个命令失败去偷偷调用另一个命令。

包管理等长表单弹窗使用固定标题、工作区导航、独立的正文滚动容器和固定策略摘要，不能依赖
页面滚动、制造嵌套滚动区或把超出窗口的控件直接裁掉。Mods 列表只保留包 ID、版本和一行本地化
摘要；管理是主动作，升级和移除使用带提示的紧凑图标动作。折叠活动条只允许单行截断摘要；
完整错误和多行日志只在活动抽屉中显示。

`outdated` 的更新与诊断分开呈现：可升级项显示当前到目标的变化，受阻候选显示 solver
保留的事实。`upgrade` 的多个 Pareto 极大方案、降级、替换和将删除的包在统一 interaction
窗口选择和确认，不由 GUI 自行筛掉。

### 其他页面

- Discover：展示 provider、名称、摘要和兼容标签，并提供直接添加；搜索任务在页面内明确区分
  尚未搜索、查询中、零结果和失败，失败不得伪装为空目录；
  provider 图标 URL 由 GUI 的原生 HTTP client 加载并保留本地图标占位；版本标签只显示 CLI
  返回的人类版本号，不解释 provider 的 opaque ID；
- Compatibility：schema 5 readiness、coverage、warning 和按风险排序的证据摘要；可按模组与
  风险阈值筛选，并通过 `audit --report` 导出完整 JSON；`fail-on-risk` 是 CI 退出码策略，
  不在桌面界面伪装为风险筛选；
- Accounts：侧边栏底部始终显示当前实例账户或全局默认账户，主账户页显示 Launcher 从皮肤
  脸部底层与帽子层合成后由 CLI 提供的 `avatar_path`（无皮肤时使用本地首字母占位）；GUI
  不把完整皮肤材质裁剪成头像；先选择 Microsoft、Offline 或标准 External Yggdrasil，再进入对应登录任务；Microsoft 设备授权返回后立即通过系统默认浏览器打开经过校验的 HTTPS 登录页，弹窗同时明确显示可点击链接、验证码、复制操作和上游说明，自动打开失败也不丢失继续登录所需信息；
  主页面只展示身份卡、全局默认和当前实例选择；External Yggdrasil 在添加账户时选择端点，
  端点的选择/新增/移除是独立步骤，确认端点后才进入单独的凭据表单；Settings 不承载账户
  认证端点；新增端点接受站点地址或精确 API root，由 CLI 完成 ALI 服务发现和 metadata
  验证，GUI 不自行拼接认证路径；
  账户列表加载失败保留上一次成功数据，并显示可重试错误，不能渲染成“没有账户”；会话被
  明确撤销时显示“登录已失效”，Microsoft 重新进入设备授权，Yggdrasil 回到原端点登录页；
- Server：EULA 完整正文及 digest 接受、启动/停止/状态/控制台命令；启动走
  `orbit launch --server`，停止、状态和命令仍直接走 Launcher；
- Activity：真实阶段、动态完成量、结构化错误和取消；底层 subcommand、PID 与未结构化 stderr
  不渲染成终端日志，协议错误作为产品错误明确呈现。
- Settings：使用原生设置分组、字段说明、枚举选择、路径选择和秘密输入，不把配置 key/value
  表格直接暴露成“带窗口的 CLI”。GUI 偏好只由 GUI 保存；Launcher 与 Orbit 的业务配置分别通过
  `orbit-launcher config ...` / `orbit config ...` 读取、设置和恢复默认值。没有安装 Orbit 时
  明确禁用 Orbit 配置区，不读取其 TOML 猜值。JAR cache 清理由明确危险确认后调用
  `orbit cache clean`，不由 GUI 删除缓存文件。

## CLI 能力映射约束

GUI 只集成有稳定桌面领域语义的命令，不能按“每个 subcommand 一个按钮”机械展开：

| 领域 | GUI 中的 CLI 能力 | 有意不重复的接口 |
| --- | --- | --- |
| 模组 | init、list、search/add（GUI 用默认勾选项决定是否把推荐的完整 `--string` 约束传给 add，并提供 env/optional）、versions、constraint、env、enable/disable、remote、remove/purge、sync、fix、install、outdated/upgrade、import/export、migrate check/export、audit、cache、instances register 与 config | CLI 的其它原始 add version 字符串由安装后的可视化版本策略编辑器替代；`info` 的长文本详情由 Discover 摘要替代；Orbit 的实例注册由 init/迁移自动同步，不再提供一套与 Launcher 并列的注册表页面；install 的 group/target 策略要等 TOML group 编辑器提供完整模型后再加入 |
| 运行时 | install/new、instance list/show/import/rename/remove/default、Loader configure/install、Minecraft/Loader/Java catalogs、Java 管理、Minecraft directory/move；客户端/服务端启动通过 `orbit launch [--server]` 单向调用 Launcher | 未安装的 `instance create` 中间态、launch/server dry-run、前台 server run 与隐藏 supervisor 属于 CLI/自动化接口；未同时安装 Orbit 与 Launcher 时数据感知启动明确禁用 |
| 账户与服务端 | login/list/refresh/select/clear/logout、Yggdrasil provider、EULA、start/stop/status/command | account show 已由账户卡片覆盖；秘密、EULA 与 token 不由 GUI 另存 |
| 配置与审计 | 两套 typed config list/set/unset、audit min-risk/mod/report | config path/get 已包含在设置模型中；audit fail-on-risk 只用于 CI 退出码 |

这张表是边界约束：新增 CLI 能力时必须决定其桌面领域交互、复用现有入口或明确保持 CLI-only，
不得悄悄形成第二条业务实现。

安装进度区分“下载/组装 Mojang Java 逐文件清单”和真正的归档解压：普通 assets、libraries
和 Java 文件不显示为解压；只有启动时重建 native 目录以及 Forge/NeoForge 官方 installer
内部处理属于必要的提取步骤。

Activity 折叠条始终保留当前任务、当前阶段、完成量和紧凑进度条；展开态只增加领域任务历史，
不显示底层 CLI 命令名、PID 或把 stderr 复制成终端；
不把同一进度信息重复成高大的纵向卡片。展开抽屉后，点击抽屉外区域或关闭按钮都会先播放
对称的退出过渡，再卸载抽屉；遮罩只负责界面层级和关闭交互，不伪造任何任务状态。

页面、安装向导步骤、模态框、活动抽屉和提示使用 GPUI 原生短过渡动画。动画只表达视图层级
变化；下载、解析、求解和物化进度仍只由 CLI 的强类型真实事件驱动，不按时间伪造完成量。

Yggdrasil 密码使用可清零内存容器，只经子进程 stdin 传递，不进入任务日志、参数或持久化
偏好。Microsoft token 仍只由 Launcher 的系统秘密存储负责。EULA 接受只提交用户刚查看的
官方正文 digest。

## HMCL 参考范围

界面参考 HMCL 的版本浏览层级：版本类型筛选、搜索、发布时间、推荐/稳定标签，以及先选
Minecraft 再显示兼容组件。实现未复制 HMCL 代码、图标或资源，也不包含 LittleSkin OAuth、
联机等 Orbit 边界外能力。所有兼容判断仍来自 Launcher 自己的官方 metadata adapter。

## 开发验证

```text
cargo test -p orbit-gui
cargo check --workspace --all-targets
cargo run -p orbit-gui
```

发布安装包必须把三个可执行文件放在同一目录。Windows 和 Linux 构建分别使用原生目标，
不把 Web 资源或浏览器运行时打入发布物。
