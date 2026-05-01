use anyhow::Result;
use orbit_core::detection::LoaderDetectionService;
use orbit_core::init::{detect_mc_version, InitInput, run_init};

pub async fn handle(
    name: String,
    mc_version: Option<String>,
    modloader: Option<String>,
    modloader_version: Option<String>,
) -> Result<()> {
    let instance_dir = std::env::current_dir()?;

    // ── 1. 确定 MC 版本 ────────────────────────
    let mc_ver = match mc_version {
        Some(v) => v,
        None => match detect_mc_version(&instance_dir) {
            Ok(ver) => {
                println!("✓ Detected Minecraft version: {} ({})", ver.id, if ver.stable { "stable" } else { "snapshot" });
                ver.id
            }
            Err(_) => prompt_mc_version()?,
        },
    };

    // ── 2. 确定加载器及其版本 ──────────────────
    let (loader, loader_ver) = if let Some(ref l) = modloader {
        let service = LoaderDetectionService::new();
        if service.find_by_name(l).is_none() {
            anyhow::bail!("unknown modloader: '{l}'. Supported: fabric");
        }
        let ver = modloader_version.unwrap_or_else(|| default_loader_version(l));
        (l.clone(), ver)
    } else {
        let service = LoaderDetectionService::new();
        let results = service.detect_all(&instance_dir)?;
        let best = results.first();

        match best {
            Some(info) if info.confidence >= orbit_core::detection::Confidence::Certain => {
                let ver = info.version.clone().unwrap_or_else(|| {
                    modloader_version.unwrap_or_else(|| default_loader_version("fabric"))
                });
                println!(
                    "✓ Detected {} loader {} ({})",
                    info.loader.as_str(),
                    ver,
                    info.evidence.join(", ")
                );
                (info.loader.as_str().to_string(), ver)
            }
            _ => {
                let (l, name) = select_loader_interactive(&service)?;
                let ver = modloader_version.unwrap_or_else(|| default_loader_version(&l));
                eprintln!("  Using {} loader {}", name, ver);
                (l, ver)
            }
        }
    };

    // ── 3. 执行 init ───────────────────────────
    let input = InitInput {
        name: name.clone(),
        mc_version: mc_ver,
        modloader: loader.clone(),
        modloader_version: loader_ver,
        instance_dir,
    };

    let output = run_init(input)?;

    // ── 4. 输出结果 ────────────────────────────
    println!(
        "✓ Initialized Orbit project '{name}' ({loader}, MC {})",
        output.manifest.project.mc_version
    );
    println!("  orbit.toml created");
    if output.scanned_mods.is_empty() {
        println!("  No mods found in mods/ directory.");
    } else {
        let identified = output.scanned_mods.iter().filter(|m| m.mod_id.is_some()).count();
        let unknown = output.scanned_mods.len() - identified;
        println!(
            "  Scanned {} mods ({} identified, {} unknown)",
            output.scanned_mods.len(),
            identified,
            unknown,
        );
    }
    println!("  Run 'orbit install' to restore missing mods.");

    Ok(())
}

// ── 交互式辅助 ──────────────────────────────────

fn select_loader_interactive(
    service: &LoaderDetectionService,
) -> Result<(String, &'static str)> {
    let loaders = service.known_loaders();
    if loaders.is_empty() {
        anyhow::bail!("no modloaders available for detection");
    }
    eprintln!("? Could not auto-detect modloader. Available loaders:");
    for (i, (loader, name)) in loaders.iter().enumerate() {
        eprintln!("  [{}] {} ({})", i + 1, name, loader.as_str());
    }
    let (loader, name) = &loaders[0];
    eprintln!("  Auto-selecting the only option: {name}");
    Ok((loader.as_str().to_string(), *name))
}

fn prompt_mc_version() -> Result<String> {
    let default = "1.20.1";
    eprint!("? Minecraft version [{}]: ", default);
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let input = input.trim();
    if input.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(input.to_string())
    }
}

fn default_loader_version(loader: &str) -> String {
    match loader {
        "fabric" => "0.15.7".into(),
        _ => "0.0.0".into(),
    }
}
