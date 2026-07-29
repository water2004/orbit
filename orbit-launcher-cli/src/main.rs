mod app;
mod cli;
mod output;
mod supervisor_ipc;

use std::io::{BufRead, IsTerminal, Write};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::{CommandFactory, FromArgMatches};
use cli::{Cli, OutputFormat, ProgressFormat};
use orbit_i18n::tr;
use orbit_launcher_core::{
    EulaDocument, InstallProgressEvent, LaunchOutputStream, LaunchPreparationEvent,
    LaunchProcessEvent, LauncherError, MicrosoftLoginProgressEvent, RepositoryMoveEvent,
    SupervisorEvent,
};
use output::{ErrorEnvelope, ProgressData, ProgressEnvelope, SuccessEnvelope};
use zeroize::Zeroizing;

#[tokio::main]
async fn main() -> ExitCode {
    let requested_language = orbit_i18n::requested_from_args(std::env::args_os());
    orbit_i18n::install(requested_language);
    let matches = orbit_i18n::get_matches(Cli::command());
    let cli = Cli::from_arg_matches(&matches).expect("Clap matches the derived CLI schema");
    orbit_i18n::install(cli.language);
    let command_name = command_name(&cli.command);
    let runtime =
        orbit_launcher_core::RuntimeContext::load(orbit_launcher_core::RuntimePathOptions {
            config_dir: cli.config_dir.clone(),
            data_dir: cli.data_dir.clone(),
            cache_dir: cli.cache_dir.clone(),
        });
    let runtime = match runtime {
        Ok(runtime) => runtime,
        Err(error) => {
            return render_launcher_error(cli.output_format, command_name, &error);
        }
    };
    let current_dir = match std::env::current_dir() {
        Ok(path) => path,
        Err(error) => {
            return render_launcher_error(
                cli.output_format,
                command_name,
                &LauncherError::Io(error),
            );
        }
    };
    let mut frontend =
        TerminalFrontend::new(cli.output_format, cli.progress_format, cli.non_interactive);
    match app::execute(
        cli.command,
        cli.instance.as_deref(),
        &current_dir,
        &runtime,
        &mut frontend,
    )
    .await
    {
        Ok(output) => {
            let process_succeeded = output.process_succeeded();
            render_success(cli.output_format, output);
            if process_succeeded {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(error) => render_app_error(cli.output_format, command_name, &error),
    }
}

fn command_name(command: &cli::Commands) -> &'static str {
    match command {
        cli::Commands::Install { .. } => "install",
        cli::Commands::Launch { .. } => "launch",
        cli::Commands::Config { command } => match command {
            cli::ConfigCommands::Path => "config.path",
            cli::ConfigCommands::List => "config.list",
            cli::ConfigCommands::Get { .. } => "config.get",
            cli::ConfigCommands::Set { .. } => "config.set",
            cli::ConfigCommands::Unset { .. } => "config.unset",
            cli::ConfigCommands::Yggdrasil { command } => match command {
                cli::YggdrasilProviderCommands::List => "config.yggdrasil.list",
                cli::YggdrasilProviderCommands::Add { .. } => "config.yggdrasil.add",
                cli::YggdrasilProviderCommands::Remove { .. } => "config.yggdrasil.remove",
            },
        },
        cli::Commands::Instance { command } => match command {
            cli::InstanceCommands::Create { .. } => "instance.create",
            cli::InstanceCommands::Import { .. } => "instance.import",
            cli::InstanceCommands::List => "instance.list",
            cli::InstanceCommands::Show => "instance.show",
            cli::InstanceCommands::Rename { .. } => "instance.rename",
            cli::InstanceCommands::Configure { .. } => "instance.configure",
            cli::InstanceCommands::Remove => "instance.remove",
            cli::InstanceCommands::Default { .. } => "instance.default",
        },
        cli::Commands::Server { command } => match command {
            cli::ServerCommands::Run { .. } => "server.run",
            cli::ServerCommands::Start => "server.start",
            cli::ServerCommands::Stop => "server.stop",
            cli::ServerCommands::Status => "server.status",
            cli::ServerCommands::Command { .. } => "server.command",
            cli::ServerCommands::Eula { command } => match command {
                cli::EulaCommands::Show => "server.eula.show",
                cli::EulaCommands::Accept { .. } => "server.eula.accept",
            },
        },
        cli::Commands::Account { command } => match command {
            cli::AccountCommands::Login { command } => match command {
                cli::AccountLoginCommands::Offline { .. } => "account.login.offline",
                cli::AccountLoginCommands::Yggdrasil { .. } => "account.login.yggdrasil",
                cli::AccountLoginCommands::Microsoft { command } => match command {
                    cli::MicrosoftLoginCommands::Begin => "account.login.microsoft.begin",
                    cli::MicrosoftLoginCommands::Complete { .. } => {
                        "account.login.microsoft.complete"
                    }
                },
            },
            cli::AccountCommands::List => "account.list",
            cli::AccountCommands::Show { .. } => "account.show",
            cli::AccountCommands::Refresh { .. } => "account.refresh",
            cli::AccountCommands::Select { .. } => "account.select",
            cli::AccountCommands::Clear { .. } => "account.clear",
            cli::AccountCommands::Logout { .. } => "account.logout",
        },
        cli::Commands::Versions { command } => match command {
            cli::VersionCommands::Minecraft => "versions.minecraft",
            cli::VersionCommands::Loader { .. } => "versions.loader",
            cli::VersionCommands::Java { .. } => "versions.java",
        },
        cli::Commands::Java { command } => match command {
            cli::JavaCommands::List { .. } => "java.list",
            cli::JavaCommands::Verify { .. } => "java.verify",
            cli::JavaCommands::Remove { .. } => "java.remove",
        },
        cli::Commands::Minecraft { command } => match command {
            cli::MinecraftCommands::Directory => "minecraft.directory",
            cli::MinecraftCommands::Move { .. } => "minecraft.move",
        },
        cli::Commands::Supervisor => "server.supervisor",
    }
}

fn render_success(format: OutputFormat, output: app::CommandOutput) {
    let command = output.command_name();
    match format {
        OutputFormat::Json => match output {
            app::CommandOutput::ConfigPath(value) => print_json(command, value),
            app::CommandOutput::ConfigList(value) => print_json(command, value),
            app::CommandOutput::ConfigEntry(value) => print_json(command, value),
            app::CommandOutput::ConfigMutation(value) => print_json(command, value),
            app::CommandOutput::EulaDocument(value) => print_json(command, value),
            app::CommandOutput::EulaAcceptance(value) => print_json(command, value),
            app::CommandOutput::Install(value) => print_json(command, value),
            app::CommandOutput::LaunchPlan(value) => print_json(command, value),
            app::CommandOutput::LaunchResult(value) => print_json(command, value),
            app::CommandOutput::ServerStart(value) => print_json(command, value),
            app::CommandOutput::ServerStatus(value) => print_json(command, value),
            app::CommandOutput::ServerControl(value) => print_json(command, value),
            app::CommandOutput::SupervisorResult(value) => print_json(command, value),
            app::CommandOutput::InstanceList(value) => print_json(command, value),
            app::CommandOutput::InstanceDetail(value) => print_json(command, value),
            app::CommandOutput::InstanceMutation(value) => print_json(command, value),
            app::CommandOutput::Rename(value) => print_json(command, value),
            app::CommandOutput::InstanceConfigured(value) => print_json(command, value),
            app::CommandOutput::Default(value) => print_json(command, value),
            app::CommandOutput::AccountList(value) => print_json(command, value),
            app::CommandOutput::AccountDetail(value) => print_json(command, value),
            app::CommandOutput::AccountRefresh(value) => print_json(command, value),
            app::CommandOutput::AccountLogin(value) => print_json(command, value),
            app::CommandOutput::AccountSelection(value) => print_json(command, value),
            app::CommandOutput::AccountLogout(value) => print_json(command, value),
            app::CommandOutput::MicrosoftDeviceSession(value) => print_json(command, value),
            app::CommandOutput::YggdrasilProviderList(value) => print_json(command, value),
            app::CommandOutput::YggdrasilProviderMutation(value) => print_json(command, value),
            app::CommandOutput::JavaRuntimeList(value) => print_json(command, value),
            app::CommandOutput::JavaRuntimeMutation(value) => print_json(command, value),
            app::CommandOutput::MinecraftVersions(value) => print_json(command, value),
            app::CommandOutput::LoaderVersions(value) => print_json(command, value),
            app::CommandOutput::JavaRequirement(value) => print_json(command, value),
            app::CommandOutput::MinecraftDirectory(value) => print_json(command, value),
            app::CommandOutput::MinecraftDirectoryMove(value) => print_json(command, value),
        },
        OutputFormat::Text => render_text(output),
    }
}

fn print_json<T: serde::Serialize>(command: &'static str, value: T) {
    let envelope = SuccessEnvelope::new(command, value);
    println!(
        "{}",
        serde_json::to_string(&envelope).expect("launcher output views are serializable")
    );
}

fn render_text(output: app::CommandOutput) {
    match output {
        app::CommandOutput::ConfigPath(view) => println!("{}", view.path.display()),
        app::CommandOutput::ConfigList(view) => {
            for setting in view.settings {
                render_config_entry(&setting);
            }
        }
        app::CommandOutput::ConfigEntry(view) => render_config_entry(&view),
        app::CommandOutput::ConfigMutation(view) => {
            let current = view.current.unwrap_or_else(|| tr!("<unset>").into_owned());
            let previous = view.previous.unwrap_or_else(|| tr!("<unset>").into_owned());
            let source = if view.explicit {
                tr!("explicit")
            } else {
                tr!("default")
            };
            println!(
                "{}",
                tr!(
                    "%{key} = %{current} (%{source}; was %{previous})",
                    key = view.key,
                    current = current,
                    source = source,
                    previous = previous
                )
            );
        }
        app::CommandOutput::EulaDocument(view) => {
            print!("{}", view.text);
            println!("\n{}", tr!("Official URL: %{url}", url = view.url));
            println!("SHA-256: {}", view.digest_sha256);
            println!(
                "{}",
                tr!(
                    "To accept this exact document, run: orbit-launcher --instance %{instance} server eula accept %{digest}",
                    instance = view.instance_id,
                    digest = view.digest_sha256
                )
            );
        }
        app::CommandOutput::EulaAcceptance(view) => println!(
            "{}",
            tr!(
                "Accepted Minecraft EULA %{digest} for instance %{instance}.",
                digest = view.digest_sha256,
                instance = view.instance_id
            )
        ),
        app::CommandOutput::Install(view) => {
            println!(
                "{}",
                tr!(
                    "Installed %{kind} instance %{id}.",
                    kind = tr!(&view.kind),
                    id = view.instance_id
                )
            );
            println!(
                "  {}",
                tr!("Minecraft: %{version}", version = view.minecraft_version)
            );
            println!("  {}", tr!("Loader: %{loader}", loader = view.loader));
            println!(
                "  {}",
                tr!(
                    "Java: %{version} (%{runtime})",
                    version = view.java_version,
                    runtime = view.java_runtime_id
                )
            );
            println!(
                "  {}",
                tr!(
                    "Artifacts: %{downloaded} downloaded, %{cached} cached",
                    downloaded = view.downloaded_artifacts,
                    cached = view.cached_artifacts
                )
            );
            if let Some(digest) = view.eula_digest_sha256 {
                println!("  EULA SHA-256: {digest}");
            }
        }
        app::CommandOutput::LaunchPlan(view) => {
            println!(
                "{}",
                tr!(
                    "Verified %{kind} launch plan for %{instance}.",
                    kind = tr!(&view.kind),
                    instance = view.instance_id
                )
            );
            println!(
                "  {}",
                tr!(
                    "Working directory: %{path}",
                    path = view.working_directory.display()
                )
            );
            println!(
                "  {}",
                tr!("Executable: %{path}", path = view.executable.display())
            );
            println!("  {}", tr!("Arguments (authentication redacted):"));
            for argument in view.arguments {
                println!("    {argument}");
            }
        }
        app::CommandOutput::LaunchResult(view) => {
            let status = if view.success {
                tr!("exited normally")
            } else {
                tr!("failed")
            };
            println!(
                "{}",
                tr!(
                    "%{kind} process %{pid} %{status} (exit %{exit}, %{elapsed} ms).",
                    kind = tr!(&view.kind),
                    pid = view.pid,
                    status = status,
                    exit = view
                        .exit_code
                        .map_or_else(|| tr!("signal").into_owned(), |code| code.to_string()),
                    elapsed = view.elapsed_milliseconds
                )
            );
        }
        app::CommandOutput::ServerStart(view) => {
            println!(
                "{}",
                tr!(
                    "Started server supervisor %{pid} for instance %{instance}.",
                    pid = view.state.supervisor_pid,
                    instance = view.state.instance_id
                )
            );
            println!(
                "  {}",
                tr!("State: %{state}", state = tr!(view.state.phase.as_str()))
            );
            println!(
                "  {}",
                tr!("Standard output: %{path}", path = view.stdout_log.display())
            );
            println!(
                "  {}",
                tr!("Standard error: %{path}", path = view.stderr_log.display())
            );
        }
        app::CommandOutput::ServerStatus(view) => match view.state {
            Some(state) => println!(
                "{}",
                tr!(
                    "Server supervisor %{pid} is %{state} (child %{child}, generation %{generation}, restarts %{restarts}).",
                    pid = state.supervisor_pid,
                    state = tr!(state.phase.as_str()),
                    child = state
                        .child_pid
                        .map_or_else(|| tr!("none").into_owned(), |pid| pid.to_string()),
                    generation = state.generation,
                    restarts = state.restarts
                )
            ),
            None => println!("{}", tr!("Server supervisor is not running.")),
        },
        app::CommandOutput::ServerControl(view) => println!(
            "{}",
            tr!(
                "Server %{action}: %{message} (accepted: %{accepted}, state: %{state}).",
                action = tr!(&view.action),
                message = view.message,
                accepted = tr!(if view.accepted { "yes" } else { "no" }),
                state = tr!(view.state.phase.as_str())
            )
        ),
        app::CommandOutput::SupervisorResult(view) => println!(
            "{}",
            tr!(
                "Server supervisor stopped after %{generations} generation(s) and %{restarts} restart(s); exit %{exit}, requested: %{requested}, restart limit reached: %{limited}.",
                generations = view.generations,
                restarts = view.restarts,
                exit = view
                    .final_exit_code
                    .map_or_else(|| tr!("signal").into_owned(), |code| code.to_string()),
                requested = tr!(if view.stopped_by_request { "yes" } else { "no" }),
                limited = tr!(if view.restart_limit_reached {
                    "yes"
                } else {
                    "no"
                })
            )
        ),
        app::CommandOutput::InstanceList(view) => {
            if view.instances.is_empty() {
                println!("{}", tr!("No launcher instances are registered."));
            } else {
                for instance in view.instances {
                    let default = if instance.is_default {
                        format!(" [{}]", tr!("default"))
                    } else {
                        String::new()
                    };
                    println!(
                        "{}  {}  {}  {}{}",
                        instance.id,
                        instance.name,
                        instance.kind,
                        instance.directory.display(),
                        default
                    );
                }
            }
        }
        app::CommandOutput::InstanceDetail(view) => {
            println!("{} ({})", view.instance.name, view.instance.id);
            println!(
                "  {}",
                tr!(
                    "Directory: %{path}",
                    path = view.instance.directory.display()
                )
            );
            println!(
                "  {}",
                tr!("Kind: %{kind}", kind = tr!(&view.instance.kind))
            );
            println!(
                "  {}",
                tr!("Context: %{context}", context = tr!(view.context.as_str()))
            );
            println!(
                "  {}",
                tr!("Minecraft: %{version}", version = view.desired.minecraft)
            );
            let loader_version = view
                .desired
                .loader_version
                .unwrap_or_else(|| tr!("n/a").into_owned());
            println!(
                "  {}",
                tr!(
                    "Loader: %{loader} %{version}",
                    loader = view.desired.loader,
                    version = loader_version
                )
            );
            println!(
                "  {}",
                tr!("Java: %{policy}", policy = tr!(&view.desired.java_policy))
            );
        }
        app::CommandOutput::InstanceMutation(view) => {
            println!(
                "{}",
                tr!(
                    "%{action} instance '%{name}' (%{id}) at %{path}",
                    action = tr!(view.action.as_str()),
                    name = view.instance.name,
                    id = view.instance.id,
                    path = view.instance.directory.display()
                )
            );
            if view.action == output::InstanceMutationAction::Removed {
                println!("{}", tr!("Instance files were preserved."));
            }
        }
        app::CommandOutput::Rename(view) => {
            println!(
                "{}",
                tr!(
                    "Renamed instance '%{old}' to '%{new}' (%{id}).",
                    old = view.old_name,
                    new = view.new_name,
                    id = view.id
                )
            );
        }
        app::CommandOutput::InstanceConfigured(view) => {
            println!(
                "{}",
                tr!(
                    "Updated desired runtime for %{instance}.",
                    instance = view.instance.name
                )
            );
            println!(
                "  {}",
                tr!("Minecraft: %{version}", version = view.desired.minecraft)
            );
            println!(
                "  {}",
                tr!(
                    "Loader: %{loader} %{version}",
                    loader = view.desired.loader,
                    version = view
                        .desired
                        .loader_version
                        .unwrap_or_else(|| tr!("managed").into_owned())
                )
            );
            println!(
                "  {}",
                tr!(
                    "Java policy: %{policy}",
                    policy = tr!(&view.desired.java_policy)
                )
            );
        }
        app::CommandOutput::Default(view) => match view.instance {
            Some(instance) => println!(
                "{}",
                tr!(
                    "Default instance: %{name} (%{id})",
                    name = instance.name,
                    id = instance.id
                )
            ),
            None => println!("{}", tr!("No default instance is configured.")),
        },
        app::CommandOutput::AccountList(view) => {
            if view.accounts.is_empty() {
                println!("{}", tr!("No launcher accounts are configured."));
            } else {
                for account in view.accounts {
                    render_account(&account);
                }
            }
        }
        app::CommandOutput::AccountDetail(view) => render_account(&view),
        app::CommandOutput::AccountRefresh(view) => {
            println!("{}", tr!("Refreshed account profile."));
            render_account(&view);
        }
        app::CommandOutput::AccountLogin(view) => render_account(&view.account),
        app::CommandOutput::AccountSelection(view) => match view.account {
            Some(account) => println!(
                "{}",
                tr!(
                    "Selected %{name} (%{id}) for %{scope} scope.",
                    name = account.profile_name,
                    id = account.id,
                    scope = tr!(view.scope)
                )
            ),
            None => println!(
                "{}",
                tr!(
                    "Cleared the %{scope} account selection.",
                    scope = tr!(view.scope)
                )
            ),
        },
        app::CommandOutput::AccountLogout(view) => println!(
            "{}",
            tr!(
                "Removed local session for %{name} (%{id}).",
                name = view.account.profile_name,
                id = view.account.id
            )
        ),
        app::CommandOutput::MicrosoftDeviceSession(view) => {
            println!("{}", tr!("Open: %{url}", url = view.verification_uri));
            println!("{}", tr!("Enter code: %{code}", code = view.user_code));
            println!(
                "{}",
                tr!("Login session: %{id}", id = view.login_session_id)
            );
            println!(
                "{}",
                tr!(
                    "Then run: orbit-launcher account login microsoft complete %{id}",
                    id = view.login_session_id
                )
            );
        }
        app::CommandOutput::YggdrasilProviderList(view) => {
            if view.providers.is_empty() {
                println!("{}", tr!("No External Yggdrasil providers are configured."));
            } else {
                for provider in view.providers {
                    let insecure = if provider.allow_insecure_http {
                        format!(" [{}]", tr!("insecure HTTP allowed"))
                    } else {
                        String::new()
                    };
                    println!("{}  {}{}", provider.id, provider.api_root, insecure);
                }
            }
        }
        app::CommandOutput::YggdrasilProviderMutation(view) => println!(
            "{}",
            tr!(
                "%{action} External Yggdrasil provider '%{id}' (%{url}).",
                action = tr!(&view.action),
                id = view.provider.id,
                url = view.provider.api_root
            )
        ),
        app::CommandOutput::JavaRuntimeList(view) => {
            if view.runtimes.is_empty() {
                println!("{}", tr!("No managed Java runtimes are installed."));
            }
            for runtime in view.runtimes {
                println!(
                    "{}",
                    tr!(
                        "%{id}: Java %{major} %{version} (%{provider}, %{platform}, %{files} files, %{bytes})%{verified}",
                        id = runtime.runtime_id,
                        major = runtime.major,
                        version = runtime.version,
                        provider = runtime.provider,
                        platform = runtime.platform,
                        files = runtime.files,
                        bytes = human_bytes(runtime.bytes),
                        verified = if runtime.verified == Some(true) {
                            format!(" [{}]", tr!("verified"))
                        } else {
                            String::new()
                        }
                    )
                );
                println!(
                    "  {}",
                    tr!("Executable: %{path}", path = runtime.executable.display())
                );
            }
        }
        app::CommandOutput::JavaRuntimeMutation(view) => println!(
            "{}",
            tr!(
                "%{action} managed Java runtime %{id} (Java %{major} %{version}).",
                action = tr!(if view.action == "verified" {
                    "Verified"
                } else {
                    "Removed"
                }),
                id = view.runtime.runtime_id,
                major = view.runtime.major,
                version = view.runtime.version
            )
        ),
        app::CommandOutput::MinecraftVersions(view) => {
            println!(
                "{}",
                tr!(
                    "Minecraft versions (latest release %{release}, latest snapshot %{snapshot}):",
                    release = view.latest_release,
                    snapshot = view.latest_snapshot
                )
            );
            for version in view.versions {
                let latest = if version.latest_release {
                    format!(" [{}]", tr!("latest release"))
                } else if version.latest_snapshot {
                    format!(" [{}]", tr!("latest snapshot"))
                } else {
                    String::new()
                };
                println!(
                    "{}  {}  {}{}",
                    version.id, version.version_type, version.release_time, latest
                );
            }
        }
        app::CommandOutput::LoaderVersions(view) => {
            println!(
                "{}",
                tr!(
                    "%{loader} versions compatible with Minecraft %{minecraft}:",
                    loader = view.loader,
                    minecraft = view.minecraft
                )
            );
            for version in view.versions {
                let mut tags = Vec::new();
                if version.recommended {
                    tags.push(tr!("recommended"));
                }
                if version.stable {
                    tags.push(tr!("stable"));
                }
                if version.latest {
                    tags.push(tr!("latest"));
                }
                let tags = if tags.is_empty() {
                    String::new()
                } else {
                    format!(
                        " [{}]",
                        tags.iter()
                            .map(AsRef::as_ref)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                let java = version
                    .minimum_java_major
                    .map(|major| format!(" · Java {major}+"))
                    .unwrap_or_default();
                println!("{}{}{}", version.version, tags, java);
            }
        }
        app::CommandOutput::JavaRequirement(view) => match (view.component, view.major) {
            (Some(component), Some(major)) => println!(
                "{}",
                tr!(
                    "Minecraft %{minecraft} requires Java %{major} (%{component}).",
                    minecraft = view.minecraft,
                    major = major,
                    component = component
                )
            ),
            _ => println!(
                "{}",
                tr!(
                    "Minecraft %{minecraft} publishes no managed Java requirement.",
                    minecraft = view.minecraft
                )
            ),
        },
        app::CommandOutput::MinecraftDirectory(view) => println!(
            "{}",
            tr!(
                "Managed Minecraft directory: %{path} (%{source})",
                path = view.directory.display(),
                source = tr!(if view.explicit { "explicit" } else { "default" })
            )
        ),
        app::CommandOutput::MinecraftDirectoryMove(view) => {
            println!(
                "{}",
                tr!(
                    "Moved managed Minecraft directory from %{previous} to %{current}.",
                    previous = view.previous.display(),
                    current = view.current.display()
                )
            );
            if !view.source_removed {
                println!(
                    "{}",
                    tr!(
                        "The verified destination is active, but the old directory could not be removed: %{path}",
                        path = view.previous.display()
                    )
                );
            }
        }
    }
}

fn render_account(view: &output::AccountView) {
    let default = if view.is_default {
        format!(" [{}]", tr!("default"))
    } else {
        String::new()
    };
    let provider = view
        .provider_id
        .as_ref()
        .map(|id| format!("{}:{id}", view.provider))
        .unwrap_or_else(|| view.provider.clone());
    println!(
        "{}  {}  {}  {}{}",
        view.id, view.profile_name, view.profile_id, provider, default
    );
}

fn render_config_entry(view: &output::ConfigEntryView) {
    let value = view
        .value
        .clone()
        .unwrap_or_else(|| tr!("<unset>").into_owned());
    let source = if view.explicit {
        tr!("explicit")
    } else {
        tr!("default")
    };
    println!(
        "{}",
        tr!(
            "%{key} = %{value} [%{source}]",
            key = view.key,
            value = value,
            source = source
        )
    );
}

fn render_error(format: OutputFormat, command: &str, code: &str, message: &str) -> ExitCode {
    match format {
        OutputFormat::Json => {
            let envelope = ErrorEnvelope::new(command, code, message);
            eprintln!(
                "{}",
                serde_json::to_string(&envelope).expect("launcher error view is serializable")
            );
        }
        OutputFormat::Text => eprintln!("{}: {message}", tr!("error")),
    }
    match code {
        "argument" => ExitCode::from(2),
        "interaction_required" | "reauthentication_required" | "eula_required" => ExitCode::from(4),
        _ => ExitCode::from(1),
    }
}

fn render_launcher_error(format: OutputFormat, command: &str, error: &LauncherError) -> ExitCode {
    let message = localized_launcher_error(error);
    render_error(format, command, error.code(), &message)
}

fn render_app_error(format: OutputFormat, command: &str, error: &app::AppError) -> ExitCode {
    match error {
        app::AppError::Core(error) => render_launcher_error(format, command, error),
        app::AppError::Argument(detail) => {
            let message = tr!("Invalid command usage: %{detail}", detail = detail);
            render_error(format, command, error.code(), &message)
        }
    }
}

fn localized_launcher_error(error: &LauncherError) -> String {
    match error {
        LauncherError::Io(detail) => {
            tr!("I/O operation failed: %{detail}", detail = detail).to_string()
        }
        LauncherError::Network(detail) => {
            tr!("Network operation failed: %{detail}", detail = detail).to_string()
        }
        LauncherError::InvalidRemoteData(detail) => tr!(
            "Remote service returned invalid data: %{detail}",
            detail = detail
        ),
        LauncherError::ArtifactIntegrity(detail) => tr!(
            "Artifact integrity check failed: %{detail}",
            detail = detail
        ),
        LauncherError::UnsupportedRequirement(detail) => tr!(
            "Unsupported launcher requirement: %{detail}",
            detail = detail
        ),
        LauncherError::LockParse(detail) => tr!(
            "Failed to parse orbit-launcher.lock: %{detail}",
            detail = detail
        ),
        LauncherError::InvalidLock(detail) => {
            tr!("Invalid orbit-launcher.lock: %{detail}", detail = detail)
        }
        LauncherError::EulaRequired(detail) => tr!(
            "Minecraft EULA confirmation is required: %{detail}",
            detail = detail
        ),
        LauncherError::InteractionRequired(detail) => {
            tr!("Interactive input is required: %{detail}", detail = detail)
        }
        LauncherError::SecretStore(detail) => {
            tr!(
                "Secure credential storage failed: %{detail}",
                detail = detail
            )
        }
        LauncherError::Authentication(detail) => {
            tr!("Account authentication failed: %{detail}", detail = detail)
        }
        LauncherError::ReauthenticationRequired { account_id, detail } => tr!(
            "Account %{account} must be signed in again: %{detail}",
            account = account_id,
            detail = detail
        ),
        LauncherError::Launch(detail) => {
            tr!("Launch preparation failed: %{detail}", detail = detail)
        }
        LauncherError::ConfigParse(detail) => tr!(
            "Failed to parse launcher config.toml: %{detail}",
            detail = detail
        ),
        LauncherError::ConfigDocumentParse(detail) => tr!(
            "Failed to edit launcher config.toml: %{detail}",
            detail = detail
        ),
        LauncherError::ManifestParse(detail) => tr!(
            "Failed to parse orbit-launcher.toml: %{detail}",
            detail = detail
        ),
        LauncherError::RegistryParse(detail) => {
            tr!("Failed to parse instances.toml: %{detail}", detail = detail)
        }
        LauncherError::TomlSerialize(detail) => {
            tr!("Failed to serialize TOML: %{detail}", detail = detail)
        }
        LauncherError::InvalidConfig(detail) => {
            tr!("Invalid launcher configuration: %{detail}", detail = detail)
        }
        LauncherError::InvalidManifest(detail) => {
            tr!("Invalid instance manifest: %{detail}", detail = detail)
        }
        LauncherError::InvalidRegistry(detail) => {
            tr!("Invalid instances registry: %{detail}", detail = detail)
        }
        LauncherError::ManifestNotFound(path) => tr!(
            "orbit-launcher.toml was not found in '%{path}'",
            path = path.display()
        ),
        LauncherError::InstanceNotFound(instance) => {
            tr!(
                "Instance '%{instance}' is not registered",
                instance = instance
            )
        }
        LauncherError::DuplicateInstanceName(name) => {
            tr!("Instance name '%{name}' is already registered", name = name)
        }
        LauncherError::DuplicateInstanceId(id) => tr!(
            "Instance ID '%{id}' is already registered at another path",
            id = id
        ),
        LauncherError::DuplicateInstancePath(path) => tr!(
            "Path '%{path}' is already registered to another instance",
            path = path.display()
        ),
        LauncherError::RelativeInstanceDirectory(path) => tr!(
            "Instance directory must be an absolute path: '%{path}'",
            path = path.display()
        ),
        LauncherError::InstancePathNotDirectory(path) => tr!(
            "Instance path is not a directory: '%{path}'",
            path = path.display()
        ),
        LauncherError::InstanceContextRequired => {
            tr!("Instance context is required; change to an instance directory or pass --instance")
                .into_owned()
        }
        LauncherError::ExplicitInstanceRequired(instance) => tr!(
            "Refusing to use default instance '%{instance}' for this operation; change to its directory or pass --instance",
            instance = instance
        ),
        LauncherError::InstanceRegistryMismatch(detail) => tr!(
            "Instance registry and manifest disagree: %{detail}",
            detail = detail
        ),
        LauncherError::Transaction(detail) => {
            tr!("Instance transaction failed: %{detail}", detail = detail)
        }
        LauncherError::JavaRuntimeNotFound(runtime) => tr!(
            "Managed Java runtime '%{runtime}' is not installed",
            runtime = runtime
        ),
        LauncherError::JavaRuntimeInUse {
            runtime_id,
            instances,
        } => tr!(
            "Managed Java runtime '%{runtime}' is still used by instances: %{instances}",
            runtime = runtime_id,
            instances = instances
        ),
        LauncherError::UnsupportedPlatform => tr!(
            "System data directories are unsupported on this platform; pass explicit directories"
        )
        .into_owned(),
    }
}

struct TerminalFrontend {
    output_format: OutputFormat,
    progress_format: ProgressFormat,
    non_interactive: bool,
    sequence: u64,
    last_text_progress: Instant,
    installer_output_lines: u64,
}

impl TerminalFrontend {
    fn new(
        output_format: OutputFormat,
        progress_format: ProgressFormat,
        non_interactive: bool,
    ) -> Self {
        Self {
            output_format,
            progress_format,
            non_interactive,
            sequence: 0,
            last_text_progress: Instant::now() - Duration::from_secs(2),
            installer_output_lines: 0,
        }
    }

    fn render_text_progress(&mut self, data: ProgressData) {
        match data {
            ProgressData::MetadataStarted => eprintln!("{}", tr!("Resolving Mojang metadata…")),
            ProgressData::MinecraftResolved { version, .. } => {
                eprintln!(
                    "{}",
                    tr!("Resolved Minecraft %{version}.", version = version)
                )
            }
            ProgressData::EulaChecked { accepted, .. } if accepted => {
                eprintln!("{}", tr!("Verified EULA acceptance."))
            }
            ProgressData::EulaChecked { .. } => {
                eprintln!("{}", tr!("Current EULA requires explicit acceptance."))
            }
            ProgressData::JavaManifestStarted => {
                eprintln!("{}", tr!("Resolving managed Java runtime…"))
            }
            ProgressData::JavaRuntimeResolved {
                runtime_id,
                artifacts,
                total_bytes,
            } => eprintln!(
                "{}",
                tr!(
                    "Resolved Java runtime %{id}: %{artifacts} files, %{bytes}.",
                    id = runtime_id,
                    artifacts = artifacts,
                    bytes = human_bytes(total_bytes)
                )
            ),
            ProgressData::ArtifactStarted {
                logical_name,
                total_bytes,
            } => eprintln!(
                "{}",
                tr!(
                    "Downloading %{name}%{size}…",
                    name = logical_name,
                    size = total_bytes
                        .map(|size| format!(" ({})", human_bytes(size)))
                        .unwrap_or_default()
                )
            ),
            ProgressData::ArtifactBytes {
                logical_name,
                downloaded_bytes,
                total_bytes,
            } if self.last_text_progress.elapsed() >= Duration::from_secs(1) => {
                self.last_text_progress = Instant::now();
                let total = total_bytes
                    .map(|size| format!(" / {}", human_bytes(size)))
                    .unwrap_or_default();
                eprintln!(
                    "{}",
                    tr!(
                        "Downloading %{name}: %{downloaded}%{total}",
                        name = logical_name,
                        downloaded = human_bytes(downloaded_bytes),
                        total = total
                    )
                );
            }
            ProgressData::ArtifactBytes { .. } => {}
            ProgressData::ArtifactCached { logical_name, .. } => {
                eprintln!("{}", tr!("Using cached %{name}.", name = logical_name))
            }
            ProgressData::ArtifactFinished { logical_name, .. } => {
                eprintln!("{}", tr!("Downloaded %{name}.", name = logical_name))
            }
            ProgressData::JavaMaterialized { completed, total }
                if completed == total || completed % 25 == 0 =>
            {
                eprintln!(
                    "{}",
                    tr!(
                        "Materializing Java runtime: %{completed}/%{total} files.",
                        completed = completed,
                        total = total
                    )
                )
            }
            ProgressData::JavaMaterialized { .. } => {}
            ProgressData::JavaRuntimeVerified { runtime_id } => {
                eprintln!("{}", tr!("Verified Java runtime %{id}.", id = runtime_id))
            }
            ProgressData::JavaRuntimeCached { runtime_id } => {
                eprintln!(
                    "{}",
                    tr!("Using installed Java runtime %{id}.", id = runtime_id)
                )
            }
            ProgressData::LoaderInstallerStarted {
                loader,
                version,
                side,
            } => {
                self.installer_output_lines = 0;
                eprintln!(
                    "{}",
                    tr!(
                        "Running official %{loader} %{version} installer for %{side}…",
                        loader = loader,
                        version = version,
                        side = tr!(&side)
                    )
                );
            }
            ProgressData::LoaderInstallerOutput { stream, line } => {
                self.installer_output_lines += 1;
                if self.installer_output_lines <= 20
                    || self.installer_output_lines.is_multiple_of(100)
                {
                    eprintln!("[{stream}] {line}")
                }
            }
            ProgressData::LoaderInstallerOutputSuppressed { maximum_lines } => eprintln!(
                "{}",
                tr!(
                    "Installer output exceeded %{maximum} lines; additional lines are suppressed.",
                    maximum = maximum_lines
                )
            ),
            ProgressData::LoaderInstallerFinished { loader, version } => {
                eprintln!(
                    "{}",
                    tr!(
                        "Official %{loader} %{version} installer completed.",
                        loader = loader,
                        version = version
                    )
                )
            }
            ProgressData::StagingVerified => {
                eprintln!("{}", tr!("Verified staged instance runtime."))
            }
            ProgressData::Committed => eprintln!("{}", tr!("Committed instance runtime.")),
            ProgressData::MicrosoftAuthorizationPolling { .. }
            | ProgressData::MicrosoftAuthorizationReceived
            | ProgressData::XboxAuthenticated
            | ProgressData::MinecraftAuthenticated
            | ProgressData::AccountSessionStored { .. } => {
                unreachable!("authentication progress is rendered by its own command")
            }
            ProgressData::LaunchArtifactVerified { completed, total }
                if completed == total || completed % 25 == 0 =>
            {
                eprintln!(
                    "{}",
                    tr!(
                        "Verifying installed runtime: %{completed}/%{total} artifacts.",
                        completed = completed,
                        total = total
                    )
                )
            }
            ProgressData::LaunchArtifactVerified { .. } => {}
            ProgressData::LaunchJavaVerified { runtime_id } => {
                eprintln!(
                    "{}",
                    tr!("Verified managed Java runtime %{id}.", id = runtime_id)
                )
            }
            ProgressData::LaunchNativesPrepared { files } => eprintln!(
                "{}",
                tr!("Prepared %{files} native runtime file(s).", files = files)
            ),
            ProgressData::LaunchPlanReady => eprintln!("{}", tr!("Launch plan is ready.")),
            ProgressData::RepositoryCopying { completed, total }
                if completed == total || completed % 100 == 0 =>
            {
                eprintln!(
                    "{}",
                    tr!(
                        "Copying Minecraft directory: %{completed}/%{total} files.",
                        completed = completed,
                        total = total
                    )
                )
            }
            ProgressData::RepositoryCopying { .. } => {}
            ProgressData::RepositoryVerifying { completed, total }
                if completed == total || completed % 100 == 0 =>
            {
                eprintln!(
                    "{}",
                    tr!(
                        "Verifying Minecraft directory: %{completed}/%{total} files.",
                        completed = completed,
                        total = total
                    )
                )
            }
            ProgressData::RepositoryVerifying { .. } => {}
            ProgressData::RepositorySwitching => {
                eprintln!("{}", tr!("Switching registered client instances."))
            }
            ProgressData::RepositoryRemovingSource => {
                eprintln!("{}", tr!("Removing the verified source directory."))
            }
            ProgressData::ProcessSpawned { pid } => {
                eprintln!("{}", tr!("Started Java process %{pid}.", pid = pid))
            }
            ProgressData::ProcessOutput { stream, line } => match (self.output_format, stream) {
                (OutputFormat::Text, LaunchOutputStream::Stdout) => println!("{line}"),
                _ => eprintln!("{line}"),
            },
            ProgressData::ProcessExited { exit_code, success } => eprintln!(
                "{}",
                tr!(
                    "Java process exited: %{exit} (success: %{success}).",
                    exit = exit_code
                        .map_or_else(|| tr!("signal").into_owned(), |code| code.to_string()),
                    success = tr!(if success { "yes" } else { "no" })
                )
            ),
            ProgressData::SupervisorSpawned { pid, generation } => {
                eprintln!(
                    "{}",
                    tr!(
                        "Started server process %{pid} (generation %{generation}).",
                        pid = pid,
                        generation = generation
                    )
                )
            }
            ProgressData::SupervisorCommandSent { command } => {
                eprintln!(
                    "{}",
                    tr!("Sent server command: %{command}", command = command)
                )
            }
            ProgressData::SupervisorStopRequested => {
                eprintln!("{}", tr!("Requested a graceful server stop."))
            }
            ProgressData::SupervisorExited {
                exit_code,
                success,
                expected,
                uptime_milliseconds,
            } => eprintln!(
                "{}",
                tr!(
                    "Server exited: %{exit} (success: %{success}, expected: %{expected}, uptime: %{uptime} ms).",
                    exit = exit_code
                        .map_or_else(|| tr!("signal").into_owned(), |code| code.to_string()),
                    success = tr!(if success { "yes" } else { "no" }),
                    expected = tr!(if expected { "yes" } else { "no" }),
                    uptime = uptime_milliseconds
                )
            ),
            ProgressData::SupervisorBackoff {
                delay_seconds,
                restart_attempt,
            } => {
                eprintln!(
                    "{}",
                    tr!(
                        "Restart attempt %{attempt} begins in %{seconds} second(s).",
                        attempt = restart_attempt,
                        seconds = delay_seconds
                    )
                )
            }
            ProgressData::SupervisorRestarting { generation } => {
                eprintln!(
                    "{}",
                    tr!(
                        "Starting server generation %{generation}.",
                        generation = generation
                    )
                )
            }
            ProgressData::SupervisorRestartLimitReached {
                attempts,
                window_seconds,
            } => eprintln!(
                "{}",
                tr!(
                    "Restart limit reached: %{attempts} restart(s) within %{seconds} seconds.",
                    attempts = attempts,
                    seconds = window_seconds
                )
            ),
            ProgressData::SupervisorStopped => {
                eprintln!("{}", tr!("Server supervisor stopped."))
            }
        }
    }
}

impl app::Frontend for TerminalFrontend {
    fn progress(&mut self, event: InstallProgressEvent) {
        let data = ProgressData::from(event);
        match self.progress_format {
            ProgressFormat::None => {}
            ProgressFormat::Text => self.render_text_progress(data),
            ProgressFormat::Ndjson => {
                self.sequence += 1;
                let phase = data.phase();
                let envelope = ProgressEnvelope::new("install", self.sequence, phase, data);
                eprintln!(
                    "{}",
                    serde_json::to_string(&envelope)
                        .expect("launcher progress views are serializable")
                );
            }
        }
    }

    fn confirm_eula(&mut self, document: &EulaDocument) -> Result<bool, LauncherError> {
        if self.non_interactive
            || self.output_format == OutputFormat::Json
            || !std::io::stdin().is_terminal()
            || !std::io::stderr().is_terminal()
        {
            return Err(LauncherError::InteractionRequired(tr!(
                "Minecraft EULA %{digest} must be accepted with 'server eula show' and 'server eula accept <digest>' before retrying install",
                digest = document.digest_sha256
            )));
        }
        let stderr = std::io::stderr();
        let mut stderr = stderr.lock();
        writeln!(
            stderr,
            "\n{}\n",
            tr!(
                "The complete current Minecraft EULA follows. Installation will not continue without explicit acceptance."
            )
        )?;
        write!(stderr, "{}", document.text)?;
        writeln!(
            stderr,
            "\n{}",
            tr!("Official URL: %{url}", url = document.url)
        )?;
        writeln!(stderr, "SHA-256: {}", document.digest_sha256)?;
        write!(
            stderr,
            "{}",
            tr!("Type I AGREE to accept this exact document: ")
        )?;
        stderr.flush()?;
        let mut input = String::new();
        std::io::stdin().lock().read_line(&mut input)?;
        Ok(input.trim() == "I AGREE")
    }

    fn read_password(
        &mut self,
        prompt: &str,
        stdin: bool,
    ) -> Result<Zeroizing<String>, LauncherError> {
        if stdin {
            let mut value = String::new();
            let bytes = std::io::stdin().lock().read_line(&mut value)?;
            if bytes == 0 {
                return Err(LauncherError::InteractionRequired(
                    tr!("--password-stdin was specified but stdin contained no password")
                        .into_owned(),
                ));
            }
            while value.ends_with(['\r', '\n']) {
                value.pop();
            }
            return Ok(Zeroizing::new(value));
        }
        if self.non_interactive
            || self.output_format == OutputFormat::Json
            || !std::io::stdin().is_terminal()
            || !std::io::stderr().is_terminal()
        {
            return Err(LauncherError::InteractionRequired(
                tr!("A Yggdrasil password must be read from a secure TTY or --password-stdin")
                    .into_owned(),
            ));
        }
        rpassword::prompt_password(prompt)
            .map(Zeroizing::new)
            .map_err(LauncherError::from)
    }

    fn microsoft_login_progress(&mut self, event: MicrosoftLoginProgressEvent) {
        let data = ProgressData::from_microsoft(event);
        match self.progress_format {
            ProgressFormat::None => {}
            ProgressFormat::Text => match data {
                ProgressData::MicrosoftAuthorizationPolling {
                    attempt,
                    elapsed_seconds,
                    ..
                } => eprintln!(
                    "{}",
                    tr!(
                        "Waiting for Microsoft authorization (attempt %{attempt}, %{seconds}s)…",
                        attempt = attempt,
                        seconds = elapsed_seconds
                    )
                ),
                ProgressData::MicrosoftAuthorizationReceived => {
                    eprintln!("{}", tr!("Microsoft authorization received."))
                }
                ProgressData::XboxAuthenticated => {
                    eprintln!("{}", tr!("Authenticated with Xbox Live."))
                }
                ProgressData::MinecraftAuthenticated => {
                    eprintln!("{}", tr!("Verified Minecraft ownership and profile."))
                }
                ProgressData::AccountSessionStored { .. } => {
                    eprintln!("{}", tr!("Stored the renewable account session securely."))
                }
                _ => unreachable!("Microsoft progress produced a non-authentication event"),
            },
            ProgressFormat::Ndjson => {
                self.sequence += 1;
                let phase = data.phase();
                let envelope = ProgressEnvelope::new(
                    "account.login.microsoft.complete",
                    self.sequence,
                    phase,
                    data,
                );
                eprintln!(
                    "{}",
                    serde_json::to_string(&envelope)
                        .expect("launcher progress views are serializable")
                );
            }
        }
    }

    fn launch_preparation(&mut self, command: &'static str, event: LaunchPreparationEvent) {
        self.render_launch_event(command, ProgressData::from_launch_preparation(event));
    }

    fn launch_process(&mut self, command: &'static str, event: LaunchProcessEvent) {
        self.render_launch_event(command, ProgressData::from_launch_process(event));
    }

    fn supervisor_event(&mut self, command: &'static str, event: SupervisorEvent) {
        self.render_launch_event(command, ProgressData::from_supervisor(event));
    }

    fn repository_move(&mut self, event: RepositoryMoveEvent) {
        self.render_launch_event("minecraft.move", ProgressData::from_repository(event));
    }
}

impl TerminalFrontend {
    fn render_launch_event(&mut self, command: &'static str, data: ProgressData) {
        match self.progress_format {
            ProgressFormat::None => {}
            ProgressFormat::Text => self.render_text_progress(data),
            ProgressFormat::Ndjson => {
                self.sequence += 1;
                let phase = data.phase();
                let envelope = ProgressEnvelope::new(command, self.sequence, phase, data);
                eprintln!(
                    "{}",
                    serde_json::to_string(&envelope)
                        .expect("launcher progress views are serializable")
                );
            }
        }
    }
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
