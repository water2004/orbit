# Linux deb

Orbit 的 Debian 包面向 `amd64`，把 `orbit`、`orbit-launcher` 和 `orbit-gui` 一同安装到
`/usr/bin`，并安装原生桌面入口与 scalable 图标。三个可执行文件始终相邻，因此 GUI
不扫描 `PATH` 或猜测组件位置。README、GUI 边界文档与 MIT 许可信息位于
`/usr/share/doc/orbit/`。用户配置和 JAR cache 位于用户级 XDG 目录，不属于 deb 管理的
系统文件；卸载软件包不会删除用户数据或 Minecraft 实例。

安装 GitHub Release 中的包：

```bash
sudo apt install ./orbit_0.1.2-1_amd64.deb
orbit --help
orbit-launcher --help
orbit-gui
```

卸载：

```bash
sudo apt remove orbit
```

## 本地构建

本地构建要求 Linux、稳定版 Rust、Python 3、`dpkg-deb`、GPUI 的 Wayland/X11 原生开发库
与固定版本 `cargo-deb 3.7.0`：

```bash
sudo apt-get install libclang-dev libgtk-3-dev libssl-dev \
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev
cargo install cargo-deb --version 3.7.0 --locked
bash ./scripts/build-linux-deb.sh
```

脚本使用 `x86_64-unknown-linux-gnu` release target，先执行三个程序的锁定 Cargo 构建，
再运行两个 CLI 产物的 `--help`，最后由 cargo-deb 根据 `orbit-cli/Cargo.toml` 的强类型
包元数据生成 deb，并用 `dpkg-deb --info/--contents` 验证控制信息与安装树。

已经存在当前三个 Linux release binary 时可执行：

```bash
bash ./scripts/build-linux-deb.sh --skip-cargo-build
```

Windows 本地不构建 deb，也不需要安装 WSL、Docker 或未经固定的交叉编译器。正式包由
tag release GitHub Actions 在 Linux runner 上原生构建。
