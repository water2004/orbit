pub mod commands;
use clap::{Parser, Subcommand};

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
