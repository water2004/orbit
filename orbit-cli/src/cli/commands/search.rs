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

    let providers = super::create_instance_providers(&instance_dir, platform.as_deref())?;

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

    let mut results = Vec::new();
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

    println!();
    for (provider, item) in &results {
        let compatible = ref_mc
            .as_ref()
            .map(|rmc| item.mc_versions.iter().any(|v| v == rmc))
            .unwrap_or(false);

        let check = if compatible { "\u{2713}" } else { " " };
        // Format downloads for readability
        let dl = if item.downloads >= 1_000_000 {
            format!("{:.1}M", item.downloads as f64 / 1_000_000.0)
        } else if item.downloads >= 1_000 {
            format!("{:.1}K", item.downloads as f64 / 1_000.0)
        } else {
            item.downloads.to_string()
        };

        let desc: String = item
            .description
            .chars()
            .take(80)
            .chain(if item.description.chars().count() > 80 {
                Some('\u{2026}') // …
            } else {
                None
            })
            .collect();

        // Show the latest few MC versions (search API doesn't return mod version)
        let mc_list = item
            .mc_versions
            .iter()
            .rev()
            .take(3)
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ");

        // Show slug prominently — this is what users type for `orbit install <slug>`
        let name_part = if item.name.to_lowercase() != item.slug.to_lowercase().replace('-', " ") {
            format!("{} — {}", item.slug, item.name)
        } else {
            item.slug.clone()
        };

        println!(
            "  {check} {name_part} ({platform})  \u{2b07} {dl}  mc [{mc_list}]",
            platform = provider,
            dl = dl,
        );
        println!("    {desc}");
    }

    println!();
    eprintln!("Found {} results.", results.len());

    Ok(())
}
