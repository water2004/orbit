mod app;
mod cli;
mod output;
mod supervisor_ipc;

use std::io::{BufRead, IsTerminal, Write};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::Parser;
use cli::{Cli, OutputFormat, ProgressFormat};
use orbit_launcher_core::{
    EulaDocument, InstallProgressEvent, LaunchOutputStream, LaunchPreparationEvent,
    LaunchProcessEvent, LauncherError, MicrosoftLoginProgressEvent, SupervisorEvent,
};
use output::{ErrorEnvelope, ProgressData, ProgressEnvelope, SuccessEnvelope};
use zeroize::Zeroizing;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
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
            return render_error(cli.format, command_name, error.code(), &error.to_string());
        }
    };
    let current_dir = match std::env::current_dir() {
        Ok(path) => path,
        Err(error) => return render_error(cli.format, command_name, "io", &error.to_string()),
    };
    let mut frontend = TerminalFrontend::new(cli.format, cli.progress_format, cli.non_interactive);
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
            render_success(cli.format, output);
            if process_succeeded {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(error) => render_error(cli.format, command_name, error.code(), &error.to_string()),
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
            cli::AccountCommands::Select { .. } => "account.select",
            cli::AccountCommands::Clear { .. } => "account.clear",
            cli::AccountCommands::Logout { .. } => "account.logout",
        },
        cli::Commands::Java { command } => match command {
            cli::JavaCommands::List { .. } => "java.list",
            cli::JavaCommands::Verify { .. } => "java.verify",
            cli::JavaCommands::Remove { .. } => "java.remove",
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
            app::CommandOutput::AccountLogin(value) => print_json(command, value),
            app::CommandOutput::AccountSelection(value) => print_json(command, value),
            app::CommandOutput::AccountLogout(value) => print_json(command, value),
            app::CommandOutput::MicrosoftDeviceSession(value) => print_json(command, value),
            app::CommandOutput::YggdrasilProviderList(value) => print_json(command, value),
            app::CommandOutput::YggdrasilProviderMutation(value) => print_json(command, value),
            app::CommandOutput::JavaRuntimeList(value) => print_json(command, value),
            app::CommandOutput::JavaRuntimeMutation(value) => print_json(command, value),
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
            let current = view.current.as_deref().unwrap_or("<unset>");
            let previous = view.previous.as_deref().unwrap_or("<unset>");
            let source = if view.explicit { "explicit" } else { "default" };
            println!("{} = {} ({source}; was {previous})", view.key, current);
        }
        app::CommandOutput::EulaDocument(view) => {
            print!("{}", view.text);
            println!("\nOfficial URL: {}", view.url);
            println!("SHA-256: {}", view.digest_sha256);
            println!(
                "To accept this exact document, run: orbit-launcher --instance {} server eula accept {}",
                view.instance_id, view.digest_sha256
            );
        }
        app::CommandOutput::EulaAcceptance(view) => println!(
            "Accepted Minecraft EULA {} for instance {}.",
            view.digest_sha256, view.instance_id
        ),
        app::CommandOutput::Install(view) => {
            println!("Installed {} instance {}.", view.kind, view.instance_id);
            println!("  Minecraft: {}", view.minecraft_version);
            println!("  loader: {}", view.loader);
            println!("  Java: {} ({})", view.java_version, view.java_runtime_id);
            println!(
                "  artifacts: {} downloaded, {} cached",
                view.downloaded_artifacts, view.cached_artifacts
            );
            if let Some(digest) = view.eula_digest_sha256 {
                println!("  EULA SHA-256: {digest}");
            }
        }
        app::CommandOutput::LaunchPlan(view) => {
            println!(
                "Verified {} launch plan for {}.",
                view.kind, view.instance_id
            );
            println!("  working directory: {}", view.working_directory.display());
            println!("  executable: {}", view.executable.display());
            println!("  arguments (authentication redacted):");
            for argument in view.arguments {
                println!("    {argument}");
            }
        }
        app::CommandOutput::LaunchResult(view) => {
            let status = if view.success {
                "exited normally"
            } else {
                "failed"
            };
            println!(
                "{} process {} {status} (exit {}, {} ms).",
                view.kind,
                view.pid,
                view.exit_code
                    .map_or_else(|| "signal".to_string(), |code| code.to_string()),
                view.elapsed_milliseconds
            );
        }
        app::CommandOutput::ServerStart(view) => {
            println!(
                "Started server supervisor {} for instance {}.",
                view.state.supervisor_pid, view.state.instance_id
            );
            println!("  state: {}", view.state.phase.as_str());
            println!("  stdout: {}", view.stdout_log.display());
            println!("  stderr: {}", view.stderr_log.display());
        }
        app::CommandOutput::ServerStatus(view) => match view.state {
            Some(state) => println!(
                "Server supervisor {} is {} (child {}, generation {}, restarts {}).",
                state.supervisor_pid,
                state.phase.as_str(),
                state
                    .child_pid
                    .map_or_else(|| "none".to_string(), |pid| pid.to_string()),
                state.generation,
                state.restarts
            ),
            None => println!("Server supervisor is not running."),
        },
        app::CommandOutput::ServerControl(view) => println!(
            "Server {}: {} (accepted: {}, state: {}).",
            view.action,
            view.message,
            view.accepted,
            view.state.phase.as_str()
        ),
        app::CommandOutput::SupervisorResult(view) => println!(
            "Server supervisor stopped after {} generation(s) and {} restart(s); exit {}, requested: {}, restart limit reached: {}.",
            view.generations,
            view.restarts,
            view.final_exit_code
                .map_or_else(|| "signal".to_string(), |code| code.to_string()),
            view.stopped_by_request,
            view.restart_limit_reached
        ),
        app::CommandOutput::InstanceList(view) => {
            if view.instances.is_empty() {
                println!("No launcher instances are registered.");
            } else {
                for instance in view.instances {
                    let default = if instance.is_default {
                        " [default]"
                    } else {
                        ""
                    };
                    println!(
                        "{}  {}  {}  {}{}",
                        instance.id,
                        instance.name,
                        instance.kind,
                        instance.root.display(),
                        default
                    );
                }
            }
        }
        app::CommandOutput::InstanceDetail(view) => {
            println!("{} ({})", view.instance.name, view.instance.id);
            println!("  root: {}", view.instance.root.display());
            println!("  kind: {}", view.instance.kind);
            println!("  context: {}", view.context.as_str());
            println!("  Minecraft: {}", view.desired.minecraft);
            let loader_version = view.desired.loader_version.as_deref().unwrap_or("n/a");
            println!("  loader: {} {}", view.desired.loader, loader_version);
            println!("  Java: {}", view.desired.java_policy);
        }
        app::CommandOutput::InstanceMutation(view) => {
            println!(
                "{} instance '{}' ({}) at {}",
                view.action.as_str(),
                view.instance.name,
                view.instance.id,
                view.instance.root.display()
            );
            if view.action == output::InstanceMutationAction::Removed {
                println!("Instance files were preserved.");
            }
        }
        app::CommandOutput::Rename(view) => {
            println!(
                "Renamed instance '{}' to '{}' ({}).",
                view.old_name, view.new_name, view.id
            );
        }
        app::CommandOutput::InstanceConfigured(view) => {
            println!("Updated desired runtime for {}.", view.instance.name);
            println!("  Minecraft: {}", view.desired.minecraft);
            println!(
                "  Loader: {} {}",
                view.desired.loader,
                view.desired.loader_version.as_deref().unwrap_or("managed")
            );
            println!("  Java policy: {}", view.desired.java_policy);
        }
        app::CommandOutput::Default(view) => match view.instance {
            Some(instance) => println!("Default instance: {} ({})", instance.name, instance.id),
            None => println!("No default instance is configured."),
        },
        app::CommandOutput::AccountList(view) => {
            if view.accounts.is_empty() {
                println!("No launcher accounts are configured.");
            } else {
                for account in view.accounts {
                    render_account(&account);
                }
            }
        }
        app::CommandOutput::AccountDetail(view) => render_account(&view),
        app::CommandOutput::AccountLogin(view) => render_account(&view.account),
        app::CommandOutput::AccountSelection(view) => match view.account {
            Some(account) => println!(
                "Selected {} ({}) for {} scope.",
                account.profile_name, account.id, view.scope
            ),
            None => println!("Cleared the {} account selection.", view.scope),
        },
        app::CommandOutput::AccountLogout(view) => println!(
            "Removed local session for {} ({}).",
            view.account.profile_name, view.account.id
        ),
        app::CommandOutput::MicrosoftDeviceSession(view) => {
            println!("Open: {}", view.verification_uri);
            println!("Enter code: {}", view.user_code);
            println!("Login session: {}", view.login_session_id);
            println!(
                "Then run: orbit-launcher account login microsoft complete {}",
                view.login_session_id
            );
        }
        app::CommandOutput::YggdrasilProviderList(view) => {
            if view.providers.is_empty() {
                println!("No External Yggdrasil providers are configured.");
            } else {
                for provider in view.providers {
                    let insecure = if provider.allow_insecure_http {
                        " [insecure HTTP allowed]"
                    } else {
                        ""
                    };
                    println!("{}  {}{}", provider.id, provider.api_root, insecure);
                }
            }
        }
        app::CommandOutput::YggdrasilProviderMutation(view) => println!(
            "{} External Yggdrasil provider '{}' ({}).",
            view.action, view.provider.id, view.provider.api_root
        ),
        app::CommandOutput::JavaRuntimeList(view) => {
            if view.runtimes.is_empty() {
                println!("No managed Java runtimes are installed.");
            }
            for runtime in view.runtimes {
                println!(
                    "{}: Java {} {} ({}, {}, {} files, {}){}",
                    runtime.runtime_id,
                    runtime.major,
                    runtime.version,
                    runtime.provider,
                    runtime.platform,
                    runtime.files,
                    human_bytes(runtime.bytes),
                    if runtime.verified == Some(true) {
                        " [verified]"
                    } else {
                        ""
                    }
                );
                println!("  executable: {}", runtime.executable.display());
            }
        }
        app::CommandOutput::JavaRuntimeMutation(view) => println!(
            "{} managed Java runtime {} (Java {} {}).",
            if view.action == "verified" {
                "Verified"
            } else {
                "Removed"
            },
            view.runtime.runtime_id,
            view.runtime.major,
            view.runtime.version
        ),
    }
}

