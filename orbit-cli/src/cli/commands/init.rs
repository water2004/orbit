use anyhow::Result;
use orbit_core::detection::LoaderDetectionService;
use orbit_core::init::{InitInput, run_init};

pub async fn handle(
    name: String,
    mc_version: Option<String>,
    modloader: Option<String>,
    modloader_version: Option<String>,
) -> Result<()> {
    let instance_dir = std::env::current_dir()?;

    // ── 1. 确定加载器 ──────────────────────────
    let loader = if let Some(ref l) = modloader {
        let service = LoaderDetectionService::new();
        if service.find_by_name(l).is_none() {
            anyhow::bail!("unknown modloader: '{l}'. Supported: fabric");
        }
        l.clone()
    } else {
        // 自动检测（Phase 1: 全部返回 None → 交互式选择）
        let service = LoaderDetectionService::new();
        let results = service.detect_all(&instance_dir)?;

        let best = results.first();
        let auto_detected = best.map(|r| r.confidence >= orbit_core::detection::Confidence::Low).unwrap_or(false);

        if auto_detected {
            best.unwrap().loader.as_str().to_string()
        } else {
            select_loader_interactive(&service)?
        }
    };

    // ── 2. 确定 MC 版本 ────────────────────────
    let mc_ver = match mc_version {
        Some(v) => v,
        None => prompt_mc_version()?,
    };

    // ── 3. 确定加载器版本 ──────────────────────
    let loader_ver = match modloader_version {
        Some(v) => v,
        None => prompt_loader_version(&loader)?,
    };

    // ── 4. 执行 init ───────────────────────────
    let input = InitInput {
        name: name.clone(),
        mc_version: mc_ver,
        modloader: loader.clone(),
        modloader_version: loader_ver,
        instance_dir,
    };

    let output = run_init(input)?;

    // ── 5. 输出结果 ────────────────────────────
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
) -> Result<String> {
    let loaders = service.known_loaders();
    if loaders.is_empty() {
        anyhow::bail!("no modloaders available for detection");
    }

    // Phase 1: 只有 Fabric
    eprintln!("? Could not auto-detect modloader. Available loaders:");
    for (i, (loader, name)) in loaders.iter().enumerate() {
        eprintln!("  [{}] {} ({})", i + 1, name, loader.as_str());
    }

    // 暂时只有 Fabric，直接选它
    let (loader, name) = &loaders[0];
    eprintln!("  Auto-selecting the only option: {} ({})", name, loader.as_str());

    Ok(loader.as_str().to_string())
}

fn prompt_mc_version() -> Result<String> {
    // Phase 1: 手动输入（后续改为从 version.json 自动探测）
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

fn prompt_loader_version(loader: &str) -> Result<String> {
    let default = match loader {
        "fabric" => "0.15.7",
        _ => "0.0.0",
    };
    eprint!("? {} loader version [{}]: ", loader, default);
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let input = input.trim();
    if input.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(input.to_string())
    }
}
