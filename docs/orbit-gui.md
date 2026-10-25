# Orbit GUI

Orbit GUI 使用 Vue 3 + Tauri 2，以 Modrinth App 的导航、实例库、内容页和基础控件为基础改造。
来源、上游提交及复制文件见 [UPSTREAM.md](../orbit-gui/ui/UPSTREAM.md)。不引入 Modrinth
的认证、下载、启动、托管服务、广告、分析或品牌资源。

## 单一业务边界

GUI 不链接 core，不读取业务 TOML、lock、JAR、账户存储或归属账本，不解析整合包。
只执行同目录或用户明确选择的 Orbit/Launcher 可执行文件，不扫描 PATH。
Rust 桥负责进程生命周期、协议验证、展示投影及系统集成；Vue 负责展示和既有命令的顺序编排。

- stdout：schema 2 success envelope；stderr：进度、交互、错误；stdin：同一进程的 interaction_response。
- 严格 UTF-8，拒绝旧 schema 与损坏编码。失败不得转走文件直读或兼容 API。
- 取消或关闭 GUI 终止仍受管理的子进程组/Windows Job。Windows 不弹控制台。
- Launcher 负责运行时/账户/Java；模组及数据管理归 Orbit。游戏启动调用 orbit launch。

## 信息架构

主导航为实例库、发现、创建实例、设置和账户。实例内部再分模组内容、运行时、兼容性；
服务端实例另外提供服务端控制。实例选择是明确上下文，不是命令参数表单。
异步结果绑定发起时的实例，旧请求不得覆盖新选择。
实例页面统一使用 `/instance/:id/...` 路由；进入时加载该实例，不能依赖上次的全局选择。
发现页只有 `/browse?instance=<id>` 一条路径，工具栏的实例下拉框明确安装目标。
实例选择与搜索放在同一紧凑工具栏；添加约束、隐藏已安装和高级选项放在下方选项行，
不再以独立大侧栏承载少量勾选项。创建组件页将 Loader 类型、版本和 Java 要求组织在同一张卡片，
控制类型按钮与版本选择器宽度，步骤操作紧随内容，不用空白撑到窗口底部。
从实例添加入口进入时预选该实例，从侧边栏进入同一页，可以直接切换目标；不再有独立的
选实例中间页或实例内的第二套发现路由。切换时重载目标事实，筛选、已安装状态与安装目标一起切换。
不存在先进行不带版本/Loader 的全局搜索、点击安装后才补选实例的路径。

### 实例与安装

实例库提供筛选、卡片/紧凑布局和直接启动。创建入口分为新建、从整合包安装、导入已有目录，
实例库的快捷入口复用此流程；导入目录不依赖联网获取 Minecraft 版本列表。
创建依次选择 Minecraft、兼容 Loader/Java、核对名称和位置。版本目录只来自
versions minecraft|loader|java，默认正式版，可切换快照或全部，不能自由猜测版本。
Minecraft、Loader、Java、目标实例与其他下拉选择共用有界、可搜索、可键盘操作的控件，
不使用不同浏览器/系统的原生 select 弹层；元数据加载和按钮忙碌状态显示加载动画，不伪造进度。

从整合包创建先用 package inspect 获取运行时及 optional 内容。客户端使用 Launcher 的
隔离实例目录；服务端选择父目录并展示新建目录。install --new 执行同一安装事务。
首次服务端安装通过 JSON 交互展示完整 EULA、官方 URL 和摘要，仅在明确同意后继续；
--non-interactive 仍拒绝尚未接受的协议，不因 GUI 自动授权。

运行时页提供打开目录、Loader 更新、校验修复、客户端分辨率、重命名、默认实例和取消注册。
取消注册保留磁盘数据。Java 页提供清单、校验和经确认的删除；仓库迁移调用 minecraft move。
实例清单由 Launcher 返回游戏版本、Loader 和 `icon_path`；图标由 `instance icon set|clear`
校验、缩放和持久化，GUI 不读写实例 TOML。默认图标和 Loader 小图标复用许可允许的
Modrinth 资源并保留出处。自定义图标随 Launcher 状态导出，恢复时与实例配置在同一事务中提交。

### 模组和方案

