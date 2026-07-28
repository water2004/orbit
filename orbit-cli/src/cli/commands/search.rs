use super::CliContext;
use anyhow::Result;

use crate::cli::output::{
    OutputFormat, SearchFilters, SearchOutput, SearchResultView, render, search_result_view,
};

pub async fn handle(
    query: String,
    platform: Option<String>,
    limit: usize,
    mc_version: Option<String>,
    modloader: Option<String>,
    ctx: &CliContext,
) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;
    let ref_mc = match mc_version.clone() {
        Some(version) => Some(version),
        None => orbit_core::OrbitManifest::mc_version_from_dir(&instance_dir),
    };

    let providers =
        super::create_instance_providers(&instance_dir, platform.as_deref(), &ctx.runtime)?;

    if ctx.output.format == OutputFormat::Text {
        eprintln!(
            "{}",
            tr!(
                "Searching for \"%{query}\" on %{providers}%{filters}…",
                query = query,
                providers = providers
                    .iter()
                    .map(|provider| provider.name())
                    .collect::<Vec<_>>()
                    .join(", "),
                filters = if mc_version.is_some() || modloader.is_some() {
                    tr!(
                        " (Minecraft %{minecraft}, loader %{loader})",
                        minecraft = mc_version
                            .as_deref()
                            .map_or_else(|| tr!("any").into_owned(), ToString::to_string),
                        loader = modloader
                            .as_deref()
                            .map_or_else(|| tr!("any").into_owned(), ToString::to_string)
                    )
                } else {
                    String::new()
                }
            )
        );
    }

    let mut results: Vec<(&str, orbit_core::providers::SearchResultItem)> = Vec::new();
    let platforms = providers
        .iter()
        .map(|p| p.name().to_string())
        .collect::<Vec<_>>();
    for provider in &providers {
        for item in provider
            .search(&query, mc_version.as_deref(), modloader.as_deref(), limit)
            .await?
        {
            results.push((provider.name(), item));
        }
    }
    let total = results.len();
    results.truncate(limit);

    if results.is_empty() {
        if ctx.output.format == OutputFormat::Text {
            eprintln!("{}", tr!("No results found for '%{query}'.", query = query));
        } else {
            crate::cli::output::print_json(
                "search",
                &SearchOutput {
                    query: query.clone(),
                    platforms,
                    filters: SearchFilters {
                        mc_version: mc_version.clone(),
                        modloader: modloader.clone(),
                    },
                    ref_mc_version: ref_mc.clone(),
                    results: Vec::new(),
                    truncated: false,
                },
            );
        }
        return Ok(());
    }

    let views: Vec<SearchResultView> = results
        .iter()
        .map(|(provider, item)| search_result_view(provider, item, ref_mc.as_deref()))
        .collect();
    let output = SearchOutput {
        query: query.clone(),
        platforms,
        filters: SearchFilters {
            mc_version: mc_version.clone(),
            modloader: modloader.clone(),
        },
        ref_mc_version: ref_mc.clone(),
        truncated: total > views.len(),
        results: views,
    };

    render(ctx.output, "search", &output, |view| {
        let rows: Vec<(&str, &orbit_core::providers::SearchResultItem)> = results
            .iter()
            .map(|(provider, item)| (*provider, item))
            .collect();
        let mut table =
            crate::cli::output::search_results_table(&rows, view.ref_mc_version.as_deref());
        table.push('\n');
        table.push_str(&tr!("Found %{count} results.", count = view.results.len()));
        table
    });
    Ok(())
}
