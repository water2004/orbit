# 已有实例导入与完整删除

## 边界与共同模型

外部启动器仅由 `external_instance/` 在导入边界解析，归一化为准确运行时、profile
目录、实际 game directory 和来源证据。正常安装和启动不探测它们，GUI 不扫描目录、
解析 metadata 或生成实例配置。Launcher 不调用 Orbit、不识别 mod、不求解依赖。

| 来源 | 运行时 | 数据目录 |
| --- | --- | --- |
| 官方/标准 JSON | inheritance、libraries、game patch；存在时核对 Minecraft JAR version.json | 匹配版本的 launcher_profiles.json gameDir；缺省仓库根 |
| HMCL | 同一 profile 管线 | hmclversion.cfg 的 ROOT_FOLDER / VERSION_FOLDER / CUSTOM；modpack 强制隔离；新版 runningDirectory override |
| PCL | 同一 profile 管线 | PCL/Setup.ini 的 VersionArgumentIndieV2 或旧版明确 1/2；其 INI 用冒号而非等号 |
| Prism/MultiMC | mmc-pack.json formatVersion 1 components | .minecraft 或 minecraft；二者同时存在不自动选 |
| CurseForge | minecraftinstance.json gameVersion / baseModLoader.name | profile 根；Fabric/Quilt 名尾部精确 MC 后缀不属于 loader 版本 |
| GDLauncher Carbon | `_version = "1"` 的标准 game_configuration | instance/，不是元数据父目录 |
| ATLauncher | instance.json 中的 Minecraft profile | 实例根；bin 是旧运行时，不是游戏数据 |
| Modrinth App | 只读 app.db，profiles 或 instances + applied content set | settings.custom_dir（若有）下的 profiles；不读未应用草稿或账户表 |
| Orbit | 自有 manifest | 原地重新注册，不复制/重装；使用同一 import 命令及统一结果结构 |

只枚举所选仓库的版本或实例库的直接子实例。选择准确 versions/id 时不扫描相邻版本，
不递归搜索用户磁盘。多候选必须指定 source；HMCL/PCL 继承的全局设置不可取得时，
必须明确选择实际运行目录，不推测隔离默认值；多个官方 profile 指向不同目录亦同。

外部客户端复制到受管仓库的 `instances/<新名称>`，不原地改造源实例，也不继续共享其
可变目录。Prism 的未知/重复运行时组件明确拒绝，不静默省略。不是任意旧 Loader、定制组件或服务端发行包转换器；是否能安装运行时仍由
现有官方 adapter/版本范围判断，未知格式显式报错，不执行外部脚本。已有 Orbit 客户端
和服务端都可重新注册与完整删除。

## 命令与 UI

```text
orbit-launcher instance inspect --directory <source>
orbit-launcher instance import --directory <source> [--source <id>] \
  [--name <new-name>] [--game-directory <actual-game-directory>] [--dry-run]
orbit-launcher --instance <id> instance delete [--dry-run] [--yes]
orbit-launcher --instance <id> instance unregister
```

inspect/dry-run 不下载、不复制、不注册、不升级来源格式。import 的统一结果为
`instance`、`copied`、`source_directory`、`files`、`bytes`、`warnings`。
GUI 先选来源、核对 MC/Loader 与数据目录，再展示 CLI 提供的目标、文件数/字节数及排除
项，确认后执行同一 import；更改名称/目录使预览失效。真实复制进度在当前弹窗展示。
导入后若没有 Orbit 工作区，可在模组页显式初始化。

## 数据与恢复

- 模组、配置、存档和任意 mod 用户数据按不透明文件复制，不以有限 mod 文件名白名单
  决定保留范围，不修改约束或修复依赖。
- 来源根的 assets/libraries/versions、natives、Java、旧启动器 bin/metadata/账户库/日志
  不作为游戏数据复制；运行时复用正常官方安装、校验和缓存。账户令牌、Java 路径覆盖、
  JVM 参数、启动钩子不导入。
- 不跟随链接/特殊数据文件（含 Windows junction/reparse point），明确报错。有界流式复制并验证 SHA-256，前后比对来源清单、
  大小和修改时间；正在变化的源必须先关闭游戏再重试。
- 数据文件在目标私有 staging 写完并校验后不覆盖地发布。普通已有目标不能合并。
  运行时和数据全部完成后才注册。普通执行失败清理本次新建目录；进程强制终止后，
  `.orbit-launcher/import.json` 保存源、身份和准确运行时，再次以相同参数导入可预览并
  继续。只复用内容相同的已发布文件，目标被修改时保留并报错，不静默覆盖。
- 不改变已发布的全局/实例 schema；既有升级仍只由 storage_migrations 执行。

## 完整删除与注销

`instance unregister` 仅注销；`instance delete` 删除整个目录，**包含 worlds/saves、mods、
截图与设置**。GUI 分别提供按钮，完整删除先展示 CLI 计划的准确名称/绝对路径，再显式
确认；终端默认要求输入 DELETE，无交互调用需传 `--yes`。

删除验证 manifest ID、canonical 根目录、其它注册实例/全局路径，不能删除共享运行库。
游戏、服务端 supervisor 与安装事务使用同一跨平台生命周期锁；锁放在实例父目录的
`.orbit-launcher-locks/`，避免 Windows 的目录内打开句柄妨碍安全暂存。

同文件系统暂存完整目录后，原子更新注册表及默认实例，再送入系统回收站。文件占用、
回收站不可用、提交失败必须报错并恢复目录与注册表；回滚失败明确报告恢复路径。禁止
悄悄退回永久递归删除。注册表变更持有同一写锁。回收站恢复后可重新 import 注册。

## 研究依据

核对日期 2026-10-25，只独立适配格式与行为，不复制第三方实现：

- [HMCL v3.6.20](https://github.com/HMCL-dev/HMCL/blob/v3.6.20/HMCL/src/main/java/org/jackhuang/hmcl/game/HMCLGameRepository.java) 及
  [新版 HMCL](https://github.com/HMCL-dev/HMCL/blob/d297417909208deb3a4bb8378af4769f97de44c8/HMCL/src/main/java/org/jackhuang/hmcl/game/HMCLGameRepository.java)。
- [PCL ModMinecraft](https://github.com/Hex-Dragon/PCL2/blob/93ede5d9e638c99fcec853a9ba1eecd91ceee7f5/Plain%20Craft%20Launcher%202/Modules/Minecraft/ModMinecraft.vb) 与 ModBase INI reader。
- [Prism MinecraftInstance](https://github.com/PrismLauncher/PrismLauncher/blob/develop/launcher/minecraft/MinecraftInstance.cpp)。
- [GDLauncher 布局](https://gdlauncher.com/docs/instance-folder-anatomy/) 与
  [Carbon schema/importers](https://github.com/gorilla-devs/GDLauncher-Carbon/tree/main/crates/carbon_app/src/managers/instance)。
- [ATLauncher Instance](https://github.com/ATLauncher/ATLauncher/blob/master/src/main/java/com/atlauncher/data/Instance.java)。
- [Modrinth storage migrations](https://github.com/modrinth/code/tree/main/packages/app-lib/migrations) 与 state/instances 查询。