内容页显示逻辑包、版本、启用状态和可行更新。发现页使用当前实例运行时搜索，添加采用
CLI 返回的 provider/project ID。默认 beta/snapshot 排除规则是可取消的勾选项，显式传给 add。
搜索词、结果和添加选项按实例保留；运行时版本或 Loader 变化时废弃旧兼容结果。
添加在结果卡片上显示进行中状态，CLI 完成并刷新清单后改为管理入口，不重复弹出参数表单。
已安装判定只匹配 CLI 返回的远端身份，不猜测 slug/mod_id。启用状态使用开关，
导入、导出和维护收进内容工具菜单；维护按同步磁盘、修复依赖、恢复锁定文件解释用途。

包管理默认显示概览，按需进入版本规则、字符串过滤、文件与数据、包设置：

- 进入版本规则或字符串过滤才请求候选清单；打开概览或文件页不触发远端版本下载。
  数字边界或区间从真实候选选择，不用原始约束文本框。
- 字符串以有序条件行编辑交、并、逐项取反、整体取补及精确/包含/开头/结尾/空/存在，
  可控制大小写。GUI 仅解析和序列化，不执行匹配。
- 应用策略走 constraint set，由 CLI 对完整图求解、选择、确认并原子提交。
- 文件与数据按需调用 ownership，显示顶层 JAR、独占文件/目录树及保留节点。
  reset 保留包本体，purge 删除包及独占数据，均由 CLI 确认准确范围。
- 包设置管理环境和多远端，不能删除最后一个远端。内置模块只显示数量。

sync 如实对账，fix 求解修复，install 按 lock 恢复，三者不得隐式串联。
升级与受阻诊断分开呈现，GUI 不自行筛选 Pareto 解。
单包升级仍调用 `upgrade <mod_id>`：core 先最大化目标版本，再最小化其余包变动；
全部升级调用无包参数的 `upgrade`，保持全局版本 Pareto 极大。两者不能混用。
弹窗发起的任务在原弹窗内显示真实进度、失败信息和取消入口。任务在创建时显式绑定展示作用域，
不从全局“最近一个任务”猜测归属；导入/导出执行期间保留原弹窗，完成后才关闭。
任务终态与进度快照分离：收到 finished 后清除运行中消息和计数，忽略迟到的进度事件。
升级直接展示同一次 CLI 返回的 changes、diagnostics 和 warnings：无变更明确显示当前约束下无可行
更高版本，提供实际受阻原因；不能丢掉结果后继续显示“正在选择”。活动历史复用同一任务结果控件。
历史只保存有界提示投影（最多 16 项，额外条数明确标记），不缓存整份命令 JSON；GUI 不追加第二次升级检查。

独立因子由同一个 CLI 依次询问，每步只选择当前因子，不能提前确认整体事务。
只有最终 confirmation 才显示应用按钮；共同操作只显示一次，差异用 ◆ 和文字标记。
显示包、版本变化及所选顶层 JAR 文件名，不显示哈希、内置清单或零值诊断字段计数。
提交期间禁止重复点击，后一个问题不能被前一次响应清空。取消任何一步停止后续编排。
模态框固定标题和底部操作、正文独立滚动，背景不可交互，按钮不能被长内容挤出窗口。
普通弹窗默认提供右上角关闭按钮和 Escape；查看信息不要求应用更改才能退出。
CLI 交互仅在 `allow_cancel` 时允许关闭，关闭通过原进程回传取消，不能只隐藏等待中的问题。
应用策略按钮仅显示在规则编辑页，其他工作区底部为关闭。

### 整合包与迁移

普通导入使用 orbit import，不暗中修复。optional 文件由 Launcher 检查后供用户选择。
导出支持 Orbit 自有包和 mrpack，选择仅模组或包含所属数据；Orbit 包可通过 Launcher
export --base 追加世界与运行时状态。

迁移先 orbit export，再 Launcher export --base；源导出成功后才 install --new --from
创建目标。随后 migrate check --source-pack 提供审阅，继续后 migrate export，最后目标
orbit install。源实例不原地修改，取消或失败不继续下一步，快照路径保留供恢复。
严格保包无解时由 CLI 询问是否搜索最少删包方案，不提供常驻“软/硬策略”控件。

### 账户、设置与审计

账户展示 CLI 提供的合成头像、默认账户与会话状态。先选择 Microsoft、Yggdrasil 或离线，
端点管理与凭据输入是独立弹窗。Microsoft 打开 HTTPS 设备登录页，并保留验证码和网址。
皮肤头像和模组图片保留透明区域，有图时不垫占位渐变；皮肤不裁掉帽子层的方形边角。
占位底色只用于无图/加载失败时的首字母占位。
失败不清空已有账户；token 由 Launcher 管理，密码只经 stdin，不进入参数或任务记录。
标题栏可快速切换账户：客户端实例页设置当前实例账户，其他页面设置全局默认账户；
账户管理仍独立存在，切换只调用 Launcher `account select` 并刷新同一 CLI 数据。

