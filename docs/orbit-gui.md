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

### 实例与安装

实例库提供筛选、卡片/紧凑布局、直接启动和导入已有目录。
创建依次选择 Minecraft、兼容 Loader/Java、核对名称和位置。版本目录只来自
versions minecraft|loader|java，默认正式版，可切换快照或全部，不能自由猜测版本。

从整合包创建先用 package inspect 获取运行时及 optional 内容。客户端使用 Launcher 的
隔离实例目录；服务端选择父目录并展示新建目录。install --new 执行同一安装事务。
首次服务端安装通过 JSON 交互展示完整 EULA、官方 URL 和摘要，仅在明确同意后继续；
--non-interactive 仍拒绝尚未接受的协议，不因 GUI 自动授权。

运行时页提供打开目录、Loader 更新、校验修复、客户端分辨率、重命名、默认实例和取消注册。
取消注册保留磁盘数据。Java 页提供清单、校验和经确认的删除；仓库迁移调用 minecraft move。

### 模组和方案

内容页显示逻辑包、版本、启用状态和可行更新。发现页使用当前实例运行时搜索，添加采用
CLI 返回的 provider/project ID。默认 beta/snapshot 排除规则是可取消的勾选项，显式传给 add。

包管理分为数字版本、字符串过滤、文件与数据、包设置四个工作区：

- 数字边界或区间从真实候选选择，不用原始约束文本框。
- 字符串以有序条件行编辑交、并、逐项取反、整体取补及精确/包含/开头/结尾/空/存在，
  可控制大小写。GUI 仅解析和序列化，不执行匹配。
- 应用策略走 constraint set，由 CLI 对完整图求解、选择、确认并原子提交。
- 文件与数据按需调用 ownership，显示顶层 JAR、独占文件/目录树及保留节点。
  reset 保留包本体，purge 删除包及独占数据，均由 CLI 确认准确范围。
- 包设置管理环境和多远端，不能删除最后一个远端。内置模块只显示数量。

sync 如实对账，fix 求解修复，install 按 lock 恢复，三者不得隐式串联。
升级与受阻诊断分开呈现，GUI 不自行筛选 Pareto 解。

独立因子由同一个 CLI 依次询问，每步只选择当前因子，不能提前确认整体事务。
只有最终 confirmation 才显示应用按钮；共同操作只显示一次，差异用 ◆ 和文字标记。
显示包、版本变化及所选顶层 JAR 文件名，不显示哈希、内置清单或零值诊断字段计数。
提交期间禁止重复点击，后一个问题不能被前一次响应清空。取消任何一步停止后续编排。
模态框固定标题和底部操作、正文独立滚动，背景不可交互，按钮不能被长内容挤出窗口。

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
失败不清空已有账户；token 由 Launcher 管理，密码只经 stdin，不进入参数或任务记录。

设置按外观、Launcher、Java、Orbit 分组，使用选择、数值和路径控件，不暴露 key/value
命令表格。GUI 只保存自己的展示偏好；两套业务配置调用各自 config 命令。
审计保留有界风险投影，完整证据通过 audit --report 保存，不截断 JSON 后伪装成功。

## 展示与安全

支持 system（默认）/English/简体中文，系统/浅色/深色主题与强调色。
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
开发验证包含多步选择、最终确认、竞态、取消、协议错误和迁移顺序。
ui/tests/visual.html 为独立布局测试入口，不进入生产路由或打包。
原生窗口、真实登录及游戏的最终验收由用户执行，构建通过不代表全部平台端到端验证。
