# Linux deb

Orbit 的 Debian 包面向 `amd64`，安装可执行文件到 `/usr/bin/orbit`，并把 README 与
MIT 许可信息放入 `/usr/share/doc/orbit/`。用户配置和 JAR cache 位于用户级 XDG
目录，不属于 deb 管理的系统文件；卸载软件包不会删除用户数据或 Minecraft 实例。

安装 GitHub Release 中的包：

```bash
sudo apt install ./orbit_0.1.1-1_amd64.deb
orbit --help
```

卸载：

```bash
sudo apt remove orbit
```

## 本地构建

本地构建要求 Linux、稳定版 Rust、Python 3、`dpkg-deb` 与固定版本
`cargo-deb 3.7.0`：

```bash
cargo install cargo-deb --version 3.7.0 --locked
bash ./scripts/build-linux-deb.sh
```

脚本使用 `x86_64-unknown-linux-gnu` release target，先执行锁定依赖的 Cargo 构建，再
运行产物的 `--help`，最后由 cargo-deb 根据 `orbit-cli/Cargo.toml` 的强类型包元数据
生成 deb，并用 `dpkg-deb --info/--contents` 验证控制信息与安装树。

已经存在当前 Linux release binary 时可执行：

```bash
bash ./scripts/build-linux-deb.sh --skip-cargo-build
```

Windows 本地不构建 deb，也不需要安装 WSL、Docker 或未经固定的交叉编译器。正式包由
tag release GitHub Actions 在 Linux runner 上原生构建。
