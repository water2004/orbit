# Windows MSI

Orbit 的 Windows MSI 是 64 位、per-machine 安装包。它把 `orbit.exe` 安装到
`%ProgramFiles%\Orbit\bin`，并可按安装选项把该目录加入系统 `PATH`；卸载时会
移除由 MSI 添加的 `PATH` 项。安装需要管理员权限。

双击 MSI 会进入标准安装向导，依次提供欢迎页、MIT 许可页、安装目录选择、Windows
集成选项、安装确认、进度和完成页。“加入系统 PATH”默认勾选，但用户可以在安装前
取消。已安装后再次运行同一 MSI，可以修改 PATH 集成、修复或卸载。

`/quiet` 等标准 Windows Installer 参数仍可用于无人值守部署。静默安装默认加入
系统 PATH；传递 `ADD_TO_PATH=0` 可以关闭：

```powershell
msiexec.exe /i orbit-0.1.0-x86_64.msi /quiet ADD_TO_PATH=0
```

## 构建

构建机需要：

- Windows x64；
- Rust MSVC toolchain；
- .NET SDK 6 或更高版本；
- 首次恢复 Cargo 与 .NET 工具时可访问对应包源。

在仓库根目录运行：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\build-windows-msi.ps1
```

脚本执行锁定的 Cargo release 构建，然后恢复仓库
`.config/dotnet-tools.json` 中固定的 WiX 版本与同版本的 WixUI 扩展。输出路径为：

```text
target\wix\orbit-<version>-x86_64.msi
```

仅在已经构建过 `target\release\orbit.exe` 时，可以跳过 Cargo：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\build-windows-msi.ps1 -SkipCargoBuild
```

MSI 默认不签名。正式发布前应在 CI 的受控签名步骤中使用项目证书签名，并保留
SHA-256 校验值。

## WiX 版本与许可

仓库固定 WiX 7.0.0，并在构建命令中使用官方为构建脚本和 CI 提供的
`-acceptEula wix7` 显式接受方式。构建者仍需遵守 WiX 7 EULA 与 Open Source
Maintenance Fee 条款；不要移除或绕过这一明确的许可边界。

WiX 只在构建阶段运行，不会成为 MSI 的运行时依赖。

## 升级规则

`UpgradeCode` 和组件 GUID 是稳定标识，不得在普通版本升级时修改。构建脚本根据
Cargo 的三段数字版本生成稳定的 x64 `ProductCode`：同一版本重建仍是同一产品，
提高版本时才得到新产品代码。MSI 通过 major upgrade 替换旧版本并拒绝降级。

若改变安装目录或拆分组件，应先核对 Windows Installer component rules，不能复用
与资源路径不再匹配的组件 GUID。
