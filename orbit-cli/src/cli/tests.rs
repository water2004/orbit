use clap::{CommandFactory, Parser};
use std::path::PathBuf;

use super::{
    BoundInclusion, Cli, Commands, ConfigCommands, ConstraintCommands, ConstraintPolicyCommands,
    InstanceCommands, MigrateCommands, PackageCommands, RemoteCommands,
};

#[test]
fn clap_schema_has_no_global_subcommand_argument_collisions() {
    Cli::command().debug_assert();
}

#[test]
fn instance_config_read_and_write_have_distinct_mutation_semantics() {
    let read = Cli::try_parse_from(["orbit", "config", "--scope", "instance", "list"]).unwrap();
    assert!(!read.command.mutates_instance());
    let write = Cli::try_parse_from([
        "orbit",
        "config",
        "--scope",
        "instance",
        "set",
        "resolver.catalogs",
        "modrinth,curseforge",
    ])
    .unwrap();
    assert!(write.command.mutates_instance());
    assert_eq!(write.command.command_name(), "config.set");
    let Commands::Config {
        scope: super::ConfigScope::Instance,
        command: ConfigCommands::Set { key, value },
    } = write.command
    else {
        panic!("instance config set not parsed");
    };
    assert_eq!(key, "resolver.catalogs");
    assert_eq!(value, "modrinth,curseforge");
}

#[test]
fn language_flag_is_an_optional_override_of_global_config() {
    let configured = Cli::try_parse_from(["orbit", "config", "path"]).unwrap();
    assert!(configured.language.is_none());

    let explicit = Cli::try_parse_from(["orbit", "--language", "zh-CN", "config", "path"]).unwrap();
    assert_eq!(
        explicit.language,
        Some(orbit_i18n::LanguageMode::SimplifiedChinese)
    );
}

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
        }
        .mutates_instance()
    );
    assert!(
        Commands::Import {
            file: "pack.orbitbundle".to_string(),
            merge_strategy: None,
            optional_files: Vec::new(),
            all_optional: false,
        }
        .mutates_instance()
    );
    assert!(!Commands::Outdated { mod_name: None }.mutates_instance());
    assert!(
        Commands::Package {
            package: "sodium".to_string(),
            command: PackageCommands::Reset,
        }
        .mutates_instance()
    );
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
            archive_format: "orbit".to_string(),
            content: "mods-and-data".to_string(),
        }
        .mutates_instance()
    );
}

#[test]
fn mrpack_optional_files_are_selected_by_exact_path() {
    let cli = Cli::try_parse_from([
        "orbit",
        "import",
        "pack.mrpack",
        "--optional",
        "mods/map.jar",
        "--optional",
        "resourcepacks/extra.zip",
    ])
    .unwrap();
    let Commands::Import {
        optional_files,
        all_optional,
        ..
    } = cli.command
    else {
        panic!("unexpected command");
    };
    assert_eq!(optional_files, ["mods/map.jar", "resourcepacks/extra.zip"]);
    assert!(!all_optional);
}

#[test]
fn machine_output_and_archive_formats_are_independent_arguments() {
    let cli = Cli::try_parse_from([
        "orbit",
        "--output-format",
        "json",
        "--progress-format",
        "ndjson",
        "export",
        "pack.mrpack",
        "--format",
        "mrpack",
    ])
    .unwrap();
    assert_eq!(cli.output_format, super::OutputFormat::Json);
    let Commands::Export { archive_format, .. } = cli.command else {
        panic!("export command was not parsed");
    };
    assert_eq!(archive_format, "mrpack");
}

