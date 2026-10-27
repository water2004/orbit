# Windows MSI

Orbit 的 Windows MSI 是 64 位、per-machine 套件安装包。安装向导提供三个互斥档位：

1. 仅安装 `orbit.exe` 及其 `orbit-runtime-agent.jar`；
2. 安装 `orbit.exe`、Runtime Agent 与 `orbit-launcher.exe`；
3. 完整安装三个程序，即额外安装 `orbit-gui.exe`（默认）。

所选程序安装到同一个 `%ProgramFiles%\Orbit\bin`。Agent 始终跟随 Orbit 组件安装，不属于
Launcher，也不单独暴露功能；只有 `orbit launch` 会注入它。完整档位还会在开始菜单创建 Orbit
桌面应用入口；GUI 使用相邻的两个 CLI，不扫描 `PATH`。安装选项可把该目录加入系统
`PATH`；卸载时会移除由 MSI 添加的 `PATH` 项。安装需要管理员权限。

项目图标的唯一设计源是 `assets/orbit.svg`：中心点与外环三分之二使用同一主色，剩余
三分之一由四段等长彩色短弧组成，弧段之间保留明确空隙。`orbit-gui` 的 Rust build
script 在 Windows 构建时从该 SVG 生成多尺寸 ICO 并嵌入 PE 资源，因此窗口品牌区、开始菜单、
任务栏和 `orbit-gui.exe` 本身使用同一标志。Vue 品牌区直接使用 SVG；Tauri 默认窗口图标的
PNG/ICO 位于 `orbit-gui/icons/`，是同一 SVG 的生成快照，修改标志时应同步重新生成。
WiX 的 Icon 表和开始菜单 Shortcut 显式引用构建生成的 ICO，不依赖 Shell 从目标 EXE 猜测。

完整 GUI 档位需要系统已安装 Microsoft Edge WebView2 Runtime。GUI 前端资源嵌入 EXE，
无需运行本地 Web 服务器。GUI 采用 GPL-3.0-only，Orbit/Launcher 仍为 MIT，安装携带完整许可和上游说明。

双击 MSI 会进入标准安装向导，依次提供欢迎页、套件许可说明页、安装目录选择、安装档位与
Windows 集成选项、安装确认、进度和完成页。“加入系统 PATH”默认勾选，但用户可以在
安装前取消。已安装后再次运行同一 MSI，可以修改安装档位和 PATH 集成、修复或卸载。
卸载采用三个互斥档位，默认第一档：

1. 删除程序文件和所有可重建缓存，保留配置与实例。
2. 再删除 Orbit、Launcher、GUI 的配置及已保存登录，保留实例。
3. 全部删除，包括实例、存档、模组、截图、托管 Java 和完整受管 Minecraft 目录中的残留文件。

缓存包括 JAR/版本数据库、下载工件/索引/临时下载、头像及 WebView 缓存，并消费持久化配置中
指定的缓存位置。前两档保留实例索引、共享已安装 Minecraft 资源及实例 lock 引用的托管 Java；
已安装运行时不是可丢弃缓存。第三档也覆盖注册在自定义位置的实例，以及配置/注册表中的
Minecraft 仓库，不通过目录名猜测扫描磁盘。其他用户的数据始终不在范围内。

完整向导显示三档单选；Windows 设置/控制面板触发的 basic UI 卸载也有原生三档选择。
卸载前再次列出准确路径并确认，第三档执行永久删除而不是注销或回收站操作。取消不删除数据；
程序升级、修复和安装档位变更不会进入清理。路径重定向、危险祖先路径、确认后配置/索引变化
或运行中实例均会拒绝清理。

清理由内嵌原生 `orbit-installer-actions` 实施，各自的配置与实例格式仍由所属 core 读取；
只读预览在内存中应用已发布 schema 的迁移，不写回。实例先持有同一生命周期锁并暂存，再
暂存包含它们的应用目录；卸载失败时先恢复父目录、再恢复实例。只有程序卸载成功提交后才
永久清除暂存副本。提交后的物理清理失败会明确警告并给出保留的恢复目录，不因部分删除
再发起不可能完整恢复的 MSI 回滚。MSI 参数只含路径与快照摘要，不携带 API key 或配置正文。

