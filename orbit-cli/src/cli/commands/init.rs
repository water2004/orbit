use super::CliContext;
use anyhow::Result;
use orbit_core::detection::LoaderDetectionService;
use orbit_core::init::{InitInput, detect_mc_version, run_init};
use orbit_core::providers::create_identification_providers;

pub async fn handle(
    name: String,
    mc_version: Option<String>,
    modloader: Option<String>,
    modloader_version: Option<String>,
    ctx: &CliContext,
) -> Result<()> {
    let instance_dir = std::env::current_dir()?;
    let registered_path = instance_dir.clone();

    // ── 1. 确定 MC 版本 ────────────────────────
    let mc_ver = match mc_version {
        Some(v) => v,
        None => match detect_mc_version(&instance_dir) {
            Ok(ver) => {
                println!(
                    "✓ Detected Minecraft version: {} ({})",
                    ver.id,
                    if ver.stable { "stable" } else { "snapshot" }
                );
                ver.id
            }
            Err(_) if ctx.yes => {
                anyhow::bail!(
                    "could not detect the Minecraft version; pass --mc-version when using --yes"
                )
            }
            Err(_) => prompt_mc_version()?,
        },
    };

    // ── 2. 确定加载器及其版本 ──────────────────
    let service = LoaderDetectionService::new();
    let (loader, loader_ver) = if let Some(ref requested_loader) = modloader {
        let detector = service.find_by_name(requested_loader).ok_or_else(|| {
            let supported = service
                .known_loaders()
                .into_iter()
                .map(|(loader, _)| loader.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            anyhow::anyhow!("unknown modloader: '{requested_loader}'. Supported: {supported}")
        })?;
        let detected = detector.detect(&instance_dir)?;
        let loader = detector.loader_type().as_str().to_string();
        let version = choose_loader_version(modloader_version, detected.version, &loader, ctx.yes)?;
        (loader, version)
    } else {
        let results = service.detect_all(&instance_dir)?;
        let best = results.first();

        match best {
            Some(info) if info.confidence >= orbit_core::detection::Confidence::Certain => {
                let loader = info.loader.as_str().to_string();
                let ver = choose_loader_version(
                    modloader_version,
                    info.version.clone(),
                    &loader,
                    ctx.yes,
                )?;
                println!(
                    "✓ Detected {} loader {} ({})",
                    loader,
                    ver,
                    info.evidence.join(", ")
                );
                (loader, ver)
            }
            _ => {
                if ctx.yes {
                    anyhow::bail!(
                        "could not auto-detect the modloader; pass --modloader and \
                         --modloader-version when using --yes"
                    );
                }
                let (l, name) = select_loader_interactive(&service)?;
                let ver = choose_loader_version(modloader_version, None, &l, ctx.yes)?;
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
        dry_run: ctx.dry_run,
    };

    let providers = create_identification_providers(&ctx.runtime.config().auth)?;
    let output = run_init(
        input,
        &providers,
        super::install_interaction(ctx.dry_run, ctx.yes),
    )
    .await?;

    // ── 4. 输出结果 ────────────────────────────
    if ctx.dry_run {
        println!(
            "[dry-run] would initialize Orbit project '{name}' ({loader}, MC {})",
            output.manifest.project.mc_version
        );
        println!("  [dry-run] would create orbit.toml");
        println!(
            "  [dry-run] would create orbit.lock ({} entries)",
            output.locked_packages
        );
    } else {
        println!(
            "✓ Initialized Orbit project '{name}' ({loader}, MC {})",
            output.manifest.project.mc_version
        );
        println!("  orbit.toml created");
        println!("  orbit.lock created ({} entries)", output.locked_packages);
    }
    if output.scanned_mods.is_empty() {
        println!("  No mods found in mods/ directory.");
    } else {
        let identified = output
            .scanned_mods
            .iter()
            .filter(|m| m.mod_id.is_some())
            .count();
        let unknown = output.scanned_mods.len() - identified;
        println!(
            "  Scanned {} mods ({} identified, {} unknown)",
            output.scanned_mods.len(),
            identified,
            unknown,
        );
    }
    if let Some(error) = &output.dependency_error {
        eprintln!("Dependency graph verification failed:\n{error}");
        eprintln!("Use 'orbit install' or 'orbit sync' to fix missing dependencies.");
    }
    for package in &output.removed {
        if ctx.dry_run {
            println!(
                "  [dry-run] would remove unselected package {} v{} ({})",
                package.mod_id, package.version, package.filename
            );
        } else {
            println!(
                "  Removed unselected package {} v{} ({})",
                package.mod_id, package.version, package.filename
            );
        }
    }
    if !ctx.dry_run {
        orbit_core::register_instance(
            ctx.runtime.paths(),
            orbit_core::InstanceEntry {
                name: name.clone(),
                path: registered_path.to_string_lossy().into_owned(),
                mc_version: output.manifest.project.mc_version.clone(),
                modloader: loader.clone(),
                is_default: false,
            },
        )?;
        println!("  Run 'orbit install' to restore missing mods.");
    }

    Ok(())
}

// ── 交互式辅助 ──────────────────────────────────

fn select_loader_interactive(service: &LoaderDetectionService) -> Result<(String, &'static str)> {
    let loaders = service.known_loaders();
    if loaders.is_empty() {
        anyhow::bail!("no modloaders available for detection");
    }
    eprintln!("? Could not auto-detect modloader. Available loaders:");
    for (i, (loader, name)) in loaders.iter().enumerate() {
        eprintln!("  [{}] {} ({})", i + 1, name, loader.as_str());
    }
    eprint!("Choose a loader [1]: ");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let index = if input.trim().is_empty() {
        0
    } else {
        input
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|index| index.checked_sub(1))
            .filter(|index| *index < loaders.len())
            .ok_or_else(|| anyhow::anyhow!("invalid modloader choice"))?
    };
    let (loader, name) = &loaders[index];
    Ok((loader.as_str().to_string(), *name))
}

fn prompt_mc_version() -> Result<String> {
    eprint!("? Minecraft version: ");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let version = input.trim();
    if version.is_empty() {
        anyhow::bail!("Minecraft version is required; pass --mc-version");
    }
    Ok(version.to_string())
}

fn choose_loader_version(
    explicit: Option<String>,
    detected: Option<String>,
    loader: &str,
    non_interactive: bool,
) -> Result<String> {
    if let Some(version) = explicit.or(detected) {
        return Ok(version);
    }
    if non_interactive {
        anyhow::bail!(
            "could not detect the {loader} loader version; pass --modloader-version when using --yes"
        );
    }
    eprint!("? {loader} loader version: ");
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let version = input.trim();
    if version.is_empty() {
        anyhow::bail!(
            "loader version is required for reproducible installs; pass --modloader-version"
        );
    }
    Ok(version.to_string())
}
