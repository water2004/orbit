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
    orbit_i18n::install(cli.language);
    let command = cli.command.command_name();
    let format = cli.format;
    // `--progress-format ndjson` opts into the structured stderr protocol.
    // `--quiet` always silences progress. Text-mode progress (spinner/bar) is
    // driven by `ui.progress_bar` config inside `operation_progress`.
    let progress = if !cli.quiet && cli.progress_format == ProgressFormat::Ndjson {
        ProgressFormat::Ndjson
    } else {
        ProgressFormat::None
    };
    let output = OutputCfg { format, progress };

    let runtime = match orbit_core::RuntimeContext::load(orbit_core::RuntimePathOptions {
        layout: cli.data_layout,
        config_file: cli.config.clone(),
        cache_dir: cli.cache_dir.clone(),
    }) {
        Ok(runtime) => runtime,
        Err(error) => {
            exit_with_error(&error.into(), output, command);
        }
    };
    let ctx = CliContext {
        command,
        machine_sequence: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        verbose: cli.verbose,
        quiet: cli.quiet,
        yes: cli.yes,
        dry_run: cli.dry_run,
        instance: cli.instance.clone(),
        runtime,
        output,
    };
    let command_result = cli.command.execute(&ctx).await;
    let cache_result = ctx.runtime.prune_jar_cache().map_err(anyhow::Error::from);
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
        ChecksumMismatch { .. } => "checksum_mismatch",
        ProviderApiKeyRequired { .. } => "provider_api_key_required",
        Io(_) => "io",
        Network(_) => "network",
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
        Json(detail) => tr!("Failed to parse JSON: %{detail}", detail = detail),
        Zip(detail) => tr!("Failed to process ZIP data: %{detail}", detail = detail),
        Other(detail) => tr!("Operation failed: %{detail}", detail = detail),
    }
}

fn exit_code_for(code: &str) -> i32 {
    match code {
        "argument" => 2,
        "cancelled" => 3,
        _ => 1,
    }
}
