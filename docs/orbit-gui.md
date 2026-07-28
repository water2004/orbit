# Orbit GUI

`orbit-gui` 是 Orbit 套件的跨平台 Rust 原生桌面前端。它不链接 `orbit-core` 或
`orbit-launcher-core`，也不实现下载、求解、安装、账户或启动业务；唯一边界是启动安装在
同一目录的 `orbit` / `orbit-launcher` 进程，并使用两者已有的 JSON/NDJSON/stdin 协议。

## 原生技术边界

界面使用 `eframe/egui`。默认 renderer 是 `wgpu`：Windows 通常映射到 D3D12，Linux 映射
到 Vulkan；这是原生 GPU 渲染后端，不是 WebView、浏览器或 HTML/JavaScript 运行时。
Linux 同时启用 Wayland/X11 窗口后端。图片由原生客户端读取 CLI 返回的展示 URL；URL
从不参与包身份、版本或依赖判断。

GUI 默认只接受与自身相邻的 `orbit(.exe)` 和 `orbit-launcher(.exe)`，也允许用户在设置页
明确选择准确路径。它不扫描 `PATH`，不链接 core，不在 CLI 失败后改走文件直读或兼容 API。
Windows MSI 的完整安装档位与 Linux deb 将三个程序安装在同一目录；Windows 的完整档位
同时创建开始菜单入口，Linux 安装 desktop entry 与 scalable 图标。Windows MSI 也允许只
安装 Orbit，或安装 Orbit + Launcher；没有 GUI 时不会创建桌面入口。

界面语言、主题和强调色都是独立的展示策略。语言可选跟随系统（默认）、English 或简体中文；
GUI 将同一个显式 `--language system|en|zh-CN` 传给每次 CLI 调用，因此窗口文本、CLI 提示、
进度与结构化错误保持一致。中文模式从系统字体数据库选择微软雅黑、Noto Sans CJK SC、思源
黑体等真实 CJK 字体，不硬编码某个平台的字体路径；系统缺少可用中文字体时明确报错。偏好由
eframe 原生持久化。页面的信息架构、空状态和任务流不依赖某个主题或语言才能成立。

源码同样按这一边界分层：`app.rs` 只保留共享状态、任务调度、结果归并和命令动作；
`app/pages/` 分别实现 Home、Mods、Discover、Audit、Runtime、Accounts、Server、Settings
与 Activity；`process.rs`/`wire.rs` 是唯一进程协议入口；`theme.rs` 只处理共享展示 token 与控件，
`model.rs` 只承载稳定 view model。页面不能自行读取 TOML、lock、JAR 或 Launcher 存储。
文本与密码输入必须使用 `theme.rs` 的语义化表单组件；页面不得直接创建 `TextEdit` 或自行
决定高度、内边距和常用宽度。这样主题、焦点反馈、密码遮蔽和中英文提示始终走同一路径。

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
方案中共同动作只显示一次，`different: true` 的选项差异用 `◆` 与文字同时突出。物理 JAR
文件名和候选哈希不作为包名或方案标题展示。

## 交互模型

### Runtime

Runtime 页使用 Launcher 的官方只读目录，而不是自由输入后试错：

1. `versions minecraft` 返回 Mojang 完整版本清单、类型、时间和 latest 标记；
2. 选择精确 Minecraft 后，`versions loader` 返回 Fabric、Quilt、Forge 或 NeoForge 官方
   来源声明的兼容版本；
3. `versions java` 返回官方 version JSON 要求的 Java component/major；
4. `instance show` 同时给出 desired intent 与 installed lock 摘要，界面突出当前/目标差异；
5. 保存调用 `instance configure`，安装或更新调用同一个 `install` 事务。

Java 不单独猜版本。安装事务根据目标 Minecraft 自动下载并验证 Mojang managed runtime。
Runtime 页可列出、完整校验和清理未使用 Java；任一注册实例 lock 仍引用的 runtime 不能删除。

### Mods

Mods 页以 lock 中逻辑包为单位显示环境、根/传递关系、依赖、contained 模块和多远端。首次
接管由已安装 Launcher lock 的精确 Minecraft/Loader 版本调用 `orbit init`，不在 GUI 中
重复探测。搜索、添加、sync、install、outdated、单包/全部 upgrade、环境与远端管理都调用
现有 Orbit 命令。GUI 不复制 CLI 的项目详情报告；Discover 只提供搜索所需的名称、摘要、
来源、兼容标签和直接添加动作，需要完整项目数据时使用 `orbit info`。

`outdated` 的更新与诊断分开呈现：可升级项显示当前到目标的变化，受阻候选显示 solver
保留的事实。`upgrade` 的多个 Pareto 极大方案、降级、替换和将删除的包在统一 interaction
窗口选择和确认，不由 GUI 自行筛掉。

### 其他页面

- Discover：展示 provider、名称、摘要和兼容标签，并提供直接添加；搜索任务在页面内明确区分
  尚未搜索、查询中、零结果和失败，失败不得伪装为空目录；
- Compatibility：schema 5 readiness、coverage、warning 和按风险排序的证据摘要；
- Accounts：先选择 Microsoft、Offline 或标准 External Yggdrasil，再进入对应登录任务；
  主页面只展示身份卡、全局默认和当前实例选择；External Yggdrasil 在添加账户时选择端点，
  端点的选择/新增/移除是独立步骤，确认端点后才进入单独的凭据表单；Settings 不承载账户
  认证端点；新增端点接受站点地址或精确 API root，由 CLI 完成 ALI 服务发现和 metadata
  验证，GUI 不自行拼接认证路径；
- Server：EULA 完整正文及 digest 接受、启动/停止/状态/控制台命令；
- Activity：真实阶段、动态完成量、日志、结构化错误和取消。

Activity 折叠条始终保留当前任务、当前阶段、完成量和紧凑进度条；展开态只增加历史任务，
不把同一进度信息重复成高大的纵向卡片。

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