#[test]
fn remote_removal_accepts_a_human_visible_list_index() {
    let cli = Cli::try_parse_from([
        "orbit", "package", "sodium", "remote", "remove", "--index", "2",
    ])
    .unwrap();
    let Commands::Package {
        package,
        command:
            PackageCommands::Remote {
                command:
                    RemoteCommands::Remove {
                        provider,
                        locator,
                        index,
                    },
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
    let cli = Cli::try_parse_from(["orbit", "package", "sodium", "env", "auto"]).unwrap();
    let Commands::Package {
        package,
        command: PackageCommands::Env { environment },
    } = cli.command
    else {
        panic!("env command was not parsed");
    };

    assert_eq!(package, "sodium");
    assert_eq!(environment, "auto");
}

#[test]
fn package_activation_commands_are_explicit_mutations() {
    let enable = Cli::try_parse_from(["orbit", "package", "sodium", "enable"]).unwrap();
    assert!(enable.command.mutates_instance());
    assert!(matches!(
        &enable.command,
        Commands::Package { package, command: PackageCommands::Enable } if package == "sodium"
    ));

    let disable = Cli::try_parse_from(["orbit", "package", "sodium", "disable"]).unwrap();
    assert!(disable.command.mutates_instance());
    assert!(matches!(
        &disable.command,
        Commands::Package { package, command: PackageCommands::Disable } if package == "sodium"
    ));
}

#[test]
fn joint_launch_selects_the_launcher_instance_and_server_target() {
    let cli = Cli::try_parse_from([
        "orbit",
        "launch",
        "--launcher",
        "D:/Orbit/orbit-launcher.exe",
        "--launcher-instance",
        "server-26.2",
        "--server",
    ])
    .unwrap();
    assert!(cli.command.mutates_instance());
    let Commands::Launch {
        launcher,
        launcher_instance,
        server,
        runtime_agent,
    } = cli.command
    else {
        panic!("launch command was not parsed");
    };
    assert_eq!(launcher, Some(PathBuf::from("D:/Orbit/orbit-launcher.exe")));
    assert_eq!(launcher_instance.as_deref(), Some("server-26.2"));
    assert!(server);
    assert!(runtime_agent.is_none());
}

#[test]
fn add_accepts_the_frontends_explicit_complete_string_constraint() {
    let cli = Cli::try_parse_from([
        "orbit",
        "add",
        "sodium",
        "--string",
        orbit_machine_protocol::RECOMMENDED_NEW_PACKAGE_STRING,
    ])
    .unwrap();
    let Commands::Add { string, .. } = cli.command else {
        panic!("add command was not parsed");
    };
    assert_eq!(
        string.as_deref(),
        Some(orbit_machine_protocol::RECOMMENDED_NEW_PACKAGE_STRING)
    );
}

#[test]
fn config_set_accepts_canonical_typed_key_syntax() {
    let cli =
        Cli::try_parse_from(["orbit", "config", "set", "cache.capacity-mib", "2048"]).unwrap();
    let Commands::Config {
        command: ConfigCommands::Set { key, value },
        scope: super::ConfigScope::Global,
    } = cli.command
    else {
        panic!("config set command was not parsed");
    };

    assert_eq!(key, "cache.capacity-mib");
    assert_eq!(value, "2048");
}

#[test]
fn package_version_inventory_and_policy_commands_are_unambiguous() {
    let versions = Cli::try_parse_from(["orbit", "package", "sodium", "versions"]).unwrap();
    let Commands::Package {
        package,
        command: PackageCommands::Versions,
    } = versions.command
    else {
        panic!("versions command was not parsed");
    };
    assert_eq!(package, "sodium");

    let ownership = Cli::try_parse_from(["orbit", "package", "sodium", "files"]).unwrap();
    let Commands::Package {
        package,
        command: PackageCommands::Files,
    } = ownership.command
    else {
        panic!("ownership command was not parsed");
    };
    assert_eq!(package, "sodium");

    let constraint = Cli::try_parse_from([
        "orbit",
        "package",
        "sodium",
        "constraint",
        "set",
        "exact",
        "0.6.13",
    ])
    .unwrap();
    let Commands::Package {
        package,
        command:
            PackageCommands::Constraint {
                command:
                    ConstraintCommands::Set {
                        string: _,
                        policy: ConstraintPolicyCommands::Exact { version },
                    },
            },
    } = constraint.command
    else {
        panic!("constraint set command was not parsed");
    };
    assert_eq!(package, "sodium");
    assert_eq!(version, "0.6.13");

    let string = Cli::try_parse_from([
        "orbit",
        "package",
        "sodium",
        "constraint",
        "set",
        "any",
        "--string",
        "all; intersect not contains(i\"beta\"); union ends_with(\"fabric\")",
    ])
    .unwrap();
    let Commands::Package {
        package: _,
        command:
            PackageCommands::Constraint {
                command: ConstraintCommands::Set { string, .. },
            },
    } = string.command
    else {
        panic!("string expression was not parsed");
    };
    assert_eq!(
        string.as_deref(),
        Some("all; intersect not contains(i\"beta\"); union ends_with(\"fabric\")")
    );

    let range = Cli::try_parse_from([
        "orbit",
        "package",
        "sodium",
        "constraint",
        "set",
        "range",
        "0.6.0",
        "0.7.0",
        "--upper-bound",
        "exclusive",
    ])
    .unwrap();
    assert!(matches!(
        range.command,
        Commands::Package {
            package: _,
            command: PackageCommands::Constraint {
                command: ConstraintCommands::Set {
                    policy: ConstraintPolicyCommands::Range {
                        lower_bound: BoundInclusion::Inclusive,
                        upper_bound: BoundInclusion::Exclusive,
                        ..
                    },
                    ..
                }
            }
        }
    ));
}

#[test]
fn migration_is_namespaced_and_only_export_mutates() {
    let check = Cli::try_parse_from(["orbit", "migrate", "check", "target"]).unwrap();
    let Commands::Migrate {
        command:
            MigrateCommands::Check {
                target,
                source_pack,
                allow_removals,
            },
    } = check.command
    else {
        panic!("migrate check was not parsed");
    };
    assert_eq!(target, PathBuf::from("target"));
    assert!(source_pack.is_none());
    assert!(!allow_removals);

    let export = Cli::try_parse_from([
        "orbit",
        "migrate",
        "export",
        "target",
        "--source-pack",
        "source.zip",
        "--consume-source-pack",
    ])
    .unwrap();
    assert!(export.command.mutates_instance());
    assert!(Cli::try_parse_from(["orbit", "check", "1.21"]).is_err());
}

#[test]
fn existing_instance_registration_requires_an_explicit_name_and_path() {
    let cli = Cli::try_parse_from([
        "orbit",
        "instance",
        "register",
        "fabric-1.21.1",
        "D:/Minecraft/instances/fabric-1.21.1",
    ])
    .unwrap();
    let Commands::Instance {
        command: InstanceCommands::Register { name, path },
    } = cli.command
    else {
        panic!("instances register was not parsed");
    };

    assert_eq!(name, "fabric-1.21.1");
    assert_eq!(path, PathBuf::from("D:/Minecraft/instances/fabric-1.21.1"));
    assert!(Cli::try_parse_from(["orbit", "instance", "register", "name"]).is_err());
}

#[test]
fn retired_entrypoints_are_not_compatibility_aliases() {
    for old in [
        "env",
        "constraint",
        "versions",
        "ownership",
        "enable",
        "disable",
        "purge",
        "reset",
        "remote",
        "instances",
    ] {
        assert!(Cli::try_parse_from(["orbit", old]).is_err(), "{old}");
    }
    assert!(Cli::try_parse_from(["orbit", "instance", "default", "test"]).is_err());
}

#[test]
fn package_actions_have_one_target_and_canonical_protocol_identity() {
    for (action, mutates) in [
        ("versions", false),
        ("files", false),
        ("enable", true),
        ("disable", true),
        ("reset", true),
        ("purge", true),
    ] {
        let cli = Cli::try_parse_from(["orbit", "--instance", "test", "package", "sodium", action])
            .unwrap();
        assert_eq!(cli.command.command_name(), format!("package.{action}"));
        assert_eq!(cli.command.mutates_instance(), mutates);
    }
    for (action, mutates) in [("show", false), ("set", true)] {
        let mut args = vec!["orbit", "package", "sodium", "constraint", action];
        if mutates {
            args.push("any");
        }
        let cli = Cli::try_parse_from(args).unwrap();
        assert_eq!(
            cli.command.command_name(),
            format!("package.constraint.{action}")
        );
        assert_eq!(cli.command.mutates_instance(), mutates);
    }
}
#[test]
fn gui_command_contract_is_accepted_by_the_real_parser() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../orbit-machine-protocol/tests/fixtures/command-contract.json"
    ))
    .unwrap();
    for case in cases.into_iter().filter(|case| case["kind"] == "orbit") {
        let args = std::iter::once("orbit").chain(
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
            cli.command.mutates_instance(),
            case["mutates"].as_bool().unwrap()
        );
    }
}
#[test]
fn command_wide_flags_cannot_be_silently_ignored_by_global_mutations() {
    for args in [
        vec!["config", "list"],
        vec!["cache", "clean"],
        vec!["instance", "list"],
        vec!["init", "new"],
        vec!["update", "check"],
    ] {
        let cli = Cli::try_parse_from(std::iter::once("orbit").chain(args)).unwrap();
        assert!(cli.command.validate_invocation(true, false).is_err());
    }
    for args in [
        vec!["instance", "register", "test", "D:/test"],
        vec!["instance", "unregister", "test"],
    ] {
        let cli = Cli::try_parse_from(std::iter::once("orbit").chain(args)).unwrap();
        assert!(cli.command.validate_invocation(false, true).is_err());
    }
    for args in [
        vec!["package", "sodium", "remote", "remove"],
        vec!["package", "sodium", "remote", "remove", "modrinth"],
    ] {
        assert!(Cli::try_parse_from(std::iter::once("orbit").chain(args)).is_err());
    }
}
