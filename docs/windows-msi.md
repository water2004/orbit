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
任务栏和 `orbit-gui.exe` 本身使用同一标志；品牌区使用同一次构建生成的 PNG，WiX 的 Icon
表和开始菜单 Shortcut 显式引用生成的 ICO，不依赖 Shell 从目标 EXE 猜测。仓库不维护会
漂移的手工 PNG/ICO 源文件。

双击 MSI 会进入标准安装向导，依次提供欢迎页、MIT 许可页、安装目录选择、安装档位与
Windows 集成选项、安装确认、进度和完成页。“加入系统 PATH”默认勾选，但用户可以在
安装前取消。已安装后再次运行同一 MSI，可以修改安装档位和 PATH 集成、修复或卸载。
卸载向导还会提供“删除 Orbit AppData”复选框，默认不勾选。勾选后会递归删除安装时
记录的 Orbit 与 Orbit Launcher 默认 roaming/local AppData；这包含 GUI 偏好、账户元数据、
系统秘密存储中的本地凭据、实例注册表、managed Java 和缓存。自定义配置/缓存路径和
Minecraft 实例不属于 MSI 管理范围，始终不会删除。

`/quiet` 等标准 Windows Installer 参数仍可用于无人值守部署。静默安装默认选择完整
档位并加入系统 PATH。`INSTALL_PROFILE` 接受 `orbit`、`launcher` 或 `complete`；
传递 `ADD_TO_PATH=0` 可以关闭 PATH 集成：

```powershell
msiexec.exe /i orbit-0.4.1-x86_64.msi /quiet INSTALL_PROFILE=launcher ADD_TO_PATH=0
```

静默卸载默认保留 AppData。只有显式传递 `REMOVE_APPDATA=1` 才删除上述两个默认目录：

```powershell
msiexec.exe /x orbit-0.4.1-x86_64.msi /quiet REMOVE_APPDATA=1
```

## 构建

构建机需要：

- Windows x64；
- Rust MSVC toolchain；
- .NET SDK 6 或更高版本；
- JDK 21（用于编译 Java 17 基线的 Runtime Agent）；
- 首次恢复 Cargo 与 .NET 工具时可访问对应包源。

在仓库根目录运行：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\build-windows-msi.ps1
```

脚本执行锁定的 Cargo release 构建，然后恢复仓库
`.config/dotnet-tools.json` 中固定的 WiX 版本与同版本的 WixUI、Util 扩展。输出路径为：

```text
target\wix\orbit-<version>-x86_64.msi
```

仅在已经构建过三个相邻的 release 可执行文件时，可以跳过 Cargo：

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
