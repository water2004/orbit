# Orbit Launcher 发布与安装包

`orbit-launcher` 从 0.2.0 起与 `orbit`、`orbit-gui` 使用同一个套件版本、`v*` tag 和 GitHub
Release。统一发布不改变运行时边界：Launcher 仍可独立安装，不链接、不调用 Orbit，也不管理
模组或运行时数据归属。需要归属记录时另装 Orbit，并通过 `orbit launch` 单向调用 Launcher；
Launcher 包本身不携带 Orbit Agent。

- Windows x64 使用套件 MSI 的“Orbit + Launcher”档位；不再维护独立 Launcher MSI。
- Debian/Ubuntu amd64 发布独立的 `orbit-launcher_<version>-1_amd64.deb`；它不依赖 Orbit 或
  GUI，是无图形服务端的推荐安装方式。
- 需要模组管理时另装 `orbit`；只有桌面环境才需要安装精确依赖两者的 `orbit-gui`。

完整构建、卸载和发布规则见 [Windows MSI](windows-msi.md)、[Linux deb](linux-deb.md) 与
[Release 流程](release-process.md)。仓库不保留旧 `launcher-v*` workflow、独立 MSI 脚本或
聚合 deb 兼容路径。
