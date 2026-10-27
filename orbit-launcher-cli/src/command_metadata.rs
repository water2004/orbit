//! Canonical command identity shared by success, errors, and progress.
use crate::cli::*;
impl Commands {
    /// The sole authority for whether --instance can target this operation.
    pub fn accepts_instance_context(&self) -> bool {
        match self {
            Self::Install { new, .. } => new.is_none(),
            Self::Launch { .. } | Self::Export { .. } | Self::Server { .. } => true,
            Self::Instance { command } => command.accepts_instance_context(),
            Self::Config { .. }
            | Self::Update { .. }
            | Self::Account { .. }
            | Self::Versions { .. }
            | Self::Java { .. }
            | Self::Package { .. }
            | Self::Minecraft { .. }
            | Self::Supervisor => false,
        }
    }

    pub fn command_name(&self) -> &'static str {
        match self {
            Self::Update { .. } => "update.check",
            Self::Install { .. } => "install",
            Self::Launch { .. } => "launch",
            Self::Export { .. } => "export",
            Self::Config { command } => match command {
                ConfigCommands::Path => "config.path",
                ConfigCommands::List => "config.list",
                ConfigCommands::Get { .. } => "config.get",
                ConfigCommands::Set { .. } => "config.set",
                ConfigCommands::Unset { .. } => "config.unset",
                ConfigCommands::Yggdrasil { command } => match command {
                    YggdrasilProviderCommands::List => "config.yggdrasil.list",
                    YggdrasilProviderCommands::Inspect { .. } => "config.yggdrasil.inspect",
                    YggdrasilProviderCommands::Add { .. } => "config.yggdrasil.add",
                    YggdrasilProviderCommands::Remove { .. } => "config.yggdrasil.remove",
                },
            },
            Self::Instance { command } => match command {
                InstanceCommands::Import { .. } => "instance.import",
                InstanceCommands::Inspect { .. } => "instance.inspect",
                InstanceCommands::Delete { .. } => "instance.delete",
                InstanceCommands::List => "instance.list",
                InstanceCommands::Show => "instance.show",
                InstanceCommands::LaunchReport => "instance.launch-report",
                InstanceCommands::Rename { .. } => "instance.rename",
                InstanceCommands::Configure { .. } => "instance.configure",
                InstanceCommands::Resolution { .. } => "instance.resolution",
                InstanceCommands::Memory { .. } => "instance.memory",
                InstanceCommands::Icon { .. } => "instance.icon",
                InstanceCommands::Java { .. } => "instance.java",
                InstanceCommands::Unregister => "instance.unregister",
                InstanceCommands::Default { .. } => "instance.default",
                InstanceCommands::Account { command } => match command {
                    InstanceAccountCommands::Show => "instance.account.show",
                    InstanceAccountCommands::Set { .. } => "instance.account.set",
                    InstanceAccountCommands::Clear => "instance.account.clear",
                },
            },
            Self::Server { command } => match command {
                ServerCommands::Run { .. } => "server.run",
                ServerCommands::Start => "server.start",
                ServerCommands::Stop => "server.stop",
                ServerCommands::Status => "server.status",
                ServerCommands::Command { .. } => "server.command",
                ServerCommands::Eula { command } => match command {
                    EulaCommands::Show => "server.eula.show",
                    EulaCommands::Accept { .. } => "server.eula.accept",
                },
            },
            Self::Account { command } => match command {
                AccountCommands::Login { command } => match command {
                    AccountLoginCommands::Offline { .. } => "account.login.offline",
                    AccountLoginCommands::Yggdrasil { .. } => "account.login.yggdrasil",
                    AccountLoginCommands::Microsoft { command } => match command {
                        MicrosoftLoginCommands::Begin => "account.login.microsoft.begin",
                        MicrosoftLoginCommands::Complete { .. } => {
                            "account.login.microsoft.complete"
                        }
                    },
                },
                AccountCommands::List => "account.list",
                AccountCommands::Show { .. } => "account.show",
                AccountCommands::Refresh { .. } => "account.refresh",
                AccountCommands::Default { command } => match command {
                    AccountDefaultCommands::Show => "account.default.show",
                    AccountDefaultCommands::Set { .. } => "account.default.set",
                    AccountDefaultCommands::Clear => "account.default.clear",
                },
                AccountCommands::Logout { .. } => "account.logout",
            },
            Self::Versions { command } => match command {
                VersionCommands::Minecraft => "versions.minecraft",
                VersionCommands::Loader { .. } => "versions.loader",
                VersionCommands::Java { .. } => "versions.java",
            },
            Self::Java { command } => match command {
                JavaCommands::List { .. } => "java.list",
                JavaCommands::Verify { .. } => "java.verify",
                JavaCommands::Remove { .. } => "java.remove",
            },
            Self::Package { command } => match command {
                PackageCommands::Inspect { .. } => "package.inspect",
            },
            Self::Minecraft { command } => match command {
                MinecraftCommands::Directory => "minecraft.directory",
                MinecraftCommands::Move { .. } => "minecraft.move",
            },
            Self::Supervisor => "server.supervisor",
        }
    }
}
#[cfg(test)]
mod tests {
    use crate::cli::Cli;
    use clap::Parser;
    #[test]
    fn gui_command_contract_has_one_protocol_identity_and_target_policy() {
        let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
            "../../orbit-machine-protocol/tests/fixtures/command-contract.json"
        ))
        .unwrap();
        for case in cases.into_iter().filter(|case| case["kind"] == "launcher") {
            let args = std::iter::once("orbit-launcher").chain(
                case["args"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|arg| arg.as_str().unwrap()),
            );
            let cli = Cli::try_parse_from(args).unwrap();
            assert_eq!(
                cli.command.command_name(),
                case["command"].as_str().unwrap()
            );
            assert_eq!(
                cli.command.accepts_instance_context(),
                case["args"][0] == "instance"
            );
        }
    }
    #[test]
    fn abandoned_entrypoints_are_not_retained_as_aliases() {
        for args in [
            vec!["instance", "create"],
            vec!["instance", "remove"],
            vec!["account", "select", "player"],
            vec!["account", "clear"],
        ] {
            assert!(Cli::try_parse_from(std::iter::once("orbit-launcher").chain(args)).is_err());
        }
        assert!(Cli::try_parse_from(["orbit-launcher", "account", "show"]).is_err());
    }
}
