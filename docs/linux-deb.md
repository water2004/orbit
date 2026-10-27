# Linux deb

Orbit 的 Debian/Ubuntu 发布面向 `amd64`，由三个职责独立的软件包组成：

| 软件包 | 安装内容 | 适用场景 |
|---|---|---|
| `orbit` | `/usr/bin/orbit`、`/usr/lib/orbit/orbit-runtime-agent.jar` | 现有客户端或服务端实例的模组包管理及数据感知联合启动 |
| `orbit-launcher` | `/usr/bin/orbit-launcher` | Minecraft、Loader、Java、账户、客户端启动与服务端监督 |
| `orbit-gui` | `/usr/bin/orbit-gui`、desktop entry、scalable icon | 图形桌面；精确依赖同版本的前两个包 |

两个 CLI 包各带 `/usr/lib/<包名>/orbit-maintenance`，经 `update-alternatives` 提供唯一的
`/usr/bin/orbit-maintenance`；只装其中一个包也可清理，不额外引入第四个包或 GUI 依赖。
该宿主只服务打包数据生命周期，不让 Launcher 与 Orbit 在运行时互相依赖。

deb 没有 MSI 安装向导那种交互式功能树，因此不能用一个包让用户安装时勾选组件。拆包也让
无图形服务器只安装所需 CLI，不引入 WebKitGTK、GTK 等图形运行依赖。服务器技术上可以
安装 `orbit-gui` 而不启动它，但没有图形会话时无法使用，也会浪费依赖和磁盘空间。

三个程序共用同一套件版本和 GitHub Release，但仍保持运行时隔离：Launcher 不调用 Orbit；
需要包级运行时归属时由 Orbit 单向调用 Launcher。GUI 只通过两个 CLI 的机器协议工作，启动
入口走 Orbit，其余 Launcher 领域操作直接走 Launcher。用户配置、账户秘密、JAR cache 和 Minecraft 实例位于
用户级 XDG 目录或用户指定目录，不属于 deb 管理的系统文件。apt/dpkg 的 remove/purge
均不擅自删除这些文件，也不会遍历 `/home` 或猜测当前桌面用户。符合 Debian 的
[maintainer script 生命周期](https://www.debian.org/doc/debian-policy/ch-maintainerscripts.html)。

安装 GitHub Release 中的包：

```bash
# 只管理模组
sudo apt install ./orbit_0.9.1-1_amd64.deb

# 无图形服务端；需要模组管理时可同时传入 orbit deb
sudo apt install ./orbit-launcher_0.9.1-1_amd64.deb

# 桌面完整套件。GitHub Release 不是 apt 仓库，所以一次传入三个本地文件。
sudo apt install ./orbit_0.9.1-1_amd64.deb \
  ./orbit-launcher_0.9.1-1_amd64.deb \
  ./orbit-gui_0.9.1-1_amd64.deb
```

## 用户数据清理与卸载

先关闭 GUI 和所有游戏/服务端，再以数据所属用户执行（不要 sudo）：

```bash
orbit-maintenance
```

交互默认第一档，展示精确路径后还需输入 `yes` 确认。三档与 MSI 共用规划与暂存/恢复逻辑：

| `--level` | 删除内容 | 保留内容 |
|---|---|---|
| `cache` | 所有可丢弃缓存，包括自定义 JAR 缓存/版本库、头像和 WebView 缓存 | 配置、账户、实例注册表、整个实例、已安装 Java/Minecraft |
| `settings` | 配置、账户和缓存 | 实例注册表、整个实例、已安装 Java/Minecraft |
| `all` | 以上全部以及受管 Minecraft 目录、已注册实例和存档 | 不属于本次确认范围的数据 |

原生 XDG 配置/数据/缓存位置彼此分离；不会仅检查 `~/.cache` 而漏掉持久层中的下载缓存。
配置的自定义 Minecraft 目录和独立服务端实例参与预览；不扫描磁盘猜测其它实例。
选择 `all` 会永久删除受管目录中的未注册残留实例，而不是仅从列表注销。

可先查看路径，或在明确授权的自动化中使用：

```bash
orbit-maintenance --level all --dry-run --output-format json
orbit-maintenance --level settings --yes
```

未给 `--level` 时 `--yes` 不替用户选择档位，stdin 不可用就失败。工具拒绝 sudo/pkexec
上下文，不能从 root 的 HOME 推测其它用户。多个用户应各自清理自己的数据。
成功后再删除程序文件：

```bash
sudo apt remove orbit-gui orbit-launcher orbit
```

卸载前暂存失败会恢复原文件。工具写出恢复计划的路径；被中断时可以用
`orbit-maintenance --restore /明确路径/orbit-cleanup-….json` 还原仍保留的暂存文件。
计划只含路径/摘要，不含账户 token；已开始永久删除的数据不能承诺恢复。
`--restore` 与清理一样先预览确认。成功后恢复计划会清除，空的维护目录不包含游戏数据。
若已卸载最后一个包，可临时重新安装任一 CLI 包获得这个入口。

## GUI 安装目录绑定

DEB 不在安装脚本里代替用户读写 GUI JSON，也不启动图形程序。在当前用户首次打开新安装/
升级后的 GUI 时，GUI 自己检测安装指纹并把 CLI 路径更新成其真实相邻可执行文件，保留
主题、语言和选中实例。随后的正常启动不会覆盖用户手动选择的 CLI。

## 本地构建

本地构建要求 Linux、稳定版 Rust、Node.js 22+、JDK 21、Python 3、`dpkg-deb`、WebKitGTK 4.1/GTK 3 开发库
与固定版本 `cargo-deb 3.7.0`：

```bash
sudo apt-get install libclang-dev libgtk-3-dev libssl-dev \
  libwebkit2gtk-4.1-dev librsvg2-dev
cargo install cargo-deb --version 3.7.0 --locked
bash ./scripts/build-linux-deb.sh
```

脚本使用 `x86_64-unknown-linux-gnu` release target，先恢复锁定 npm 依赖并构建前端，再执行三个程序及维护宿主的锁定 Cargo 构建，
再运行两个 CLI 产物的 `--help`，并在打包前校验所有静态文档、桌面入口与图标资源存在，
最后分别读取三个 crate 的强类型 cargo-deb 元数据生成
三个包。`dpkg-deb` 会验证包名、版本、安装树、维护宿主和 maintainer hooks，以及 GUI 对同版本两个 CLI 的精确依赖。
工作区测试覆盖 XDG 分离布局与 hooks 的安装/升级/卸载顺序；打包脚本另以临时 HOME 验证
原生产物的只读预览和非交互取消，不接触构建用户的实例。
构建脚本向 `cargo deb --output` 传入每个产物的确定路径，避免指定 Rust target 后 cargo-deb
把默认输出放到 target triple 子目录而令工作流误判产物缺失。

已经存在当前三个 Linux release binary 时可执行：

```bash
bash ./scripts/build-linux-deb.sh --skip-cargo-build
```

Windows 本地不构建 deb，也不需要安装 WSL、Docker 或未经固定的交叉编译器。正式包由
tag release GitHub Actions 在 Linux runner 上原生构建。不能把 Windows 可执行文件重打包成
deb，也不提供包含三个程序的兼容聚合包。
