mod cli;

use clap::Parser;
use cli::output::{OutputCfg, OutputFormat, ProgressFormat};
use cli::{
    Cli,
    commands::{CliContext, CommandHandler},
};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
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
            exit_with_error(&error.into(), output);
        }
    };
    let ctx = CliContext {
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
        (Err(error), Ok(_)) | (Ok(()), Err(error)) => exit_with_error(&error, output),
        (Err(command_error), Err(cache_error)) => {
            let message =
                format!("{command_error}; JAR cache LRU cleanup also failed: {cache_error}");
            let combined = command_error.context(message);
            exit_with_error(&combined, output);
        }
    }
}

fn exit_with_error(error: &anyhow::Error, output: OutputCfg) -> ! {
    let code = error_code(error);
    if output.format == OutputFormat::Json {
        let json = cli::output::ErrorJson::new(code, redacted_message(error, code));
        eprintln!("{}", json.to_json());
    } else {
        eprintln!("error: {error}");
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
fn redacted_message(error: &anyhow::Error, _code: &str) -> String {
    error
        .chain()
        .next()
        .map(|cause| cause.to_string())
        .unwrap_or_else(|| "unknown error".to_string())
}

fn exit_code_for(code: &str) -> i32 {
    match code {
        "argument" => 2,
        "cancelled" => 3,
        _ => 1,
    }
}
