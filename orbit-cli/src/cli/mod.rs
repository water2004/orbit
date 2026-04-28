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
}

#[derive(Subcommand)]
pub enum Commands {
    /// 初始化当前目录为 Orbit 项目
    Init {
        /// 实例名称
        name: String,
    },
    /// 实例管理
    Instances {
        #[command(subcommand)]
        command: InstanceCommands,
    },
    /// 本地状态双向对齐
    Sync,
    /// 检查更新（只读）
    Update,
    /// 执行更新
    Upgrade {
        /// 指定模组名称，如果不填则更新所有
        mod_name: Option<String>,
    },
    /// 搜索模组
    Search {
        /// 搜索关键词
        query: String,
    },
    /// 下载并补齐模组
    Install {
        /// 指定模组，支持 mr:name, cf:name 等格式
        mod_name: Option<String>,
    },
    /// 卸载模组
    Remove {
        /// 模组名称
        mod_name: String,
    },
    /// 深度清理模组及其配置
    Purge {
        /// 模组名称
        mod_name: String,
    },
    /// 列出已安装模组
    List,
    /// 导入外部模组
    Import {
        /// 文件路径 (.toml 或 .zip)
        file: String,
    },
    /// 导出当前实例为压缩包
    Export {
        /// 输出文件路径
        file: Option<String>,
    },
    /// 跨版本升级预检
    Check {
        /// 目标 MC 版本 (如 1.21)
        version: String,
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
            Commands::Init { name } => handle_init(name).await,
            Commands::Instances { command } => command.execute().await,
            Commands::Sync => handle_sync().await,
            Commands::Update => handle_update().await,
            Commands::Upgrade { mod_name } => handle_upgrade(mod_name).await,
            Commands::Search { query } => handle_search(query).await,
            Commands::Install { mod_name } => handle_install(mod_name).await,
            Commands::Remove { mod_name } => handle_remove(mod_name).await,
            Commands::Purge { mod_name } => handle_purge(mod_name).await,
            Commands::List => handle_list().await,
            Commands::Import { file } => handle_import(file).await,
            Commands::Export { file } => handle_export(file).await,
            Commands::Check { version } => handle_check(version).await,
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
