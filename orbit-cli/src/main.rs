#[macro_use]
extern crate orbit_i18n;

mod cli;

use clap::{CommandFactory, FromArgMatches};
use cli::output::{OutputCfg, OutputFormat, ProgressFormat};
use cli::{
    Cli,
    commands::{CliContext, CommandHandler},
};

#[tokio::main]
async fn main() {
    let requested_language = orbit_i18n::requested_from_args(std::env::args_os());
    orbit_i18n::install(requested_language);
    let matches = orbit_i18n::get_matches(Cli::command());
    let cli = Cli::from_arg_matches(&matches).expect("Clap matches the derived CLI schema");
    let command = cli.command.command_name();
    let format = cli.output_format;
    // `--progress-format ndjson` opts into the structured stderr protocol.
    // `--quiet` always silences progress. Text-mode progress (spinner/bar) is
    // driven by `ui.progress_bar` config inside `operation_progress`.
    let progress = if !cli.quiet && cli.progress_format == ProgressFormat::Ndjson {
        ProgressFormat::Ndjson
    } else {
        ProgressFormat::None
    };
    let output = OutputCfg {
        format,
        progress,
        quiet: cli.quiet,
    };

    let runtime = match orbit_core::RuntimeContext::load(orbit_core::RuntimePathOptions {
        layout: cli.data_layout,
        config_file: cli.config.clone(),
        cache_dir: cli.cache_dir.clone(),
        repository_dir: cli.repository_dir.clone(),
    }) {
        Ok(runtime) => runtime,
        Err(error) => {
            exit_with_error(&error.into(), output, command);
        }
    };
    let configured_language = match runtime.config().core.language {
        orbit_core::LanguagePreference::System => orbit_i18n::LanguageMode::System,
        orbit_core::LanguagePreference::English => orbit_i18n::LanguageMode::English,
        orbit_core::LanguagePreference::SimplifiedChinese => {
            orbit_i18n::LanguageMode::SimplifiedChinese
        }
    };
    let language = cli.language.unwrap_or(configured_language);
    orbit_i18n::install(language);
    cli::output::install_color_mode(runtime.config().ui.color);
    let ctx = CliContext {
        command,
        machine_sequence: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        verbose: cli.verbose,
        quiet: cli.quiet,
        yes: cli.yes,
        dry_run: cli.dry_run,
        instance: cli.instance.clone(),
        language,
        runtime,
        output,
    };
    ctx.print_verbose_runtime();
    let command_result = cli.command.execute(&ctx).await;
    let cache_result = ctx.runtime.prune_jar_cache().map_err(anyhow::Error::from);
    if let Err(error) = &command_result
        && let Some(status) = forwarded_process_status(error)
    {
        // Orbit Launcher already emitted the canonical text/JSON failure. Do
        // not add a second machine error envelope around the forwarded result.
        std::process::exit(status);
    }
    match (command_result, cache_result) {
        (Ok(()), Ok(_)) => {}
        (Err(error), Ok(_)) | (Ok(()), Err(error)) => exit_with_error(&error, output, command),
        (Err(command_error), Err(cache_error)) => {
            let message = tr!(
                "%{command}; JAR cache LRU cleanup also failed: %{cache}",
                command = command_error,
                cache = cache_error
            );
            let combined = command_error.context(message);
            exit_with_error(&combined, output, command);
        }
    }
}

fn forwarded_process_status(error: &anyhow::Error) -> Option<i32> {
    error.chain().find_map(|cause| {
        cause
            .downcast_ref::<orbit_core::OrbitError>()
            .and_then(|error| match error {
                orbit_core::OrbitError::ForwardedProcessExit(status) => Some(*status),
                _ => None,
            })
    })
}

fn exit_with_error(error: &anyhow::Error, output: OutputCfg, command: &'static str) -> ! {
    let code = error_code(error);
    if output.format == OutputFormat::Json {
        let json = cli::output::ErrorJson::new(command, code, localized_error(error));
        eprintln!(
            "{}",
            serde_json::to_string(&json).expect("error envelope is serializable")
        );
    } else {
        eprintln!("{}: {}", tr!("error"), localized_error(error));
    }
    std::process::exit(exit_code_for(code));
}