fn render_account(view: &output::AccountView) {
    let default = if view.is_default { " [default]" } else { "" };
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
    let value = view.value.as_deref().unwrap_or("<unset>");
    let source = if view.explicit { "explicit" } else { "default" };
    println!("{} = {} [{source}]", view.key, value);
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
        OutputFormat::Text => eprintln!("error: {message}"),
    }
    match code {
        "argument" => ExitCode::from(2),
        "interaction_required" | "eula_required" => ExitCode::from(4),
        _ => ExitCode::from(1),
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
            ProgressData::MetadataStarted => eprintln!("Resolving Mojang metadata..."),
            ProgressData::MinecraftResolved { version, .. } => {
                eprintln!("Resolved Minecraft {version}.")
            }
            ProgressData::EulaChecked { accepted, .. } if accepted => {
                eprintln!("Verified EULA acceptance.")
            }
            ProgressData::EulaChecked { .. } => {
                eprintln!("Current EULA requires explicit acceptance.")
            }
            ProgressData::JavaManifestStarted => eprintln!("Resolving managed Java runtime..."),
            ProgressData::JavaRuntimeResolved {
                runtime_id,
                artifacts,
                total_bytes,
            } => eprintln!(
                "Resolved Java runtime {runtime_id}: {artifacts} files, {}.",
                human_bytes(total_bytes)
            ),
            ProgressData::ArtifactStarted {
                logical_name,
                total_bytes,
            } => eprintln!(
                "Downloading {logical_name}{}...",
                total_bytes
                    .map(|size| format!(" ({})", human_bytes(size)))
                    .unwrap_or_default()
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
                    "Downloading {logical_name}: {}{total}",
                    human_bytes(downloaded_bytes)
                );
            }
            ProgressData::ArtifactBytes { .. } => {}
            ProgressData::ArtifactCached { logical_name, .. } => {
                eprintln!("Using cached {logical_name}.")
            }
            ProgressData::ArtifactFinished { logical_name, .. } => {
                eprintln!("Downloaded {logical_name}.")
            }
            ProgressData::JavaMaterialized { completed, total }
                if completed == total || completed % 25 == 0 =>
            {
                eprintln!("Materializing Java runtime: {completed}/{total} files.")
            }
            ProgressData::JavaMaterialized { .. } => {}
            ProgressData::JavaRuntimeVerified { runtime_id } => {
                eprintln!("Verified Java runtime {runtime_id}.")
            }
            ProgressData::JavaRuntimeCached { runtime_id } => {
                eprintln!("Using installed Java runtime {runtime_id}.")
            }
            ProgressData::LoaderInstallerStarted {
                loader,
                version,
                side,
            } => {
                self.installer_output_lines = 0;
                eprintln!("Running official {loader} {version} installer for {side}...");
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
                "Installer output exceeded {maximum_lines} lines; additional lines are suppressed."
            ),
            ProgressData::LoaderInstallerFinished { loader, version } => {
                eprintln!("Official {loader} {version} installer completed.")
            }
            ProgressData::StagingVerified => eprintln!("Verified staged instance runtime."),
            ProgressData::Committed => eprintln!("Committed instance runtime."),
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
                eprintln!("Verifying installed runtime: {completed}/{total} artifacts.")
            }
            ProgressData::LaunchArtifactVerified { .. } => {}
            ProgressData::LaunchJavaVerified { runtime_id } => {
                eprintln!("Verified managed Java runtime {runtime_id}.")
            }
            ProgressData::LaunchPlanReady => eprintln!("Launch plan is ready."),
            ProgressData::ProcessSpawned { pid } => eprintln!("Started Java process {pid}."),
            ProgressData::ProcessOutput { stream, line } => match (self.output_format, stream) {
                (OutputFormat::Text, LaunchOutputStream::Stdout) => println!("{line}"),
                _ => eprintln!("{line}"),
            },
            ProgressData::ProcessExited { exit_code, success } => eprintln!(
                "Java process exited: {} (success: {success}).",
                exit_code.map_or_else(|| "signal".to_string(), |code| code.to_string())
            ),
            ProgressData::SupervisorSpawned { pid, generation } => {
                eprintln!("Started server process {pid} (generation {generation}).")
            }
            ProgressData::SupervisorCommandSent { command } => {
                eprintln!("Sent server command: {command}")
            }
            ProgressData::SupervisorStopRequested => {
                eprintln!("Requested a graceful server stop.")
            }
            ProgressData::SupervisorExited {
                exit_code,
                success,
                expected,
                uptime_milliseconds,
            } => eprintln!(
                "Server exited: {} (success: {success}, expected: {expected}, uptime: {uptime_milliseconds} ms).",
                exit_code.map_or_else(|| "signal".to_string(), |code| code.to_string())
            ),
            ProgressData::SupervisorBackoff {
                delay_seconds,
                restart_attempt,
            } => {
                eprintln!("Restart attempt {restart_attempt} begins in {delay_seconds} second(s).")
            }
            ProgressData::SupervisorRestarting { generation } => {
                eprintln!("Starting server generation {generation}.")
            }
            ProgressData::SupervisorRestartLimitReached {
                attempts,
                window_seconds,
            } => eprintln!(
                "Restart limit reached: {attempts} restart(s) within {window_seconds} seconds."
            ),
            ProgressData::SupervisorStopped => eprintln!("Server supervisor stopped."),
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
            return Err(LauncherError::InteractionRequired(format!(
                "Minecraft EULA {} must be accepted with 'server eula show' and 'server eula accept <digest>' before retrying install",
                document.digest_sha256
            )));
        }
        let stderr = std::io::stderr();
        let mut stderr = stderr.lock();
        writeln!(
            stderr,
            "\nThe complete current Minecraft EULA follows. Installation will not continue without explicit acceptance.\n"
        )?;
        write!(stderr, "{}", document.text)?;
        writeln!(stderr, "\nOfficial URL: {}", document.url)?;
        writeln!(stderr, "SHA-256: {}", document.digest_sha256)?;
        write!(stderr, "Type I AGREE to accept this exact document: ")?;
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
                    "--password-stdin was specified but stdin contained no password".to_string(),
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
                "a Yggdrasil password must be read from a secure TTY or --password-stdin"
                    .to_string(),
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
                    "Waiting for Microsoft authorization (attempt {attempt}, {elapsed_seconds}s)..."
                ),
                ProgressData::MicrosoftAuthorizationReceived => {
                    eprintln!("Microsoft authorization received.")
                }
                ProgressData::XboxAuthenticated => eprintln!("Authenticated with Xbox Live."),
                ProgressData::MinecraftAuthenticated => {
                    eprintln!("Verified Minecraft ownership and profile.")
                }
                ProgressData::AccountSessionStored { .. } => {
                    eprintln!("Stored the renewable account session securely.")
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
