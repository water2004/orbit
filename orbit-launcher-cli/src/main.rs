mod app;
#[macro_use]
mod output_sink;
mod cli;
mod command_metadata;
mod interaction;
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
    StateArchiveProgressEvent, SupervisorEvent,
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
    let command_name = cli.command.command_name();
    let runtime =
        orbit_launcher_core::RuntimeContext::load(orbit_launcher_core::RuntimePathOptions {
            config_dir: cli.config_dir.clone(),
            data_dir: cli.data_dir.clone(),
            cache_dir: cli.cache_dir.clone(),
        });
    let mut runtime = match runtime {
        Ok(runtime) => runtime,
        Err(error) => {
            return render_launcher_error(cli.output_format, command_name, &error);
        }
    };
    if let Some(source) = cli.download_source {
        runtime.override_download_source(source);
    }
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
    let mut frontend = TerminalFrontend::new(
        cli.output_format,
        cli.progress_format,
        cli.non_interactive,
        command_name,
    );
    if matches!(
        cli.command,
        cli::Commands::Install { .. }
            | cli::Commands::Versions { .. }
            | cli::Commands::Instance {
                command: cli::InstanceCommands::Import { .. }
            }
    ) && let Some(attribution) = runtime.config().network.download_source.attribution()
    {
        frontend.download_source(attribution);
    }
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
            render_success(cli.output_format, command_name, output);
            if process_succeeded {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(error) => render_app_error(cli.output_format, command_name, &error),
    }
}

fn render_success(format: OutputFormat, command: &'static str, output: app::CommandOutput) {
    match format {
        OutputFormat::Json => match output {
            app::CommandOutput::UpdateCheck(value) => print_json(command, value),
            app::CommandOutput::ConfigPath(value) => print_json(command, value),
            app::CommandOutput::ConfigList(value) => print_json(command, value),
            app::CommandOutput::ConfigEntry(value) => print_json(command, value),
            app::CommandOutput::ConfigMutation(value) => print_json(command, value),
            app::CommandOutput::EulaDocument(value) => print_json(command, value),
            app::CommandOutput::EulaAcceptance(value) => print_json(command, value),
            app::CommandOutput::Install(value) => print_json(command, value),
            app::CommandOutput::LaunchPlan(value) => print_json(command, value),
            app::CommandOutput::LaunchResult(value) => print_json(command, value),
            app::CommandOutput::LaunchReport(value) => print_json(command, value),
            app::CommandOutput::ServerStart(value) => print_json(command, value),
            app::CommandOutput::ServerStatus(value) => print_json(command, value),
            app::CommandOutput::ServerControl(value) => print_json(command, value),
            app::CommandOutput::SupervisorResult(value) => print_json(command, value),
            app::CommandOutput::InstanceList(value) => print_json(command, value),
            app::CommandOutput::InstanceDetail(value) => print_json(command, value),
            app::CommandOutput::InstanceMutation(value) => print_json(command, value),
            app::CommandOutput::InstanceSources(value) => print_json(command, value),
            app::CommandOutput::InstanceImportPreview(value) => print_json(command, value),
            app::CommandOutput::InstanceImported(value) => print_json(command, value),
            app::CommandOutput::InstanceDeletion(value) => print_json(command, value),
            app::CommandOutput::Rename(value) => print_json(command, value),
            app::CommandOutput::InstanceConfigured(value) => print_json(command, value),
            app::CommandOutput::InstanceResolution(value) => print_json(command, value),
            app::CommandOutput::InstanceMemory(value) => print_json(command, value),
            app::CommandOutput::InstanceIcon(value) => print_json(command, value),
            app::CommandOutput::InstanceJava(value) => print_json(command, value),
            app::CommandOutput::LauncherStateExport(value) => print_json(command, value),
            app::CommandOutput::Default(value) => print_json(command, value),
            app::CommandOutput::AccountList(value) => print_json(command, value),
            app::CommandOutput::AccountDetail(value) => print_json(command, value),
            app::CommandOutput::AccountRefresh(value) => print_json(command, value),
            app::CommandOutput::AccountLogin(value) => print_json(command, value),
            app::CommandOutput::AccountSelection(value) => print_json(command, value),
            app::CommandOutput::AccountLogout(value) => print_json(command, value),
            app::CommandOutput::MicrosoftDeviceSession(value) => print_json(command, value),
            app::CommandOutput::YggdrasilProviderList(value) => print_json(command, value),
            app::CommandOutput::YggdrasilEndpoint(value) => print_json(command, value),
            app::CommandOutput::YggdrasilProviderMutation(value) => print_json(command, value),
            app::CommandOutput::JavaRuntimeList(value) => print_json(command, value),
            app::CommandOutput::JavaRuntimeMutation(value) => print_json(command, value),
            app::CommandOutput::MinecraftVersions(value) => print_json(command, value),
            app::CommandOutput::LoaderVersions(value) => print_json(command, value),
            app::CommandOutput::JavaRequirement(value) => print_json(command, value),
            app::CommandOutput::MinecraftDirectory(value) => print_json(command, value),
            app::CommandOutput::MinecraftDirectoryMove(value) => print_json(command, value),
            app::CommandOutput::PackageRequirement(value) => print_json(command, value),
        },
        OutputFormat::Text => render_text(output),
    }
}

