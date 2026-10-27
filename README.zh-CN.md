<p align="center">
  <img src="assets/orbit.svg" width="96" height="96" alt="Orbit 图标">
</p>

# Orbit

[English](README.md) · [下载](https://github.com/water2004/orbit/releases) · [文档](docs/orbit-cli-commands.md)

Orbit 管理已有 Minecraft 游戏目录中的模组，不要求更换启动器。Orbit Launcher 也可以创建和
运行 Minecraft 实例；Orbit GUI 则将两者放进同一个桌面界面。两个命令行工具也可以独立使用。

![Orbit 实例库](assets/screenshots/instance-library.png)

![Minecraft 与加载器选择](assets/screenshots/create-instance.png)

截图使用演示数据。界面也支持英文和跟随系统语言。

## 可以做什么

Orbit 从 JAR 元数据读取模组身份和依赖，可以从 Modrinth 和 CurseForge 查找模组、管理版本
规则、检查更新、修复依赖，并恢复锁定的文件。兼容性审计会报告可能的字节码冲突，但不能保证
整合包一定能正常运行。

Launcher 可以安装 Minecraft 和受支持的 Fabric、Quilt、Forge 或 NeoForge，管理 Java 与账户，
启动客户端或监督运行独立服务端。它还可以预览其他启动器中受支持的实例，并复制到新的隔离实例；
原实例不会被修改。支持的目录布局和限制见[已有实例导入说明](docs/orbit-launcher-import.md)。

桌面界面提供实例库、模组发现、安装、账户、版本规则、整合包导入导出和迁移，支持 Orbit
组合包和 Modrinth `.mrpack` 文件。

## 安装

从 [GitHub Releases](https://github.com/water2004/orbit/releases) 下载发布包。

- **Windows x64：** 未签名 MSI 提供仅 Orbit、Orbit + Launcher、完整套件三个安装选项；
  图形界面需要 WebView2。
- **Debian/Ubuntu amd64：** 按需安装独立的 `orbit`、`orbit-launcher` 和 `orbit-gui` deb。
  图形界面需要同版本的两个 CLI 和 WebKitGTK 4.1。

详细步骤见 [Windows](docs/windows-msi.md) 和 [Linux](docs/linux-deb.md) 安装文档。
图形界面的“设置 → 关于”可以手动检查 GitHub 正式版并打开发布页面，不会自动安装更新。

## 快速开始

在已有的受支持游戏目录中管理模组：

```sh
cd /path/to/your/game-directory
orbit init survival
orbit sync
orbit add sodium
```

`orbit sync` 对账本地文件，`orbit fix` 求解依赖问题，`orbit install` 按 lock 精确恢复文件。
`orbit outdated` 查看可用更新，`orbit upgrade` 在确认后应用。更多用法见
[Orbit 命令文档](docs/orbit-cli-commands.md)。

使用 Launcher 创建隔离客户端：

```sh
orbit-launcher install --new survival --kind client \
  --minecraft latest-release --loader fabric --loader-version stable
```

也可以使用图形界面的安装向导。已有实例导入和服务端设置见
[Launcher 命令文档](docs/orbit-launcher-cli.md)。

使用 CurseForge 需要自己的 API key。可以运行
`orbit config set auth.curseforge-api-key YOUR_API_KEY`，或设置环境变量
`ORBIT_CURSEFORGE_API_KEY`。

## 文档与开发

- [Orbit 命令](docs/orbit-cli-commands.md) · [Launcher 命令](docs/orbit-launcher-cli.md)
- [TOML 规范](docs/orbit-toml-spec.md) · [版本规则](docs/orbit-versions.md)
- [组合包格式](docs/orbit-bundle-format.md) · [兼容性审计](docs/orbit-bytecode-audit.md)
- [GUI 架构与构建](docs/orbit-gui.md) · [开发指南](docs/development.md)

## 许可证

Orbit 和 Orbit Launcher 使用 MIT。GUI 因改造 Modrinth App 组件而使用 GPL-3.0-only；
参见 [GUI 许可证](orbit-gui/ui/LICENSE)、[上游来源](orbit-gui/ui/UPSTREAM.md) 和
[截图说明](assets/screenshots/README.md)。
