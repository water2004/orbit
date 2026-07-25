use super::CliContext;
use anyhow::Result;

pub async fn handle(
    query: String,
    platform: Option<String>,
    limit: usize,
    mc_version: Option<String>,
    modloader: Option<String>,
    ctx: &CliContext,
) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;
    // Determine reference MC version for compatibility ✓ marks
    let ref_mc = match mc_version.clone() {
        Some(version) => Some(version),
        None => orbit_core::OrbitManifest::mc_version_from_dir(&instance_dir),
    };

    let providers =
        super::create_instance_providers(&instance_dir, platform.as_deref(), &ctx.runtime)?;

    eprintln!(
        "Searching for \"{query}\" on {}{}...",
        providers
            .iter()
            .map(|provider| provider.name())
            .collect::<Vec<_>>()
            .join(", "),
        if mc_version.is_some() || modloader.is_some() {
            format!(
                " (mc={}, loader={})",
                mc_version.as_deref().unwrap_or("any"),
                modloader.as_deref().unwrap_or("any")
            )
        } else {
            String::new()
        }
    );

    let mut results: Vec<(&str, orbit_core::providers::SearchResultItem)> = Vec::new();
    for provider in &providers {
        for item in provider
            .search(&query, mc_version.as_deref(), modloader.as_deref(), limit)
            .await?
        {
            results.push((provider.name(), item));
        }
    }
    results.truncate(limit);

    if results.is_empty() {
        eprintln!("No results found for '{query}'.");
        return Ok(());
    }

    let rows: Vec<(&str, &orbit_core::providers::SearchResultItem)> = results
        .iter()
        .map(|(provider, item)| (*provider, item))
        .collect();
    println!();
    println!(
        "{}",
        crate::cli::output::search_results_table(&rows, ref_mc.as_deref())
    );
    eprintln!("Found {} results.", results.len());

    Ok(())
}