/// Map an error to a stable string code used in JSON error output.
fn error_code(error: &anyhow::Error) -> &'static str {
    for cause in error.chain() {
        if let Some(orbit_error) = cause.downcast_ref::<orbit_core::OrbitError>() {
            return orbit_error_code(orbit_error);
        }
    }
    "internal"
}

fn orbit_error_code(error: &orbit_core::OrbitError) -> &'static str {
    use orbit_core::OrbitError::*;
    match error {
        ManifestNotFound => "manifest_not_found",
        ManifestParse(_) => "manifest_parse",
        ManifestSerialize(_) => "manifest_serialize",
        LockfileNotFound => "lockfile_not_found",
        ModNotFound(_) => "mod_not_found",
        VersionMismatch { .. } => "version_mismatch",
        Conflict(_) => "dependency_conflict",
        Cancelled(_) => "cancelled",
        ChecksumMismatch { .. } => "checksum_mismatch",
        ProviderApiKeyRequired { .. } => "provider_api_key_required",
        Io(_) => "io",
        Network(_) => "network",
        RuntimeData(_) => "runtime_data",
        ForwardedProcessExit(_) => "forwarded_process_exit",
        Json(_) => "json",
        Zip(_) => "zip",
        Other(_) => "internal",
    }
}

/// Human-facing message with secrets redacted. The checksum mismatch error
/// already omits hashes in its Display impl; we keep the redaction boundary
/// explicit here so future error variants cannot leak by accident.
fn localized_error(error: &anyhow::Error) -> String {
    for cause in error.chain() {
        if let Some(error) = cause.downcast_ref::<orbit_core::OrbitError>() {
            return localized_orbit_error(error);
        }
    }
    let detail = error
        .chain()
        .next()
        .map(ToString::to_string)
        .unwrap_or_else(|| tr!("unknown error").into_owned());
    tr!("Operation failed: %{detail}", detail = detail)
}

fn localized_orbit_error(error: &orbit_core::OrbitError) -> String {
    use orbit_core::OrbitError::*;
    match error {
        ManifestNotFound => tr!("orbit.toml was not found in the current instance").into_owned(),
        ManifestParse(detail) => tr!("Failed to parse orbit.toml: %{detail}", detail = detail),
        ManifestSerialize(detail) => {
            tr!("Failed to serialize orbit.toml: %{detail}", detail = detail)
        }
        LockfileNotFound => tr!("orbit.lock was not found in the current instance").into_owned(),
        ModNotFound(package) => tr!("Package '%{package}' was not found", package = package),
        VersionMismatch {
            mod_name,
            constraint,
        } => tr!(
            "No version of '%{package}' satisfies constraint '%{constraint}'",
            package = mod_name,
            constraint = constraint
        ),
        Conflict(detail) => tr!("Dependency conflict: %{detail}", detail = detail),
        Cancelled(detail) => tr!("Operation cancelled: %{detail}", detail = detail),
        ChecksumMismatch { name, .. } => tr!(
            "Content verification failed for '%{name}'; downloaded bytes differ from the trusted source",
            name = name
        ),
        ProviderApiKeyRequired {
            provider,
            environment_variable,
            config_key,
        } => tr!(
            "%{provider} requires an API key; set %{environment} or %{config} in config.toml",
            provider = provider,
            environment = environment_variable,
            config = config_key
        ),
        Io(detail) => tr!("I/O operation failed: %{detail}", detail = detail),
        Network(detail) => tr!("Network operation failed: %{detail}", detail = detail),
        RuntimeData(detail) => localized_runtime_data_error(detail),
        ForwardedProcessExit(status) => {
            tr!(
                "Orbit Launcher exited with status %{status}",
                status = status
            )
        }
        Json(detail) => tr!("Failed to parse JSON: %{detail}", detail = detail),
        Zip(detail) => tr!("Failed to process ZIP data: %{detail}", detail = detail),
        Other(detail) => tr!("Operation failed: %{detail}", detail = detail),
    }
}

