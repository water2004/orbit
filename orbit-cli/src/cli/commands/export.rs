use super::CliContext;
use anyhow::Result;

use crate::cli::output::{ExportOutput, OutputFormat};

pub async fn handle(
    file: Option<String>,
    target: Option<String>,
    format: String,
    ctx: &CliContext,
) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;
    let output = match file {
        Some(file) => std::path::PathBuf::from(file),
        None => {
            let manifest = orbit_core::ManifestFile::open(&instance_dir)?;
            let version = manifest.inner.project.version.as_deref().unwrap_or("1.0.0");
            let extension = if format == "mrpack" { "mrpack" } else { "zip" };
            std::path::PathBuf::from(format!(
                "{}-{version}.{extension}",
                safe_filename(&manifest.inner.project.name)
            ))
        }
    };
    let report = orbit_core::export_instance(
        &instance_dir,
        &output,
        target,
        &format,
        ctx.dry_run,
        super::operation_progress(ctx),
    )?;
    match ctx.output.format {
        OutputFormat::Text => {
            println!(
                "{}",
                tr!(
                    "Export %{state}: %{packages} package(s), %{bytes}, output %{path}.",
                    state = tr!(if ctx.dry_run { "preview" } else { "complete" }),
                    packages = report.packages,
                    bytes = format_bytes(report.bytes),
                    path = report.path.display()
                )
            );
        }
        OutputFormat::Json => {
            crate::cli::output::print_json(
                "export",
                &ExportOutput {
                    dry_run: ctx.dry_run,
                    path: report.path.to_string_lossy().into_owned(),
                    packages: report.packages,
                    bytes: report.bytes,
                },
            );
        }
    }
    Ok(())
}

fn safe_filename(name: &str) -> String {
    let filename: String = name
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect();
    let filename = filename.trim_matches('-');
    if filename.is_empty() {
        "orbit-pack".to_string()
    } else {
        filename.to_string()
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.2} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.2} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}
