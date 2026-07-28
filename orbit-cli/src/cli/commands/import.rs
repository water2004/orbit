use super::CliContext;
use anyhow::Result;

use crate::cli::output::{ImportOutput, OutputFormat};

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
                        "{}",
                        tr!(
                            "'%{package}' differs (existing %{existing}, imported %{imported}). Use imported value? [y/N] ",
                            package = package,
                            existing = existing.version_constraint().unwrap_or("*"),
                            imported = incoming.version_constraint().unwrap_or("*")
                        )
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
            match ctx.output.format {
                OutputFormat::Text => {
                    println!(
                        "{}",
                        tr!(
                            "Import %{state}: %{added} added, %{merged} remote sets merged, %{replaced} replaced, %{kept} kept.",
                            state = tr!(if ctx.dry_run { "preview" } else { "complete" }),
                            added = report.added.len(),
                            merged = report.merged.len(),
                            replaced = report.replaced.len(),
                            kept = report.kept.len()
                        )
                    );
                }
                OutputFormat::Json => {
                    crate::cli::output::print_json(
                        "import",
                        &ImportOutput {
                            dry_run: ctx.dry_run,
                            added: report.added,
                            merged: report.merged,
                            replaced: report.replaced,
                            kept: report.kept,
                            extracted: Vec::new(),
                        },
                    );
                }
            }
        }
        "zip" | "mrpack" => {
            let overwrite = strategy == orbit_core::ImportMergeStrategy::PreferImport;
            let report = if extension == "mrpack" {
                orbit_core::import_mrpack(&instance_dir, &source, overwrite, ctx.dry_run).await?
            } else {
                orbit_core::import_archive(&instance_dir, &source, overwrite, ctx.dry_run)?
            };
            if !ctx.dry_run && !report.extracted.is_empty() {
                let providers = orbit_core::providers::create_identification_providers(
                    &ctx.runtime.config().auth,
                )?;
                let sync = orbit_core::sync_instance(
                    &instance_dir,
                    &providers,
                    false,
                    super::install_interaction(ctx),
                )
                .await?;
                match ctx.output.format {
                    OutputFormat::Text => {
                        println!(
                            "{}",
                            tr!(
                                "Imported %{archives} archive file(s); sync added %{added}, changed %{changed}, and removed %{removed} package version(s).",
                                archives = report.extracted.len(),
                                added = sync.added.len(),
                                changed = sync.changed.len(),
                                removed = sync.removed.len()
                            )
                        );
                    }
                    OutputFormat::Json => {
                        crate::cli::output::print_json(
                            "import",
                            &ImportOutput {
                                dry_run: false,
                                added: report.added.clone(),
                                merged: report.merged.clone(),
                                replaced: report.replaced.clone(),
                                kept: report.kept.clone(),
                                extracted: report.extracted.clone(),
                            },
                        );
                    }
                }
            } else {
                match ctx.output.format {
                    OutputFormat::Text => {
                        println!(
                            "{}",
                            tr!(
                                "Import %{state}: %{archives} archive file(s) to extract, %{kept} existing file(s) kept.",
                                state = tr!(if ctx.dry_run { "preview" } else { "complete" }),
                                archives = report.extracted.len(),
                                kept = report.kept.len()
                            )
                        );
                    }
                    OutputFormat::Json => {
                        crate::cli::output::print_json(
                            "import",
                            &ImportOutput {
                                dry_run: ctx.dry_run,
                                added: report.added,
                                merged: report.merged,
                                replaced: report.replaced,
                                kept: report.kept,
                                extracted: report.extracted,
                            },
                        );
                    }
                }
            }
        }
        _ => anyhow::bail!(
            "{}",
            tr!("Unsupported file format. Expected .toml, .zip, or .mrpack.")
        ),
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
            "{}",
            tr!(
                "Unknown merge strategy '%{strategy}'. Expected prefer-existing, prefer-import, or interactive.",
                strategy = other
            )
        ),
    }
}
