use anyhow::Result;
use orbit_core::providers::{ModProvider, modrinth::ModrinthProvider};

pub async fn handle(
    query: String,
    platform: Option<String>,
    limit: usize,
    mc_version: Option<String>,
    modloader: Option<String>,
) -> Result<()> {
    // Validate / default platform
    let platform = platform.as_deref().unwrap_or("modrinth");
    if platform != "modrinth" {
        anyhow::bail!(
            "Platform '{platform}' is not yet supported. Currently only 'modrinth' is available."
        );
    }

    // Determine reference MC version for compatibility ✓ marks:
    //   user-supplied --mc-version > project orbit.toml > none
    let ref_mc = mc_version.clone().or_else(|| {
        std::fs::read_to_string("orbit.toml")
            .ok()
            .and_then(|s| toml::from_str::<orbit_core::OrbitManifest>(&s).ok())
            .map(|m| m.project.mc_version)
    });

    let provider = ModrinthProvider::new("orbit", 3)?;

    eprintln!(
        "Searching for \"{query}\" on {platform}{}...",
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

        println!(
            "  {check} {name} ({platform})  \u{2b07} {dl}  mc [{mc_list}]",
            name = item.name,
            platform = provider.name(),
            dl = dl,
        );
        println!("    {desc}");
    }

    println!();
    eprintln!("Found {} results.", results.len());

    Ok(())
}