安装包含 GUI 时，在成功提交阶段调用 GUI 自己的 `--install-cli-bindings` 无窗口入口，
将两条 CLI 路径覆盖为新安装目录中相邻的 EXE。其他偏好保持不变；损坏配置明确失败，
不能为了修路径重置用户偏好。仅安装 CLI 不触碰 GUI 配置，安装器不解析 GUI JSON。
程序卸载同时移除安装器专用的 HKLM 路径/PATH 记录，包括旧版留下的 AppData 路径值；
这不是用户的业务配置，不受三档用户数据选择影响。

`/quiet` 等标准 Windows Installer 参数仍可用于无人值守部署。静默安装默认选择完整
档位并加入系统 PATH。`INSTALL_PROFILE` 接受 `orbit`、`launcher` 或 `complete`；
传递 `ADD_TO_PATH=0` 可以关闭 PATH 集成：

```powershell
msiexec.exe /i orbit-0.9.1-x86_64.msi /quiet INSTALL_PROFILE=launcher ADD_TO_PATH=0
```

静默卸载默认选择 `UNINSTALL_DATA_LEVEL=cache`，不弹出交互。可显式选择 `settings` 或
`all`；`all` 是删除存档等所有实例内容的明确授权。旧的 AppData 勾选参数不再存在：

```powershell
msiexec.exe /x orbit-0.9.1-x86_64.msi /quiet UNINSTALL_DATA_LEVEL=all
```

## 构建

构建机需要：

- Windows x64；
- Rust MSVC toolchain；
- Node.js 22+（构建 Vue 前端）；
- .NET SDK 6 或更高版本；
- JDK 21（用于编译 Java 17 基线的 Runtime Agent）；
- 首次恢复 Cargo 与 .NET 工具时可访问对应包源。

在仓库根目录运行：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\build-windows-msi.ps1
```

脚本先恢复锁定 npm 依赖并构建 Vue 前端，再执行锁定的 Cargo release 构建，然后恢复仓库
`.config/dotnet-tools.json` 中固定的 WiX 版本与同版本的 WixUI 扩展；原生清理 DLL 作为
MSI Binary 内嵌，不作为第四个用户程序安装。输出路径为：

```text
target\wix\orbit-<version>-x86_64.msi
```

仅在已经构建过三个相邻的 release 可执行文件及清理 DLL 时，可以跳过 Cargo：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\build-windows-msi.ps1 -SkipCargoBuild
```

MSI 默认不签名。正式发布前应在 CI 的受控签名步骤中使用项目证书签名，并保留
SHA-256 校验值。

正式 tag 构建由 [release-process.md](release-process.md) 中的 GitHub Actions 在
Windows runner 上调用同一脚本。只有 tag 指向 `main` 且版本匹配时才会与 Linux deb
一起发布；本地构建不创建 GitHub Release。

## WiX 版本与许可

仓库固定 WiX 7.0.0，并在构建命令中使用官方为构建脚本和 CI 提供的
`-acceptEula wix7` 显式接受方式。构建者仍需遵守 WiX 7 EULA 与 Open Source
Maintenance Fee 条款；不要移除或绕过这一明确的许可边界。

WiX 只在构建阶段运行，不会成为 MSI 的运行时依赖。

## 升级规则

`UpgradeCode` 和组件 GUID 是稳定标识，不得在普通版本升级时修改。每次 MSI 构建会
生成新的 `ProductCode`，并允许 major upgrade 的版本范围包含当前三段 Cargo 版本：
因此同为 `0.1.0` 的后续构建也能替换之前的构建，而不是因“已安装相同版本”直接退出。
再次运行完全相同的 MSI（相同 `ProductCode`）则进入 Windows Installer 维护模式，可
修改 PATH、修复或卸载。更低版本仍被拒绝。

WiX 的 ICE61 会对“升级范围包含相同版本”给出通用警告；构建脚本仅抑制这一项，因为
这里的包含关系是同版本重建可升级的明确产品要求，其余 MSI ICE 校验仍然执行。

若改变安装目录或拆分组件，应先核对 Windows Installer component rules，不能复用
与资源路径不再匹配的组件 GUID。
