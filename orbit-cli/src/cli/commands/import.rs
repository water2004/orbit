use super::CliContext;
use anyhow::Result;
use std::collections::BTreeSet;

use crate::cli::output::{ImportOutput, OutputFormat};

pub async fn handle(
    file: String,
    merge_strategy: Option<String>,
    optional_files: Vec<String>,
    all_optional: bool,
    ctx: &CliContext,
) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;
    let source = std::path::PathBuf::from(&file);
    let extension = source
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let strategy = parse_strategy(merge_strategy.as_deref(), ctx.yes)?;
    let optional_files = optional_files.into_iter().collect();
    let mrpack_selection = MrpackSelection {
        all: all_optional,
        files: &optional_files,
    };

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
                            existing = existing.version_constraint(),
                            imported = incoming.version_constraint()
                        )
                    );
                    use std::io::Write;
                    std::io::stderr().flush()?;
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
                    ctx.print_result_line(format_args!(
                        "{}",
                        tr!(
                            "Import %{state}: %{added} added, %{merged} remote sets merged, %{replaced} replaced, %{kept} kept.",
                            state = tr!(if ctx.dry_run { "preview" } else { "complete" }),
                            added = report.added.len(),
                            merged = report.merged.len(),
                            replaced = report.replaced.len(),
                            kept = report.kept.len()
                        )
                    ));
                }
                OutputFormat::Json => {
                    ctx.print_json(
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
        "orbitbundle" | "mrpack" => {
            let mut overwrite = strategy == orbit_core::ImportMergeStrategy::PreferImport;
            let preview = if strategy == orbit_core::ImportMergeStrategy::Interactive {
                let preview = import_archive(
                    &extension,
                    &instance_dir,
                    &source,
                    false,
                    mrpack_selection,
                    true,
                    ctx,
                )
                .await?;
                if !preview.kept.is_empty() {
                    eprint!(
                        "{}",
                        tr!(
                            "Replace %{count} conflicting package file(s)? [y/N] ",
                            count = preview.kept.len()
                        )
                    );
                    use std::io::Write as _;
                    std::io::stderr().flush()?;
                    let mut input = String::new();
                    std::io::stdin().read_line(&mut input)?;
                    overwrite = matches!(input.trim().to_ascii_lowercase().as_str(), "y" | "yes");
                }
                Some(preview)
            } else {
                None
            };
            let report = if ctx.dry_run {
                if let Some(mut preview) = preview {
                    if overwrite {
                        preview.extracted.append(&mut preview.kept);
                        preview.extracted.sort();
                    }
                    preview
                } else {
                    import_archive(
                        &extension,
                        &instance_dir,
                        &source,
                        overwrite,
                        mrpack_selection,
                        true,
                        ctx,
                    )
                    .await?
                }
            } else {
                import_archive(
                    &extension,
                    &instance_dir,
                    &source,
                    overwrite,
                    mrpack_selection,
                    false,
                    ctx,
                )
                .await?
            };
            if !ctx.dry_run && !report.extracted.is_empty() {
                let providers =
                    orbit_core::providers::create_identification_providers(ctx.runtime.config())?;
                let sync = orbit_core::sync_instance(&instance_dir, &providers, false).await?;
                match ctx.output.format {
                    OutputFormat::Text => {
                        ctx.print_result_line(format_args!(
                            "{}",
                            tr!(
                                "Imported %{archives} archive file(s); sync added %{added}, changed %{changed}, and removed %{removed} stale package(s).",
                                archives = report.extracted.len(),
                                added = sync.added.len(),
                                changed = sync.changed.len(),
                                removed = sync.removed.len()
                            )
                        ));
                    }
                    OutputFormat::Json => {
                        ctx.print_json(
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
                        ctx.print_result_line(format_args!(
                            "{}",
                            tr!(
                                "Import %{state}: %{archives} archive file(s) to extract, %{kept} existing file(s) kept.",
                                state = tr!(if ctx.dry_run { "preview" } else { "complete" }),
                                archives = report.extracted.len(),
                                kept = report.kept.len()
                            )
                        ));
                    }
                    OutputFormat::Json => {
                        ctx.print_json(
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
            tr!("Unsupported file format. Expected .toml, .orbitbundle, or .mrpack.")
        ),
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct MrpackSelection<'a> {
    all: bool,
    files: &'a BTreeSet<String>,
}

async fn import_archive(
    extension: &str,
    instance_dir: &std::path::Path,
    source: &std::path::Path,
    overwrite: bool,
    selection: MrpackSelection<'_>,
    dry_run: bool,
    ctx: &CliContext,
) -> Result<orbit_core::ImportReport> {
    if extension == "mrpack" {
        Ok(orbit_core::import_mrpack(
            instance_dir,
            source,
            overwrite,
            selection.all,
            selection.files,
            dry_run,
            super::operation_progress(ctx),
        )
        .await?)
    } else {
        Ok(orbit_core::import_bundle(
            instance_dir,
            source,
            overwrite,
            dry_run,
            super::operation_progress(ctx),
        )?)
    }
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
