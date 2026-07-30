# Linux deb

Orbit 的 Debian/Ubuntu 发布面向 `amd64`，由三个职责独立的软件包组成：

| 软件包 | 安装内容 | 适用场景 |
|---|---|---|
| `orbit` | `/usr/bin/orbit` | 现有客户端或服务端实例的模组包管理 |
| `orbit-launcher` | `/usr/bin/orbit-launcher` | Minecraft、Loader、Java、账户、客户端启动与服务端监督 |
| `orbit-gui` | `/usr/bin/orbit-gui`、desktop entry、scalable icon | 图形桌面；精确依赖同版本的前两个包 |

deb 没有 MSI 安装向导那种交互式功能树，因此不能用一个包让用户安装时勾选组件。拆包也让
无图形服务器只安装所需 CLI，不引入 GPUI、X11/Wayland 等图形运行依赖。服务器技术上可以
安装 `orbit-gui` 而不启动它，但没有图形会话时无法使用，也会浪费依赖和磁盘空间。

三个程序共用同一套件版本和 GitHub Release，但仍保持运行时隔离：Launcher 不调用 Orbit，
GUI 只通过两个 CLI 的机器协议工作。用户配置、账户秘密、JAR cache 和 Minecraft 实例位于
用户级 XDG 目录或用户指定目录，不属于 deb 管理的系统文件；卸载不会删除它们。

安装 GitHub Release 中的包：

```bash
# 只管理模组
sudo apt install ./orbit_0.2.0-1_amd64.deb

# 无图形服务端；需要模组管理时可同时传入 orbit deb
sudo apt install ./orbit-launcher_0.2.0-1_amd64.deb

# 桌面完整套件。GitHub Release 不是 apt 仓库，所以一次传入三个本地文件。
sudo apt install ./orbit_0.2.0-1_amd64.deb \
  ./orbit-launcher_0.2.0-1_amd64.deb \
  ./orbit-gui_0.2.0-1_amd64.deb
```

卸载：

```bash
sudo apt remove orbit-gui orbit-launcher orbit
```

## 本地构建

本地构建要求 Linux、稳定版 Rust、Python 3、`dpkg-deb`、GPUI 的 Wayland/X11 原生开发库
与固定版本 `cargo-deb 3.7.0`：

```bash
sudo apt-get install libclang-dev libgtk-3-dev libssl-dev \
  libxcb1-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libxkbcommon-dev libxkbcommon-x11-dev
cargo install cargo-deb --version 3.7.0 --locked
bash ./scripts/build-linux-deb.sh
```

脚本使用 `x86_64-unknown-linux-gnu` release target，先执行三个程序的锁定 Cargo 构建，
再运行两个 CLI 产物的 `--help`，并在打包前校验所有静态文档、桌面入口与图标资源存在，
最后分别读取三个 crate 的强类型 cargo-deb 元数据生成
三个包。`dpkg-deb` 会验证包名、版本、安装树，以及 GUI 对同版本两个 CLI 的精确依赖。
构建脚本向 `cargo deb --output` 传入每个产物的确定路径，避免指定 Rust target 后 cargo-deb
把默认输出放到 target triple 子目录而令工作流误判产物缺失。

已经存在当前三个 Linux release binary 时可执行：

```bash
bash ./scripts/build-linux-deb.sh --skip-cargo-build
```

Windows 本地不构建 deb，也不需要安装 WSL、Docker 或未经固定的交叉编译器。正式包由
tag release GitHub Actions 在 Linux runner 上原生构建。不能把 Windows 可执行文件重打包成
deb，也不提供包含三个程序的兼容聚合包。
