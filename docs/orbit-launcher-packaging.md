# Orbit Launcher 发布与安装包

`orbit-launcher` 与 `orbit` 使用独立版本和 tag。Launcher release tag 固定为
`launcher-v<三段版本>`，例如 `launcher-v0.1.0`；tag 必须指向 `main` 中的提交，并与
`orbit-launcher-cli/Cargo.toml` 完全一致。正式发布前还必须存在
`docs/launcher-releases/v<version>.md`。

GitHub Actions 在原生平台构建并验证两个独立产物：

- Windows x64：`orbit-launcher-<version>-x86_64.msi`；
- Debian/Ubuntu amd64：`orbit-launcher_<version>-1_amd64.deb`。

Windows MSI 使用独立 UpgradeCode，安装到 `%ProgramFiles%\Orbit Launcher\bin`。向导默认
勾选把 `orbit-launcher` 加入系统 PATH；同版本重建可通过 MajorUpgrade 替换。卸载时用户可
选择是否移除安装时记录的默认 Roaming/Local AppData：这会删除 Launcher 配置、账户元数据、
当前用户 DPAPI 密文、运行时注册表、受管 Java 和缓存，但不会删除 Minecraft 实例或显式
自定义目录。静默参数为 `ADD_TO_PATH=0|1` 与 `REMOVE_APPDATA=0|1`。

Windows 本地构建：

```powershell
./scripts/build-launcher-windows-msi.ps1
```

Linux 原生构建需要稳定 Rust、Python 3、`dpkg-deb` 和固定的 `cargo-deb 3.7.0`：

```bash
cargo install cargo-deb --version 3.7.0 --locked
bash ./scripts/build-launcher-linux-deb.sh
```

Windows 不交叉伪造 deb。tag workflow 使用 `ubuntu-22.04` 原生编译并执行二进制的 `--help`
烟雾测试；MSI 在 `windows-latest` 上通过 WiX 7 构建与 ICE 校验。发布 job 汇总产物并生成
`SHA256SUMS`。
