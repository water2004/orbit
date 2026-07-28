pub mod commands;
pub mod output;
mod progress;
use crate::cli::commands::CommandHandler;
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

pub use output::{OutputFormat, ProgressFormat};

#[derive(Parser)]
#[command(name = "orbit")]
#[command(about = "The Modern, Non-intrusive Package Manager for Minecraft Mods.", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// 指定操作的实例名称
    #[arg(short = 'i', long, global = true)]
    pub instance: Option<String>,

    /// 全局配置文件的精确路径
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    /// 全局 JAR 缓存目录的精确路径
    #[arg(long, global = true)]
    pub cache_dir: Option<PathBuf>,

    /// 默认路径布局: system / executable
    #[arg(long, global = true)]
    pub data_layout: Option<orbit_core::PathLayout>,

    /// 输出格式: text / json
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,

    /// 进度协议: none / ndjson（仅 stderr）
    #[arg(long, global = true, value_enum, default_value_t = ProgressFormat::None)]
    pub progress_format: ProgressFormat,

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

    /// 设置根包的环境过滤；auto 跟随选中 JAR 的声明
    Env {
        /// JAR 元数据声明的 mod_id
        package: String,
        /// client / server / both / auto
        environment: String,
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

    /// 静态分析当前实例中 Mod 的字节码兼容风险（只读）
    Audit {
        /// 仅显示综合风险指数达到该值的风险（0-100）
        #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u8).range(0..=100))]
        min_risk: u8,
        /// 存在综合风险指数达到该值的风险时返回非零退出码（0-100）
        #[arg(long, value_parser = clap::value_parser!(u8).range(0..=100))]
        fail_on_risk: Option<u8>,
        /// 仅显示涉及该 Mod（ID、文件名或展示名）的风险
        #[arg(long = "mod")]
        mod_filter: Option<String>,
        /// 将未截断的完整结构化报告写入 JSON 文件
        #[arg(long)]
        report: Option<PathBuf>,
        /// 文本模式最多展示的高排名风险数
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },

    /// 清理全局下载缓存
    Cache {
        #[command(subcommand)]
        command: CacheCommands,
    },

    /// 查看或修改全局配置
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },

    /// 管理一个包的候选来源
    Remote {
        #[command(subcommand)]
        command: RemoteCommands,
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

#[derive(Subcommand)]
pub enum ConfigCommands {
    /// 显示实际使用的全局配置文件路径
    Path,
    /// 列出所有受支持字段的文件层值
    List,
    /// 读取一个配置文件字段
    Get {
        /// 配置键，例如 cache.capacity-mib
        key: String,
    },
    /// 设置一个经过类型校验的配置文件字段
    Set {
        /// 配置键，例如 cache.capacity-mib
        key: String,
        /// 新值
        value: String,
    },
    /// 清除可选字段，或把必填字段恢复为默认值
    Unset {
        /// 配置键，例如 network.proxy
        key: String,
    },
}

#[derive(Subcommand)]
pub enum RemoteCommands {
    /// 验证并添加一个来源
    Add {
        package: String,
        /// file / modrinth / curseforge
        provider: String,
        /// 文件路径、Modrinth project ID 或 CurseForge 数值 project ID
        locator: String,
    },
    /// 移除一个来源；不能移除最后一个
    Remove {
        package: String,
        /// file / modrinth / curseforge (omit when using --index)
        provider: Option<String>,
        /// Source locator (omit when using --index)
        locator: Option<String>,
        /// One-based index shown by `orbit remote list`
        #[arg(long, conflicts_with_all = ["provider", "locator"])]
        index: Option<usize>,
    },
    /// 列出一个包的所有来源
    List { package: String },
}

impl CommandHandler for Commands {
    async fn execute(self, ctx: &commands::CliContext) -> Result<()> {
        use crate::cli::commands::*;
        if self.mutates_instance() {
            ctx.require_explicit_mutation_target()?;
        }
        match self {
            Commands::Init {
                name,
                mc_version,
                modloader,
                modloader_version,
            } => handle_init(name, mc_version, modloader, modloader_version, ctx).await,
            Commands::Instances { command } => command.execute(ctx).await,
            Commands::Install {
                target,
                group,
                no_optional,
                locked,
                frozen,
            } => handle_install(target, group, no_optional, locked || frozen, ctx).await,
            Commands::Add {
                mod_name,
                platform,
                version,
                env,
                optional,
                no_deps,
            } => handle_add(mod_name, platform, version, env, optional, no_deps, ctx).await,
            Commands::Env {
                package,
                environment,
            } => handle_env(package, environment, ctx),
            Commands::Remove { mod_name } => handle_remove(mod_name, ctx).await,
            Commands::Purge { mod_name } => handle_purge(mod_name, ctx).await,
            Commands::Sync => handle_sync(ctx).await,
            Commands::Outdated { mod_name } => handle_outdated(mod_name, ctx).await,
            Commands::Upgrade { mod_name } => handle_upgrade(mod_name, ctx).await,
            Commands::Search {
                query,
                platform,
                limit,
                mc_version,
                modloader,
            } => handle_search(query, platform, limit, mc_version, modloader, ctx).await,
            Commands::Info { mod_name, platform } => handle_info(mod_name, platform, ctx).await,
            Commands::List { tree, target } => handle_list(tree, target, ctx).await,
            Commands::Import {
                file,
                merge_strategy,
            } => handle_import(file, merge_strategy, ctx).await,
            Commands::Export {
                file,
                target,
                format,
            } => handle_export(file, target, format, ctx).await,
            Commands::Check { version, modloader } => handle_check(version, modloader, ctx).await,
            Commands::Audit {
                min_risk,
                fail_on_risk,
                mod_filter,
                report,
                limit,
            } => handle_audit(min_risk, fail_on_risk, mod_filter, report, limit, ctx).await,
            Commands::Cache { command } => command.execute(ctx).await,
            Commands::Config { command } => handle_config(command, ctx).await,
            Commands::Remote { command } => handle_remote(command, ctx).await,
        }
    }
}

impl Commands {
    pub fn command_name(&self) -> &'static str {
        match self {
            Self::Init { .. } => "init",
            Self::Instances { .. } => "instances",
            Self::Install { .. } => "install",
            Self::Add { .. } => "add",
            Self::Env { .. } => "env",
            Self::Remove { .. } => "remove",
            Self::Purge { .. } => "purge",
            Self::Sync => "sync",
            Self::Outdated { .. } => "outdated",
            Self::Upgrade { .. } => "upgrade",
            Self::Search { .. } => "search",
            Self::Info { .. } => "info",
            Self::List { .. } => "list",
            Self::Import { .. } => "import",
            Self::Export { .. } => "export",
            Self::Check { .. } => "check",
            Self::Audit { .. } => "audit",
            Self::Cache { .. } => "cache",
            Self::Config { .. } => "config",
            Self::Remote { .. } => "remote",
        }
    }

    fn mutates_instance(&self) -> bool {
        matches!(
            self,
            Self::Install { .. }
                | Self::Add { .. }
                | Self::Env { .. }
                | Self::Remove { .. }
                | Self::Purge { .. }
                | Self::Sync
                | Self::Upgrade { .. }
                | Self::Import { .. }
                | Self::Remote {
                    command: RemoteCommands::Add { .. } | RemoteCommands::Remove { .. }
                }
        )
    }
}

