pub mod commands;
use clap::{Parser, Subcommand};
use crate::cli::commands::CommandHandler;
use anyhow::Result;

#[derive(Parser)]
#[command(name = "orbit")]
#[command(about = "The Modern, Non-intrusive Package Manager for Minecraft Mods.", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// 指定操作的实例名称
    #[arg(short = 'i', long, global = true)]
    pub instance: Option<String>,

    /// 输出详细日志
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// 静默模式，仅输出错误
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// 跳过所有交互式确认
    #[arg(short = 'y', long, global = true)]
    pub yes: bool,

    /// 仅模拟执行，不修改任何文件
    #[arg(long, global = true)]
    pub dry_run: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// 初始化当前目录为 Orbit 项目
    Init {
        /// 实例名称
        name: String,
        /// Minecraft 版本
        #[arg(long)]
        mc_version: Option<String>,
        /// 模组加载器
        #[arg(long)]
        modloader: Option<String>,
        /// 加载器版本
        #[arg(long)]
        modloader_version: Option<String>,
    },

    /// 实例管理
    Instances {
        #[command(subcommand)]
        command: InstanceCommands,
    },

    /// 根据清单还原模组环境
    Install {
        /// 目标环境: client / server / both (默认)
        #[arg(long)]
        target: Option<String>,
        /// 仅安装指定分组
        #[arg(long)]
        group: Option<String>,
        /// 跳过可选依赖
        #[arg(long)]
        no_optional: bool,
        /// 仅使用 lockfile，不发起网络解析（生产环境）
        #[arg(long)]
        locked: bool,
        /// --locked 的别名（兼容 npm 用户）
        #[arg(long)]
        frozen: bool,
    },

    /// 添加新模组
    Add {
        /// 模组名称，支持前缀: mr:name, cf:name, file:path
        mod_name: String,
        /// 指定平台
        #[arg(long)]
        platform: Option<String>,
        /// 版本约束
        #[arg(long)]
        version: Option<String>,
        /// 端侧限定: client / server / both
        #[arg(long)]
        env: Option<String>,
        /// 标记为可选依赖
        #[arg(long)]
        optional: bool,
        /// 不安装传递依赖
        #[arg(long)]
        no_deps: bool,
    },

    /// 卸载模组
    Remove {
        /// 模组名称
        mod_name: String,
    },

    /// 深度清理模组及其配置文件
    Purge {
        /// 模组名称
        mod_name: String,
    },

    /// 本地状态双向对齐
    Sync,

    /// 检查过时模组（只读）
    Outdated {
        /// 指定模组名称
        mod_name: Option<String>,
    },

    /// 执行模组升级
    Upgrade {
        /// 指定模组名称，不填则升级所有
        mod_name: Option<String>,
    },

    /// 搜索模组
    Search {
        /// 搜索关键词
        query: String,
        /// 指定平台
        #[arg(long)]
        platform: Option<String>,
        /// 结果数量限制
        #[arg(long, default_value = "20")]
        limit: usize,
        /// 按 Minecraft 版本过滤
        #[arg(long)]
        mc_version: Option<String>,
        /// 按模组加载器过滤 (fabric, forge, quilt, etc.)
        #[arg(long)]
        modloader: Option<String>,
    },

    /// 查看模组详细信息
    Info {
        /// 模组名称
        mod_name: String,
        /// 指定平台
        #[arg(long)]
        platform: Option<String>,
    },

    /// 列出已安装模组
    List {
        /// 树状展示依赖关系
        #[arg(long)]
        tree: bool,
        /// 按环境过滤
        #[arg(long)]
        target: Option<String>,
    },

    /// 导入外部模组清单
    Import {
        /// 文件路径 (.toml 或 .zip)
        file: String,
        /// 合并策略
        #[arg(long)]
        merge_strategy: Option<String>,
    },

    /// 导出当前实例为压缩包
    Export {
        /// 输出文件路径
        file: Option<String>,
        /// 目标环境过滤
        #[arg(long)]
        target: Option<String>,
        /// 导出格式: zip / mrpack
        #[arg(long, default_value = "zip")]
        format: String,
    },

    /// 跨版本升级预检
    Check {
        /// 目标 MC 版本 (如 1.21)
        version: String,
        /// 目标加载器
        #[arg(long)]
        modloader: Option<String>,
    },

    /// 清理全局下载缓存
    Cache {
        #[command(subcommand)]
        command: CacheCommands,
    },
}

#[derive(Subcommand)]
pub enum InstanceCommands {
    /// 列出所有被托管的 MC 实例
    List,
    /// 将指定实例设为全局默认
    Default { name: String },
    /// 移除对该实例的追踪
    Remove { name: String },
}

#[derive(Subcommand)]
pub enum CacheCommands {
    /// 清理下载缓存
    Clean,
}

impl CommandHandler for Commands {
    async fn execute(self) -> Result<()> {
        use crate::cli::commands::*;
        match self {
            Commands::Init { name, mc_version, modloader, modloader_version } => {
                handle_init(name, mc_version, modloader, modloader_version).await
            }
            Commands::Instances { command } => command.execute().await,
            Commands::Install { target, group, no_optional, locked, frozen } => {
                handle_install(target, group, no_optional, locked || frozen).await
            }
            Commands::Add { mod_name, platform, version, env, optional, no_deps } => {
                handle_add(mod_name, platform, version, env, optional, no_deps).await
            }
            Commands::Remove { mod_name } => handle_remove(mod_name).await,
            Commands::Purge { mod_name } => handle_purge(mod_name).await,
            Commands::Sync => handle_sync().await,
            Commands::Outdated { mod_name } => handle_outdated(mod_name).await,
            Commands::Upgrade { mod_name } => handle_upgrade(mod_name).await,
            Commands::Search { query, platform, limit, mc_version, modloader } => {
                handle_search(query, platform, limit, mc_version, modloader).await
            }
            Commands::Info { mod_name, platform } => handle_info(mod_name, platform).await,
            Commands::List { tree, target } => handle_list(tree, target).await,
            Commands::Import { file, merge_strategy } => handle_import(file, merge_strategy).await,
            Commands::Export { file, target, format } => handle_export(file, target, format).await,
            Commands::Check { version, modloader } => handle_check(version, modloader).await,
            Commands::Cache { command } => command.execute().await,
        }
    }
}

impl CommandHandler for InstanceCommands {
    async fn execute(self) -> Result<()> {
        use crate::cli::commands::instances::*;
        match self {
            InstanceCommands::List => handle_list().await,
            InstanceCommands::Default { name } => handle_default(name).await,
            InstanceCommands::Remove { name } => handle_remove(name).await,
        }
    }
}

impl CommandHandler for CacheCommands {
    async fn execute(self) -> Result<()> {
        use crate::cli::commands::cache::clean;
        match self {
            CacheCommands::Clean => clean::handle().await,
        }
    }
}