fn print_json<T: serde::Serialize>(command: &'static str, value: T) {
    let envelope = SuccessEnvelope::new(command, value);
    outln!(
        "{}",
        serde_json::to_string(&envelope).expect("launcher output views are serializable")
    );
}

fn render_text(output: app::CommandOutput) {
    match output {
        app::CommandOutput::UpdateCheck(view) => {
            use orbit_launcher_core::updates::Status;
            let message = match view.status {
                Status::UpdateAvailable => tr!("A new Orbit release is available"),
                Status::UpToDate => tr!("Orbit is up to date"),
                Status::Ahead => tr!("This build is newer than the latest stable release"),
                Status::NoRelease => tr!("No published stable release is available"),
            };
            outln!("{message} · {}", view.current_version);
            if let Some(release) = view.release {
                outln!("{} · {}\n{}", release.name, release.version, release.url);
            }
        }
        app::CommandOutput::LaunchReport(record) => {
            if let Some(record) = record {
                outln!(
                    "{} · {}",
                    record.session_id,
                    record.pid.map_or_else(|| "—".into(), |pid| pid.to_string())
                );
                outln!(
                    "{}\n{}",
                    record.stdout_log.display(),
                    record.stderr_log.display()
                );
                if let Some(result) = record.result {
                    render_text(app::CommandOutput::LaunchResult(result.into()));
                }
            } else {
                outln!("{}", tr!("No launch record exists for this instance."));
            }
        }
        app::CommandOutput::ConfigPath(view) => outln!("{}", view.path.display()),
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
            outln!(
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
            outln!("\n{}", tr!("Official URL: %{url}", url = view.url));
            outln!("SHA-256: {}", view.digest_sha256);
            outln!(
                "{}",
                tr!(
                    "To accept this exact document, run: orbit-launcher --instance %{instance} server eula accept %{digest}",
                    instance = view.instance_id,
                    digest = view.digest_sha256
                )
            );
        }
        app::CommandOutput::EulaAcceptance(view) => outln!(
            "{}",
            tr!(
                "Accepted Minecraft EULA %{digest} for instance %{instance}.",
                digest = view.digest_sha256,
                instance = view.instance_id
            )
        ),
        app::CommandOutput::Install(view) => {
            outln!(
                "{}",
                tr!(
                    "Installed %{kind} instance %{id}.",
                    kind = tr!(&view.kind),
                    id = view.instance_id
                )
            );
            if let Some(state) = view.state {
                outln!(
                    "{}",
                    tr!(
                        "Applied %{kind} state from Minecraft %{source}: %{files} file(s), %{world_files} world file(s), %{properties} server property value(s).",
                        kind = tr!(&state.kind),
                        source = state.source_minecraft_version,
                        files = state.files,
                        world_files = state.world_files,
                        properties = state.restored_properties
                    )
                );
                if !state.skipped_properties.is_empty() {
                    outln!(
                        "{}",
                        tr!(
                            "Skipped server properties absent from the target version: %{properties}",
                            properties = state.skipped_properties.join(", ")
                        )
                    );
                }
            }
            outln!(
                "  {}",
                tr!("Minecraft: %{version}", version = view.minecraft_version)
            );
            outln!("  {}", tr!("Loader: %{loader}", loader = view.loader));
            outln!(
                "  {}",
                tr!(
                    "Java: %{version} (%{runtime})",
                    version = view.java_version,
                    runtime = view.java_runtime_id
                )
            );
            outln!(
                "  {}",
                tr!(
                    "Artifacts: %{downloaded} downloaded, %{cached} cached",
                    downloaded = view.downloaded_artifacts,
                    cached = view.cached_artifacts
                )
            );
            if !view.source_pinned_artifacts.is_empty() {
                outln!(
                    "  {}",
                    tr!(
                        "No upstream checksum was provided for these artifacts; their downloaded content is pinned locally by SHA-256: %{artifacts}",
                        artifacts = view.source_pinned_artifacts.join(", ")
                    )
                );
            }
            if let Some(digest) = view.eula_digest_sha256 {
                outln!("  EULA SHA-256: {digest}");
            }
        }
        app::CommandOutput::PackageRequirement(view) => {
            outln!("{} {} ({})", view.name, view.version, view.format);
            outln!(
                "  {}",
                tr!("Minecraft: %{version}", version = view.minecraft)
            );
            outln!(
                "  {}",
                tr!(
                    "Loader: %{loader}",
                    loader = view.loader_version.map_or(view.loader.clone(), |version| {
                        format!("{} {version}", view.loader)
                    })
                )
            );
            outln!(
                "  {}",
                tr!("Targets: %{targets}", targets = view.targets.join(", "))
            );
            outln!(
                "  {}",
                tr!(
                    "Optional files: %{count}",
                    count = view.optional_files.len()
                )
            );
        }
        app::CommandOutput::LaunchPlan(view) => {
            outln!(
                "{}",
                tr!(
                    "Verified %{kind} launch plan for %{instance}.",
                    kind = tr!(&view.kind),
                    instance = view.instance_id
                )
            );
            outln!(
                "  {}",
                tr!(
                    "Working directory: %{path}",
                    path = view.working_directory.display()
                )
            );
            outln!(
                "  {}",
                tr!("Executable: %{path}", path = view.executable.display())
            );
            outln!("  {}", tr!("Arguments (authentication redacted):"));
            for argument in view.arguments {
                outln!("    {argument}");
            }
        }
        app::CommandOutput::LaunchResult(view) => {
            let status = if view.success {
                tr!("exited normally")
            } else {
                tr!("failed")
            };
            outln!(
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
            if let Some(diagnostics) = &view.diagnostics {
                render_diagnostics(diagnostics);
            }
        }
        app::CommandOutput::ServerStart(view) => {
            outln!(
                "{}",
                tr!(
                    "Started server supervisor %{pid} for instance %{instance}.",
                    pid = view.state.supervisor_pid,
                    instance = view.state.instance_id
                )
            );
            outln!(
                "  {}",
                tr!("State: %{state}", state = tr!(view.state.phase.as_str()))
            );
            outln!(
                "  {}",
                tr!("Standard output: %{path}", path = view.stdout_log.display())
            );
            outln!(
                "  {}",
                tr!("Standard error: %{path}", path = view.stderr_log.display())
            );
        }
        app::CommandOutput::ServerStatus(view) => match view.state {
            Some(state) => outln!(
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
            None => outln!("{}", tr!("Server supervisor is not running.")),
        },
        app::CommandOutput::ServerControl(view) => outln!(
            "{}",
            tr!(
                "Server %{action}: %{message} (accepted: %{accepted}, state: %{state}).",
                action = tr!(&view.action),
                message = view.message,
                accepted = tr!(if view.accepted { "yes" } else { "no" }),
                state = tr!(view.state.phase.as_str())
            )
        ),
        app::CommandOutput::SupervisorResult(view) => {
            outln!(
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
            );
            if let Some(diagnostics) = &view.diagnostics {
                render_diagnostics(diagnostics);
            }
        }
        app::CommandOutput::InstanceList(view) => {
            if view.instances.is_empty() {
                outln!("{}", tr!("No launcher instances are registered."));
            } else {
                for instance in view.instances {
                    let default = if instance.is_default {
                        format!(" [{}]", tr!("default"))
                    } else {
                        String::new()
                    };
                    outln!(
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
            outln!("{} ({})", view.instance.name, view.instance.id);
            outln!(
                "  {}",
                tr!(
                    "Directory: %{path}",
                    path = view.instance.directory.display()
                )
            );
            outln!(
                "  {}",
                tr!("Kind: %{kind}", kind = tr!(&view.instance.kind))
            );
            outln!(
                "  {}",
                tr!("Context: %{context}", context = tr!(view.context.as_str()))
            );
            outln!(
                "  {}",
                tr!("Minecraft: %{version}", version = view.desired.minecraft)
            );
            let loader_version = view
                .desired
                .loader_version
                .unwrap_or_else(|| tr!("n/a").into_owned());
            outln!(
                "  {}",
                tr!(
                    "Loader: %{loader} %{version}",
                    loader = view.desired.loader,
                    version = loader_version
                )
            );
            if let Some(resolution) = view.client_resolution {
                outln!(
                    "  {}",
                    tr!(
                        "Client resolution: %{width}×%{height}",
                        width = resolution.width,
                        height = resolution.height
                    )
                );
            }
        }
        app::CommandOutput::InstanceSources(view) => {
            for source in view.instances {
                outln!("{} — {} ({})", source.id, source.name, source.format);
                if let Some(runtime) = source.runtime {
                    outln!(
                        "  Minecraft {} · {} {}",
                        runtime.minecraft,
                        runtime.loader.as_str(),
                        runtime.loader_version.as_deref().unwrap_or("")
                    );
                }
                outln!("  {}", source.directory_evidence);
                if let Some(path) = source.game_directory {
                    outln!("  {}", path.display());
                }
                if let Some(error) = source.error {
                    outln!("  {error}");
                }
            }
        }
        app::CommandOutput::InstanceImportPreview(view) => {
            outln!(
                "{} → {}",
                view.game_directory.display(),
                view.destination.display()
            );
            outln!(
                "{}",
                tr!(
                    "Copy %{files} files (%{bytes}). The source is preserved; accounts and launch hooks are excluded.",
                    files = view.files,
                    bytes = human_bytes(view.bytes)
                )
            );
            for path in view.excluded {
                outln!("  {}: {}", tr!("Excluded"), path.display());
            }
        }
        app::CommandOutput::InstanceImported(view) => {
            outln!(
                "{}",
                tr!(
                    "Imported '%{name}' to %{path}. The source was preserved.",
                    name = view.instance.name,
                    path = view.instance.directory.display()
                )
            );
            for warning in view.warnings {
                outln!("  {warning}");
            }
        }
        app::CommandOutput::InstanceDeletion(view) => {
            outln!(
                "{}",
                tr!(
                    "Delete the entire instance '%{name}' including worlds, mods and settings: %{path}",
                    name = view.name,
                    path = view.directory.display()
                )
            );
        }
        app::CommandOutput::InstanceMutation(view) => {
            outln!(
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
                outln!("{}", tr!("Instance files were preserved."));
            }
        }
        app::CommandOutput::Rename(view) => {
            outln!(
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
            outln!(
                "{}",
                tr!(
                    "Updated desired runtime for %{instance}.",
                    instance = view.instance.name
                )
            );
            outln!(
                "  {}",
                tr!("Minecraft: %{version}", version = view.desired.minecraft)
            );
            outln!(
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
        }
        app::CommandOutput::InstanceIcon(view) => outln!(
            "{}",
            tr!(
                "Updated icon for %{instance}.",
                instance = view.instance.name
            )
        ),
        app::CommandOutput::InstanceJava(view) => outln!(
            "{}",
            tr!(
                "Updated Java selection for %{instance}.",
                instance = view.instance.name
            )
        ),
        app::CommandOutput::InstanceMemory(view) => {
            outln!(
                "{}",
                tr!(
                    "Java heap for %{instance}: %{minimum}–%{maximum} MiB (next launch)",
                    instance = view.name,
                    minimum = view.memory.min_mib,
                    maximum = view.memory.max_mib
                )
            );
        }
        app::CommandOutput::InstanceResolution(view) => match view.client_resolution {
            Some(resolution) => outln!(
                "{}",
                tr!(
                    "Set client launch resolution for %{instance} to %{width}×%{height}.",
                    instance = view.instance.name,
                    width = resolution.width,
                    height = resolution.height
                )
            ),
            None => outln!(
                "{}",
                tr!(
                    "Cleared the explicit client launch resolution for %{instance}.",
                    instance = view.instance.name
                )
            ),
        },
        app::CommandOutput::LauncherStateExport(view) => outln!(
            "{}",
            tr!(
                "Exported %{kind} state for Minecraft %{version} to %{path}: %{files} file(s), %{world_files} world file(s), %{bytes}.",
                kind = tr!(&view.kind),
                version = view.minecraft_version,
                path = view.path.display(),
                files = view.files,
                world_files = view.world_files,
                bytes = human_bytes(view.bytes)
            )
        ),
        app::CommandOutput::Default(view) => match view.instance {
            Some(instance) => outln!(
                "{}",
                tr!(
                    "Default instance: %{name} (%{id})",
                    name = instance.name,
                    id = instance.id
                )
            ),
            None => outln!("{}", tr!("No default instance is configured.")),
        },
        app::CommandOutput::AccountList(view) => {
            if view.accounts.is_empty() {
                outln!("{}", tr!("No launcher accounts are configured."));
            } else {
                for account in view.accounts {
                    render_account(&account);
                }
            }
        }
        app::CommandOutput::AccountDetail(view) => render_account(&view),
        app::CommandOutput::AccountRefresh(view) => {
            outln!("{}", tr!("Refreshed account profile."));
            render_account(&view);
        }
        app::CommandOutput::AccountLogin(view) => render_account(&view.account),
        app::CommandOutput::AccountSelection(view) => {
            use orbit_launcher_core::account::{AccountSelectionSource, AccountSelectionSummary};
            match view.selection {
                AccountSelectionSummary::Selected { account, source } => outln!(
                    "{}",
                    tr!(
                        "Account for %{scope}: %{name} (%{id}), source: %{source}",
                        scope = tr!(view.scope),
                        name = account.profile_name,
                        id = account.id,
                        source = tr!(match source {
                            AccountSelectionSource::Instance => "instance",
                            AccountSelectionSource::Global => "global",
                        })
                    )
                ),
                AccountSelectionSummary::NotConfigured => outln!(
                    "{}",
                    tr!(
                        "No account is configured for %{scope}.",
                        scope = tr!(view.scope)
                    )
                ),
                AccountSelectionSummary::Unavailable { account_id, .. } => outln!(
                    "{}",
                    tr!("Configured account %{id} is unavailable.", id = account_id)
                ),
            }
        }
        app::CommandOutput::AccountLogout(view) => outln!(
            "{}",
            tr!(
                "Removed local session for %{name} (%{id}).",
                name = view.account.profile_name,
                id = view.account.id
            )
        ),
        app::CommandOutput::MicrosoftDeviceSession(view) => {
            outln!("{}", tr!("Open: %{url}", url = view.verification_uri));
            outln!("{}", tr!("Enter code: %{code}", code = view.user_code));
            outln!(
                "{}",
                tr!("Login session: %{id}", id = view.login_session_id)
            );
            outln!(
                "{}",
                tr!(
                    "Then run: orbit-launcher account login microsoft complete %{id}",
                    id = view.login_session_id
                )
            );
        }
        app::CommandOutput::YggdrasilEndpoint(view) => {
            outln!(
                "{}: {}",
                tr!("Name"),
                view.server_name.as_deref().unwrap_or(&view.suggested_id)
            );
            outln!("{}: {}", tr!("API root"), view.api_root);
            if view.insecure_http {
                outln!(
                    "{}",
                    tr!(
                        "HTTP sends passwords without encryption; only add this service if you accept the risk."
                    )
                );
            }
            if let Some(id) = view.existing_provider_id {
                outln!("{}: {}", tr!("Configured provider"), id);
            }
        }
        app::CommandOutput::YggdrasilProviderList(view) => {
            if view.providers.is_empty() {
                outln!("{}", tr!("No External Yggdrasil providers are configured."));
            } else {
                for provider in view.providers {
                    let insecure = if provider.allow_insecure_http {
                        format!(" [{}]", tr!("insecure HTTP allowed"))
                    } else {
                        String::new()
                    };
                    outln!("{}  {}{}", provider.id, provider.api_root, insecure);
                }
            }
        }
        app::CommandOutput::YggdrasilProviderMutation(view) => outln!(
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
                outln!("{}", tr!("No Java runtimes were found."));
            }
            for runtime in view.runtimes {
                outln!(
                    "{}",
                    tr!(
                        "%{id}: Java %{major} %{version} (%{provider}, %{platform}, %{files} files, %{bytes})%{verified}",
                        id = runtime.runtime_id,
                        major = runtime.major,
                        version = runtime.version,
                        provider = runtime.provider,
                        platform = runtime.platform,
                        files = runtime
                            .files
                            .map(|n| n.to_string())
                            .unwrap_or_else(|| "—".into()),
                        bytes = runtime.bytes.map(human_bytes).unwrap_or_else(|| "—".into()),
                        verified = if runtime.verified == Some(true) {
                            format!(" [{}]", tr!("verified"))
                        } else {
                            String::new()
                        }
                    )
                );
                outln!(
                    "  {}",
                    tr!("Executable: %{path}", path = runtime.executable.display())
                );
            }
        }
        app::CommandOutput::JavaRuntimeMutation(view) => outln!(
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
            outln!(
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
                outln!(
                    "{}  {}  {}{}",
                    version.id,
                    version.version_type,
                    version.release_time,
                    latest
                );
            }
        }
        app::CommandOutput::LoaderVersions(view) => {
            outln!(
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
                outln!("{}{}{}", version.version, tags, java);
            }
        }
        app::CommandOutput::JavaRequirement(view) => match (view.component, view.major) {
            (Some(component), Some(major)) => outln!(
                "{}",
                tr!(
                    "Minecraft %{minecraft} requires Java %{major} (%{component}).",
                    minecraft = view.minecraft,
                    major = major,
                    component = component
                )
            ),
            _ => outln!(
                "{}",
                tr!(
                    "Minecraft %{minecraft} publishes no managed Java requirement.",
                    minecraft = view.minecraft
                )
            ),
        },
        app::CommandOutput::MinecraftDirectory(view) => outln!(
            "{}",
            tr!(
                "Managed Minecraft directory: %{path} (%{source})",
                path = view.directory.display(),
                source = tr!(if view.explicit { "explicit" } else { "default" })
            )
        ),
        app::CommandOutput::MinecraftDirectoryMove(view) => {
            outln!(
                "{}",
                tr!(
                    "Moved managed Minecraft directory from %{previous} to %{current}.",
                    previous = view.previous.display(),
                    current = view.current.display()
                )
            );
            if !view.source_removed {
                outln!(
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
    outln!(
        "{}  {}  {}  {}{}",
        view.id,
        view.profile_name,
        view.profile_id,
        provider,
        default
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
    outln!(
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
            errln!(
                "{}",
                serde_json::to_string(&envelope).expect("launcher error view is serializable")
            );
        }
        OutputFormat::Text => errln!("{}: {message}", tr!("error")),
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
        LauncherError::PackFormat(detail) => {
            tr!("Invalid package archive: %{detail}", detail = detail)
        }
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
    command: &'static str,
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
        command: &'static str,
    ) -> Self {
        Self {
            command,
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
            ProgressData::DownloadSource { attribution } => {
                errln!("{} — {}", attribution.name, attribution.documentation)
            }
            ProgressData::InstanceImportCopying { completed, total } => {
                if completed == 0
                    || completed == total
                    || self.last_text_progress.elapsed() >= Duration::from_secs(1)
                {
                    self.last_text_progress = Instant::now();
                    errln!(
                        "{}",
                        tr!(
                            "Copying instance data: %{completed} / %{total}.",
                            completed = human_bytes(completed),
                            total = human_bytes(total)
                        )
                    );
                }
            }
            ProgressData::InstanceImportVerified => errln!(
                "{}",
                tr!("Imported instance data verified. Source preserved.")
            ),
            ProgressData::MetadataStarted => errln!("{}", tr!("Resolving Mojang metadata…")),
            ProgressData::MinecraftResolved { version, .. } => {
                errln!(
                    "{}",
                    tr!("Resolved Minecraft %{version}.", version = version)
                )
            }
            ProgressData::EulaChecked { accepted, .. } if accepted => {
                errln!("{}", tr!("Verified EULA acceptance."))
            }
            ProgressData::EulaChecked { .. } => {
                errln!("{}", tr!("Current EULA requires explicit acceptance."))
            }
            ProgressData::JavaManifestStarted => {
                errln!("{}", tr!("Resolving managed Java runtime…"))
            }
            ProgressData::JavaRuntimeResolved {
                runtime_id,
                artifacts,
                total_bytes,
            } => errln!(
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
            } => errln!(
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
                errln!(
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
                errln!("{}", tr!("Using cached %{name}.", name = logical_name))
            }
            ProgressData::ArtifactFinished { logical_name, .. } => {
                errln!("{}", tr!("Downloaded %{name}.", name = logical_name))
            }
            ProgressData::JavaMaterialized { completed, total }
                if completed == total || completed % 25 == 0 =>
            {
                errln!(
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
                errln!("{}", tr!("Verified Java runtime %{id}.", id = runtime_id))
            }
            ProgressData::JavaRuntimeCached { runtime_id } => {
                errln!(
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
                errln!(
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
                    errln!("[{stream}] {line}")
                }
            }
            ProgressData::LoaderInstallerOutputSuppressed { maximum_lines } => errln!(
                "{}",
                tr!(
                    "Installer output exceeded %{maximum} lines; additional lines are suppressed.",
                    maximum = maximum_lines
                )
            ),
            ProgressData::LoaderInstallerFinished { loader, version } => {
                errln!(
                    "{}",
                    tr!(
                        "Official %{loader} %{version} installer completed.",
                        loader = loader,
                        version = version
                    )
                )
            }
            ProgressData::ServerSettingsInitialized { properties } => errln!(
                "{}",
                tr!(
                    "Minecraft initialized %{properties} server setting(s).",
                    properties = properties
                )
            ),
            ProgressData::StateArchiveStarted { files, total, .. } => errln!(
                "{}",
                tr!(
                    "Preparing %{files} Launcher state file(s), %{bytes} total.",
                    files = files,
                    bytes = human_bytes(total)
                )
            ),
            ProgressData::StateArchiveAdvanced { completed, total }
                if self.last_text_progress.elapsed() >= Duration::from_secs(1) =>
            {
                self.last_text_progress = Instant::now();
                errln!(
                    "{}",
                    tr!(
                        "Processing Launcher state: %{completed} / %{total}.",
                        completed = human_bytes(completed),
                        total = human_bytes(total)
                    )
                );
            }
            ProgressData::StateArchiveAdvanced { .. } => {}
            ProgressData::StateArchiveFinished { files, total, .. } => errln!(
                "{}",
                tr!(
                    "Finished %{files} Launcher state file(s), %{bytes}.",
                    files = files,
                    bytes = human_bytes(total)
                )
            ),
            ProgressData::StagingVerified => {
                errln!("{}", tr!("Verified staged instance runtime."))
            }
            ProgressData::Committed => errln!("{}", tr!("Committed instance runtime.")),
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
                errln!(
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
                errln!(
                    "{}",
                    tr!("Verified managed Java runtime %{id}.", id = runtime_id)
                )
            }
            ProgressData::LaunchNativesPrepared { files } => errln!(
                "{}",
                tr!("Prepared %{files} native runtime file(s).", files = files)
            ),
            ProgressData::LaunchPlanReady => errln!("{}", tr!("Launch plan is ready.")),
            ProgressData::RepositoryCopying { completed, total }
                if completed == total || completed % 100 == 0 =>
            {
                errln!(
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
                errln!(
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
                errln!("{}", tr!("Switching registered client instances."))
            }
            ProgressData::RepositoryRemovingSource => {
                errln!("{}", tr!("Removing the verified source directory."))
            }
            ProgressData::ProcessSpawned { pid } => {
                errln!("{}", tr!("Started Java process %{pid}.", pid = pid))
            }
            ProgressData::ProcessOutput { stream, line } => match (self.output_format, stream) {
                (OutputFormat::Text, LaunchOutputStream::Stdout) => outln!("{line}"),
                _ => errln!("{line}"),
            },
            ProgressData::ProcessExited {
                exit_code, success, ..
            } => errln!(
                "{}",
                tr!(
                    "Java process exited: %{exit} (success: %{success}).",
                    exit = exit_code
                        .map_or_else(|| tr!("signal").into_owned(), |code| code.to_string()),
                    success = tr!(if success { "yes" } else { "no" })
                )
            ),
            ProgressData::SupervisorSpawned { pid, generation } => {
                errln!(
                    "{}",
                    tr!(
                        "Started server process %{pid} (generation %{generation}).",
                        pid = pid,
                        generation = generation
                    )
                )
            }
            ProgressData::SupervisorCommandSent { command } => {
                errln!(
                    "{}",
                    tr!("Sent server command: %{command}", command = command)
                )
            }
            ProgressData::SupervisorStopRequested => {
                errln!("{}", tr!("Requested a graceful server stop."))
            }
            ProgressData::SupervisorExited {
                exit_code,
                success,
                expected,
                uptime_milliseconds,
            } => errln!(
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
                errln!(
                    "{}",
                    tr!(
                        "Restart attempt %{attempt} begins in %{seconds} second(s).",
                        attempt = restart_attempt,
                        seconds = delay_seconds
                    )
                )
            }
            ProgressData::SupervisorRestarting { generation } => {
                errln!(
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
            } => errln!(
                "{}",
                tr!(
                    "Restart limit reached: %{attempts} restart(s) within %{seconds} seconds.",
                    attempts = attempts,
                    seconds = window_seconds
                )
            ),
            ProgressData::SupervisorStopped => {
                errln!("{}", tr!("Server supervisor stopped."))
            }
        }
    }
}

impl app::Frontend for TerminalFrontend {
    fn external_import_progress(&mut self, event: orbit_launcher_core::ExternalImportEvent) {
        let data = match event {
            orbit_launcher_core::ExternalImportEvent::Install(event) => ProgressData::from(event),
            orbit_launcher_core::ExternalImportEvent::Copying { completed, total } => {
                ProgressData::InstanceImportCopying { completed, total }
            }
            orbit_launcher_core::ExternalImportEvent::Verified => {
                ProgressData::InstanceImportVerified
            }
        };
        self.render_launch_event(data);
    }

    fn confirm_instance_deletion(
        &mut self,
        plan: &orbit_launcher_core::InstanceDeletionPlan,
    ) -> Result<bool, LauncherError> {
        if self.non_interactive
            || self.output_format == OutputFormat::Json
            || !std::io::stdin().is_terminal()
        {
            return Err(LauncherError::InteractionRequired(tr!("Review 'instance delete --dry-run', then pass --yes to delete the entire directory.").into_owned()));
        }
        errln!(
            "{}",
            tr!(
                "Delete the entire instance '%{name}' including worlds, mods and settings: %{path}",
                name = plan.name,
                path = plan.directory.display()
            )
        );
        errln!(
            "{}",
            tr!("The directory will be moved to the system trash. Type DELETE to continue:")
        );
        let mut answer = String::new();
        std::io::stdin().lock().read_line(&mut answer)?;
        Ok(answer.trim() == "DELETE")
    }
    fn progress(&mut self, event: InstallProgressEvent) {
        let data = ProgressData::from(event);
        match self.progress_format {
            ProgressFormat::None => {}
            ProgressFormat::Text => self.render_text_progress(data),
            ProgressFormat::Ndjson => {
                self.sequence += 1;
                let phase = data.phase();
                let envelope = ProgressEnvelope::new(self.command, self.sequence, phase, data);
                errln!(
                    "{}",
                    serde_json::to_string(&envelope)
                        .expect("launcher progress views are serializable")
                );
            }
        }
    }

    fn confirm_eula(&mut self, document: &EulaDocument) -> Result<bool, LauncherError> {
        if !self.non_interactive && self.output_format == OutputFormat::Json {
            self.sequence += 1;
            return interaction::confirm_eula(
                document,
                self.sequence,
                std::io::stdin().lock(),
                std::io::stderr().lock(),
            );
        }
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
                } => errln!(
                    "{}",
                    tr!(
                        "Waiting for Microsoft authorization (attempt %{attempt}, %{seconds}s)…",
                        attempt = attempt,
                        seconds = elapsed_seconds
                    )
                ),
                ProgressData::MicrosoftAuthorizationReceived => {
                    errln!("{}", tr!("Microsoft authorization received."))
                }
                ProgressData::XboxAuthenticated => {
                    errln!("{}", tr!("Authenticated with Xbox Live."))
                }
                ProgressData::MinecraftAuthenticated => {
                    errln!("{}", tr!("Verified Minecraft ownership and profile."))
                }
                ProgressData::AccountSessionStored { .. } => {
                    errln!("{}", tr!("Stored the renewable account session securely."))
                }
                _ => unreachable!("Microsoft progress produced a non-authentication event"),
            },
            ProgressFormat::Ndjson => {
                self.sequence += 1;
                let phase = data.phase();
                let envelope = ProgressEnvelope::new(self.command, self.sequence, phase, data);
                errln!(
                    "{}",
                    serde_json::to_string(&envelope)
                        .expect("launcher progress views are serializable")
                );
            }
        }
    }

    fn launch_preparation(&mut self, event: LaunchPreparationEvent) {
        self.render_launch_event(ProgressData::from_launch_preparation(event));
    }

    fn launch_process(&mut self, event: LaunchProcessEvent) {
        self.render_launch_event(ProgressData::from_launch_process(event));
    }

    fn supervisor_event(&mut self, event: SupervisorEvent) {
        self.render_launch_event(ProgressData::from_supervisor(event));
    }

    fn repository_move(&mut self, event: RepositoryMoveEvent) {
        self.render_launch_event(ProgressData::from_repository(event));
    }

    fn state_archive_progress(&mut self, event: StateArchiveProgressEvent) {
        let data = ProgressData::from_state_archive(event);
        match self.progress_format {
            ProgressFormat::None => {}
            ProgressFormat::Text => self.render_text_progress(data),
            ProgressFormat::Ndjson => {
                self.sequence += 1;
                let phase = if self.command == "export" {
                    orbit_machine_protocol::ProgressPhase::Export
                } else {
                    orbit_machine_protocol::ProgressPhase::Apply
                };
                let envelope = ProgressEnvelope::new(self.command, self.sequence, phase, data);
                errln!(
                    "{}",
                    serde_json::to_string(&envelope)
                        .expect("launcher state progress views are serializable")
                );
            }
        }
    }
}

impl TerminalFrontend {
    fn download_source(
        &mut self,
        attribution: orbit_launcher_core::download_source::DownloadAttribution,
    ) {
        if self.progress_format == ProgressFormat::None {
            // Attribution remains visible even when ordinary progress is disabled.
            errln!("{} — {}", attribution.name, attribution.documentation);
        } else {
            self.render_launch_event(ProgressData::DownloadSource { attribution });
        }
    }
    fn render_launch_event(&mut self, data: ProgressData) {
        match self.progress_format {
            ProgressFormat::None => {}
            ProgressFormat::Text => self.render_text_progress(data),
            ProgressFormat::Ndjson => {
                self.sequence += 1;
                let phase = data.phase();
                let envelope = ProgressEnvelope::new(self.command, self.sequence, phase, data);
                errln!(
                    "{}",
                    serde_json::to_string(&envelope)
                        .expect("launcher progress views are serializable")
                );
            }
        }
    }
}

fn render_diagnostics(diagnostics: &orbit_launcher_core::launch_diagnostics::LaunchDiagnostics) {
    if diagnostics.reports.is_empty() {
        outln!("{}", tr!("No new crash report was generated."));
    }
    for report in diagnostics
        .reports
        .iter()
        .chain(diagnostics.log_tail.iter())
    {
        outln!("\n{}\n{}", report.path.display(), report.text);
        if report.truncated {
            outln!(
                "{}",
                tr!("Preview truncated. Open the original file for the complete report.")
            );
        }
    }
    for warning in &diagnostics.warnings {
        errln!("{warning}");
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