fn localized_runtime_data_error(error: &orbit_core::RuntimeDataError) -> String {
    use orbit_core::RuntimeDataError::*;
    match error {
        NonUnicodeSnapshotName { path } => tr!(
            "Runtime observation snapshot has a non-Unicode name at '%{path}'",
            path = path
        ),
        InvalidObservation { path, line, detail } => tr!(
            "Invalid runtime observation at %{path}:%{line}: %{detail}",
            path = path,
            line = line,
            detail = detail
        ),
        PackageChanged { package } => tr!(
            "Package '%{package}' changed after the purge plan was created; request a new plan",
            package = package
        ),
        DeleteAfterPackageRemoval {
            package,
            path,
            completed,
            detail,
        } => tr!(
            "Package '%{package}' was removed, but deleting '%{path}' failed after %{completed} path(s): %{detail}",
            package = package,
            path = path,
            completed = completed,
            detail = detail
        ),
        LedgerParse { path, detail } => tr!(
            "Failed to parse runtime ownership ledger '%{path}': %{detail}",
            path = path,
            detail = detail
        ),
        UnsupportedLedgerSchema { schema, path } => tr!(
            "Unsupported runtime ownership schema %{schema} in '%{path}'",
            schema = schema,
            path = path
        ),
        LedgerSerialize { detail } => tr!(
            "Failed to serialize runtime ownership: %{detail}",
            detail = detail
        ),
        UnsafeRelativePath { path } => {
            tr!("Unsafe instance-relative data path '%{path}'", path = path)
        }
        InstanceRoot => tr!("Refusing to remove the instance root").into_owned(),
        ControlData => tr!("Refusing to remove Orbit control data").into_owned(),
        SharedInstanceRoot { path } => tr!(
            "Refusing to remove shared instance root '%{path}' as a tree",
            path = path
        ),
        UnsafeExternalPath { path } => tr!(
            "External path is not a safe absolute child path: '%{path}'",
            path = path
        ),
        ServerDryRun => tr!("Server joint launch does not support --dry-run").into_owned(),
        ComponentPathNotAbsolute { component, path } => tr!(
            "%{component} path must be absolute: '%{path}'",
            component = localized_runtime_component(*component),
            path = path
        ),
        ComponentNotFound { component, path } => tr!(
            "%{component} was not found at '%{path}'",
            component = localized_runtime_component(*component),
            path = path
        ),
        AgentPathContainsQuote => tr!("Orbit Runtime Agent path contains a quote").into_owned(),
        AgentAlreadyPresent => {
            tr!("JAVA_TOOL_OPTIONS already contains the Orbit Runtime Agent").into_owned()
        }
        UnsupportedRuntimeAgent { loader, version } => tr!(
            "Orbit Runtime Agent support has not been verified for %{loader} loader version '%{version}'",
            loader = loader,
            version = version
        ),
    }
}

fn localized_runtime_component(component: orbit_core::RuntimeComponent) -> String {
    match component {
        orbit_core::RuntimeComponent::Launcher => tr!("Orbit Launcher executable").into_owned(),
        orbit_core::RuntimeComponent::Agent => tr!("Orbit Runtime Agent").into_owned(),
    }
}

fn exit_code_for(code: &str) -> i32 {
    match code {
        "argument" => 2,
        "cancelled" => 3,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_cancellation_has_a_stable_code_and_exit_status() {
        let error = anyhow::Error::from(orbit_core::OrbitError::Cancelled(
            "user declined the transaction".to_string(),
        ));

        assert_eq!(error_code(&error), "cancelled");
        assert_eq!(exit_code_for(error_code(&error)), 3);
        assert!(localized_error(&error).contains("user declined the transaction"));
    }
}
