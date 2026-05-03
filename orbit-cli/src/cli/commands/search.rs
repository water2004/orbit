use anyhow::Result;
use orbit_core::providers::create_providers_default;
use super::CliContext;

pub async fn handle(
    query: String,
    _platform: Option<String>,
    limit: usize,
    mc_version: Option<String>,
    modloader: Option<String>,
    _ctx: &CliContext,
) -> Result<()> {
    // Determine reference MC version for compatibility ✓ marks
    let ref_mc = mc_version.clone()
        .or_else(|| orbit_core::OrbitManifest::mc_version_from_dir(&std::env::current_dir().ok()?));

    let providers = create_providers_default()?;
    let provider = &providers[0];

    eprintln!(
        "Searching for \"{query}\" on {}{}...",
        provider.name(),
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

    let results = provider
        .search(&query, mc_version.as_deref(), modloader.as_deref(), limit)
        .await?;

    if results.is_empty() {
        eprintln!("No results found for '{query}'.");
        return Ok(());
    }

    println!();
    for item in &results {
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
            .chain(
                if item.description.chars().count() > 80 {
                    Some('\u{2026}') // …
                } else {
                    None
                },
            )
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
            platform = provider.name(),
            dl = dl,
        );
        println!("    {desc}");
    }

    println!();
    eprintln!("Found {} results.", results.len());

    Ok(())
}