设置按外观、Launcher、Java、Orbit 分组，使用选择、数值和路径控件，不暴露 key/value
命令表格。GUI 只保存自己的展示偏好；两套业务配置调用各自 config 命令。
配置合法枚举值由各自 CLI 的 `allowed_values` 返回，不能把 Launcher 的
`auto/always/never` 当成 Orbit 的 `modern/plain/off`。Orbit `config list` 的
`resolved_paths` 提供实际缓存/版本库位置；未设置覆盖时展示默认位置，而不是空白。
凭据不回显但展示是否已配置；空值不自动写回。GUI 不自行推测系统目录或默默接受缺少定义的旧 CLI。
Java 选择器提供自动、本地检测结果、Mojang 托管、文件选择；全局设置走 `config set java.selection`，
实例通过 `instance java set|clear` 覆盖或继承。系统 Java 不出现删除/托管 inventory 校验按钮。
Java 兼容判断、检测、下载与启动选择全部在 Launcher core，GUI 不扫描 PATH 或猜测版本。
审计保留有界风险投影，完整证据通过 audit --report 保存，不截断 JSON 后伪装成功。

## 展示与安全

支持 system（默认）/English/简体中文，系统/浅色/深色主题与强调色。
窗口使用无系统标题栏布局，应用顶栏可拖动、双击最大化，并保留最小化/最大化/还原/关闭。
这些操作只使用受限 Tauri window capability；关闭窗口仍执行原有子进程清理。
CLI 每次接收同一语言选择。字体、触控板惯性由系统 WebView 处理；弹窗禁止滚动链传递。
短过渡尊重减少动画设置，进度只消费 CLI 真实事件。活动条保留单行摘要、进度和取消，
抽屉展示有界历史及完整消息，点击外部可收起。

远端图片仅使用 CLI 返回的 HTTPS URL，本地图片仅为 CLI 返回的 icon/avatar 路径开放
asset scope；GUI 不提取 JAR 图标或裁剪皮肤。系统打开目录仅允许 CLI 返回的准确目录，
不授予前端任意执行文件能力。生产 CSP 禁止远端脚本、iframe 和对象嵌入。

## 代码与构建

- ui/src/pages：领域页面；components：控件、模态框、策略编辑与任务交互。
- ui/src/components/upstream：裁剪适配后的 Modrinth 控件，保留来源与许可。
- ui/src/services：任务、实例上下文、安装与迁移编排。
- src/desktop.rs：Tauri 桥和资源作用域；process.rs：唯一进程桥；wire.rs：严格协议及展示投影。
- src/string_rule.rs：原有规则的展示解析器；验证和匹配仍在 CLI/core。

开发需要 Node.js 22+、Rust，以及 Tauri 系统依赖，先构建前端再编译 Rust：

```sh
npm --prefix orbit-gui/ui ci
npm --prefix orbit-gui/ui test
npm --prefix orbit-gui/ui run build
cargo test -p orbit-gui -p orbit-launcher
cargo build -p orbit-gui
```

Windows 使用 WebView2 Runtime，Linux 使用 WebKitGTK 4.1/GTK 3。前端产物嵌入可执行文件，
无需本地 Web 服务器。发布脚本和 CI 已加入前端构建，Linux 保留三个 deb，Windows 保留三档安装。
`custom-protocol` 默认启用，保证直接 `cargo build` 也嵌入前端，而不是依赖开发服务器。

GUI 为 GPL-3.0-only，安装包携带许可证与 Modrinth 来源说明；Orbit/Launcher 保留原有许可。
仅复用许可允许的通用资源，不使用上游 COPYING.md 禁止的 Modrinth 品牌资产。
开发验证包含多步选择、最终确认、竞态、取消、协议错误和迁移顺序。
ui/tests/visual.html 验证多步选择弹窗；ui/tests/workflows.html 使用模拟 IPC 驱动真实页面，
验证发现、实例内容、管理与创建入口布局。这些入口不执行真实 CLI，不读取用户数据，
不进入生产路由或打包。组件测试覆盖默认关闭、取消回传、实例上下文、安装防重和按需加载。
原生窗口、真实登录及游戏的最终验收由用户执行，构建通过不代表全部平台端到端验证。
