use super::CliContext;
use anyhow::Result;

pub async fn handle(file: String, merge_strategy: Option<String>, ctx: &CliContext) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;
    let source = std::path::PathBuf::from(&file);
    let extension = source
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let strategy = parse_strategy(merge_strategy.as_deref(), ctx.yes)?;

    match extension.as_str() {
        "toml" => {
            let report = orbit_core::import_manifest(
                &instance_dir,
                &source,
                strategy,
                ctx.dry_run,
                |package, existing, incoming| {
                    eprint!(
                        "'{package}' differs (existing {}, imported {}). Use imported value? [y/N] ",
                        existing.version_constraint().unwrap_or("*"),
                        incoming.version_constraint().unwrap_or("*")
                    );
                    use std::io::Write;
                    std::io::stdout().flush()?;
                    let mut input = String::new();
                    std::io::stdin().read_line(&mut input)?;
                    Ok(matches!(
                        input.trim().to_ascii_lowercase().as_str(),
                        "y" | "yes"
                    ))
                },
            )?;
            println!(
                "Import {}: {} added, {} replaced, {} kept.",
                if ctx.dry_run { "preview" } else { "complete" },
                report.added.len(),
                report.replaced.len(),
                report.kept.len()
            );
        }
        "zip" | "mrpack" => {
            let overwrite = strategy == orbit_core::ImportMergeStrategy::PreferImport;
            let report = if extension == "mrpack" {
                orbit_core::import_mrpack(&instance_dir, &source, overwrite, ctx.dry_run).await?
            } else {
                orbit_core::import_archive(&instance_dir, &source, overwrite, ctx.dry_run)?
            };
            if !ctx.dry_run && !report.extracted.is_empty() {
                let providers =
                    super::create_instance_providers(&instance_dir, None, &ctx.runtime)?;
                let sync = orbit_core::sync_instance(&instance_dir, &providers, false).await?;
                println!(
                    "Imported {} JAR(s); sync added {} and changed {} package(s).",
                    report.extracted.len(),
                    sync.added.len(),
                    sync.changed.len()
                );
            } else {
                println!(
                    "Import {}: {} JAR(s) to extract, {} existing file(s) kept.",
                    if ctx.dry_run { "preview" } else { "complete" },
                    report.extracted.len(),
                    report.kept.len()
                );
            }
        }
        _ => anyhow::bail!("Unsupported file format. Expected .toml, .zip, or .mrpack."),
    }
    Ok(())
}

fn parse_strategy(
    strategy: Option<&str>,
    assume_yes: bool,
) -> Result<orbit_core::ImportMergeStrategy> {
    match strategy.unwrap_or(if assume_yes {
        "prefer-import"
    } else {
        "interactive"
    }) {
        "prefer-existing" => Ok(orbit_core::ImportMergeStrategy::PreferExisting),
        "prefer-import" => Ok(orbit_core::ImportMergeStrategy::PreferImport),
        "interactive" => Ok(orbit_core::ImportMergeStrategy::Interactive),
        other => anyhow::bail!(
            "Unknown merge strategy '{other}'. Expected prefer-existing, prefer-import, or interactive."
        ),
    }
}