impl CommandHandler for InstanceCommands {
    async fn execute(self, ctx: &commands::CliContext) -> Result<()> {
        use crate::cli::commands::instances::*;
        match self {
            InstanceCommands::List => handle_list(ctx).await,
            InstanceCommands::Default { name } => handle_default(name, ctx).await,
            InstanceCommands::Remove { name } => handle_remove(name, ctx).await,
        }
    }
}

impl CommandHandler for CacheCommands {
    async fn execute(self, ctx: &commands::CliContext) -> Result<()> {
        use crate::cli::commands::cache::clean;
        match self {
            CacheCommands::Clean => clean::handle(ctx).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{Cli, Commands, ConfigCommands, RemoteCommands};

    #[test]
    fn audit_defaults_do_not_request_a_report_file() {
        let cli = Cli::try_parse_from(["orbit", "audit"]).unwrap();
        let Commands::Audit { report, limit, .. } = cli.command else {
            panic!("audit command was not parsed");
        };

        assert!(report.is_none());
        assert_eq!(limit, 20);
    }

    #[test]
    fn classifies_instance_mutations_for_default_fallback_safety() {
        assert!(
            Commands::Install {
                target: None,
                group: None,
                no_optional: false,
                locked: false,
                frozen: false,
            }
            .mutates_instance()
        );
        assert!(
            Commands::Import {
                file: "pack.zip".to_string(),
                merge_strategy: None,
            }
            .mutates_instance()
        );
        assert!(!Commands::Outdated { mod_name: None }.mutates_instance());
        assert!(
            !Commands::Audit {
                min_risk: 0,
                fail_on_risk: None,
                mod_filter: None,
                report: None,
                limit: 20,
            }
            .mutates_instance()
        );
        assert!(
            !Commands::Export {
                file: None,
                target: None,
                format: "zip".to_string(),
            }
            .mutates_instance()
        );
    }

    #[test]
    fn remote_removal_accepts_a_human_visible_list_index() {
        let cli =
            Cli::try_parse_from(["orbit", "remote", "remove", "sodium", "--index", "2"]).unwrap();
        let Commands::Remote {
            command:
                RemoteCommands::Remove {
                    package,
                    provider,
                    locator,
                    index,
                },
        } = cli.command
        else {
            panic!("remote remove command was not parsed");
        };

        assert_eq!(package, "sodium");
        assert_eq!(index, Some(2));
        assert!(provider.is_none());
        assert!(locator.is_none());
    }

    #[test]
    fn env_command_accepts_explicit_and_auto_values_for_core_validation() {
        let cli = Cli::try_parse_from(["orbit", "env", "sodium", "auto"]).unwrap();
        let Commands::Env {
            package,
            environment,
        } = cli.command
        else {
            panic!("env command was not parsed");
        };

        assert_eq!(package, "sodium");
        assert_eq!(environment, "auto");
    }

    #[test]
    fn config_set_accepts_canonical_typed_key_syntax() {
        let cli =
            Cli::try_parse_from(["orbit", "config", "set", "cache.capacity-mib", "2048"]).unwrap();
        let Commands::Config {
            command: ConfigCommands::Set { key, value },
        } = cli.command
        else {
            panic!("config set command was not parsed");
        };

        assert_eq!(key, "cache.capacity-mib");
        assert_eq!(value, "2048");
    }
}
