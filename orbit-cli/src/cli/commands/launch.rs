use std::path::PathBuf;

use anyhow::{Context, Result};

use super::CliContext;

pub async fn handle(
    launcher: Option<PathBuf>,
    runtime_agent: Option<PathBuf>,
    launcher_instance: Option<String>,
    server: bool,
    ctx: &CliContext,
) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;
    let launcher_program = resolve_adjacent_component(
        launcher,
        if cfg!(windows) {
            "orbit-launcher.exe"
        } else {
            "orbit-launcher"
        },
    )?;
    let runtime_agent = resolve_runtime_agent(runtime_agent)?;
    let request = orbit_core::RuntimeLaunchRequest {
        instance_dir,
        launcher_program,
        runtime_agent,
        launcher_instance,
        target: if server {
            orbit_core::RuntimeLaunchTarget::Server
        } else {
            orbit_core::RuntimeLaunchTarget::Client
        },
        language: ctx.language.argument().to_string(),
        output_format: match ctx.output.format {
            crate::cli::output::OutputFormat::Text => "text",
            crate::cli::output::OutputFormat::Json => "json",
        }
        .to_string(),
        progress_format: if ctx.output.ndjson_progress() {
            "ndjson"
        } else if ctx.quiet {
            "none"
        } else {
            "text"
        }
        .to_string(),
        non_interactive: ctx.output.format == crate::cli::output::OutputFormat::Json,
        dry_run: ctx.dry_run,
    };
    orbit_core::launch_with_runtime_observation(&request)?;
    Ok(())
}

fn resolve_runtime_agent(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return absolutize(path);
    }
    #[cfg(all(target_os = "linux", not(feature = "portable")))]
    {
        return Ok(PathBuf::from("/usr/lib/orbit/orbit-runtime-agent.jar"));
    }
    #[cfg(not(all(target_os = "linux", not(feature = "portable"))))]
    resolve_adjacent_component(None, "orbit-runtime-agent.jar")
}

fn resolve_adjacent_component(explicit: Option<PathBuf>, filename: &str) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return absolutize(path);
    }
    let executable = std::env::current_exe().with_context(|| {
        tr!(
            "Failed to locate the Orbit executable while resolving %{file}",
            file = filename
        )
    })?;
    let directory = executable.parent().ok_or_else(|| {
        anyhow::anyhow!("{}", tr!("Orbit executable path has no parent directory"))
    })?;
    Ok(directory.join(filename))
}

fn absolutize(path: PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path);
    }
    Ok(std::env::current_dir()
        .with_context(|| tr!("Failed to get the current directory").into_owned())?
        .join(path))
}
