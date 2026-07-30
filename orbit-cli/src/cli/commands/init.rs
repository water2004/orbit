use super::CliContext;
use anyhow::Result;
use orbit_core::init::{
    InitInput, detect_loader_candidates, detect_mc_versions, known_loader_choices, run_init,
};
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
        None => match detect_mc_versions(&instance_dir) {
            Ok(versions) if versions.len() == 1 => {
                let ver = &versions[0];
                ctx.print_result_line(format_args!(
                    "{}",
                    tr!(
                        "✓ Detected Minecraft version: %{version} (%{channel})",
                        version = ver.id,
                        channel = tr!(if ver.stable { "stable" } else { "snapshot" })
                    )
                ));
                ver.id.clone()
            }
            Ok(versions) if versions.len() > 1 && ctx.yes => {
                anyhow::bail!(
                    "{}",
                    tr!(
                        "Multiple Minecraft versions were detected: %{versions}; pass --mc-version when using --yes",
                        versions = versions
                            .iter()
                            .map(|version| version.id.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                )
            }
            Ok(versions) if versions.len() > 1 => select_mc_version(&versions)?,
            Ok(_) if ctx.yes => anyhow::bail!(
                "{}",
                tr!("Could not detect the Minecraft version; pass --mc-version when using --yes")
            ),
            Ok(_) => prompt_mc_version()?,
            Err(error) if ctx.yes => {
                anyhow::bail!(
                    "{}",
                    tr!(
                        "Could not detect the Minecraft version: %{detail}; pass --mc-version when using --yes",
                        detail = error
                    )
                )
            }
            Err(error) => {
                eprintln!(
                    "{}",
                    tr!(
                        "? Automatic Minecraft detection failed: %{detail}",
                        detail = error
                    )
                );
                prompt_mc_version()?
            }
        },
    };

    // ── 2. 确定加载器及其版本 ──────────────────
    let (loader, loader_ver) = if let Some(ref requested_loader) = modloader {
        let detected = detect_loader_candidates(&instance_dir, &mc_ver, Some(requested_loader))?
            .into_iter()
            .next()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "{}",
                    tr!(
                        "No detector result for '%{loader}'",
                        loader = requested_loader
                    )
                )
            })?;
        let loader = detected.loader.to_string();
        let version =
            choose_loader_version(modloader_version, detected.versions, &loader, ctx.yes)?;
        (loader, version)
    } else {
        let results = detect_loader_candidates(&instance_dir, &mc_ver, None)?;
        let certain = results
            .iter()
            .filter(|info| info.certain)
            .collect::<Vec<_>>();

        match certain.as_slice() {
            [info] => {
                let loader = info.loader.as_str().to_string();
                let ver = choose_loader_version(
                    modloader_version,
                    info.versions.clone(),
                    &loader,
                    ctx.yes,
                )?;
                ctx.print_result_line(format_args!(
                    "{}",
                    tr!(
                        "✓ Detected %{loader} loader %{version} (%{evidence})",
                        loader = loader,
                        version = ver,
                        evidence = info.evidence.join(", ")
                    )
                ));
                (loader, ver)
            }
            infos if infos.len() > 1 => {
                let candidates = infos
                    .iter()
                    .map(|info| info.loader.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                anyhow::bail!(
                    "{}",
                    tr!(
                        "Multiple mod loaders match Minecraft %{minecraft}: %{loaders}; pass --modloader",
                        minecraft = mc_ver,
                        loaders = candidates
                    )
                );
            }
            _ => {
                if ctx.yes {
                    anyhow::bail!(
                        "{}",
                        tr!(
                            "Could not auto-detect the mod loader; pass --modloader and --modloader-version when using --yes"
                        )
                    );
                }
                let (l, name) = select_loader_interactive()?;
                let ver = choose_loader_version(modloader_version, Vec::new(), &l, ctx.yes)?;
                eprintln!(
                    "  {}",
                    tr!(
                        "Using %{loader} loader %{version}",
                        loader = name,
                        version = ver
                    )
                );
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

    let providers = create_identification_providers(ctx.runtime.config())?;
    let output = run_init(input, &providers).await?;

    let identified = output
        .scanned_mods
        .iter()
        .filter(|m| m.mod_id.is_some())
        .count();
    let unknown = output.scanned_mods.len() - identified;

    // Registration is part of a successful init transaction, not a text
    // rendering side effect. Machine clients such as orbit-gui must observe
    // exactly the same state transition as an interactive terminal.
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
    }

    if ctx.output.format == crate::cli::output::OutputFormat::Json {
        let view = crate::cli::output::InitOutput {
            dry_run: ctx.dry_run,
            name: name.clone(),
            mc_version: output.manifest.project.mc_version.clone(),
            modloader: loader.clone(),
            modloader_version: output.manifest.project.modloader_version.clone(),
            locked_packages: output.locked_packages,
            scanned_mods: output.scanned_mods.len(),
            identified,
            unknown,
            lock_created: output.lock_created,
            dependency_error: output.dependency_error.clone(),
        };
        ctx.print_json("init", &view);
        return Ok(());
    }
    if ctx.quiet {
        return Ok(());
    }

    // ── 4. 输出结果 ────────────────────────────
    if ctx.dry_run {
        println!(
            "{}",
            tr!(
                "[dry-run] would initialize Orbit project '%{name}' (%{loader}, Minecraft %{minecraft})",
                name = name,
                loader = loader,
                minecraft = output.manifest.project.mc_version
            )
        );
        println!("  {}", tr!("[dry-run] would create orbit.toml"));
        if output.lock_created {
            println!(
                "  {}",
                tr!(
                    "[dry-run] would create orbit.lock (%{entries} entries)",
                    entries = output.locked_packages
                )
            );
        }
    } else {
        println!(
            "{}",
            tr!(
                "✓ Initialized Orbit project '%{name}' (%{loader}, Minecraft %{minecraft})",
                name = name,
                loader = loader,
                minecraft = output.manifest.project.mc_version
            )
        );
        println!("  {}", tr!("orbit.toml created"));
        if output.lock_created {
            println!(
                "  {}",
                tr!(
                    "orbit.lock created (%{entries} entries)",
                    entries = output.locked_packages
                )
            );
        } else {
            println!(
                "  {}",
                tr!("orbit.lock was not created because local package selection is required.")
            );
        }
    }
    if output.scanned_mods.is_empty() {
        println!("  {}", tr!("No mods were found in the mods/ directory."));
    } else {
        println!(
            "  {}",
            tr!(
                "Scanned %{total} mods (%{identified} identified, %{unknown} unknown)",
                total = output.scanned_mods.len(),
                identified = identified,
                unknown = unknown
            )
        );
    }
    if let Some(error) = &output.dependency_error {
        eprintln!(
            "{}",
            tr!(
                "Dependency graph verification failed:\n%{detail}",
                detail = error
            )
        );
        eprintln!("{}", tr!("Run 'orbit fix' to resolve the package graph."));
    }
    if !ctx.dry_run && (output.dependency_error.is_some() || !output.lock_created) {
        println!(
            "  {}",
            tr!("Run 'orbit fix' to create a feasible exact lock.")
        );
    }

    Ok(())
}

// ── 交互式辅助 ──────────────────────────────────

fn select_loader_interactive() -> Result<(String, String)> {
    let loaders = known_loader_choices();
    if loaders.is_empty() {
        anyhow::bail!("{}", tr!("No mod loaders are available for detection"));
    }
    eprintln!(
        "{}",
        tr!("? Could not auto-detect a mod loader. Available loaders:")
    );
    for (i, (loader, name)) in loaders.iter().enumerate() {
        eprintln!("  [{}] {} ({})", i + 1, name, loader);
    }
    eprint!("{}", tr!("Choose a loader [1]: "));
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
            .ok_or_else(|| anyhow::anyhow!("{}", tr!("Invalid mod loader choice")))?
    };
    let (loader, name) = &loaders[index];
    Ok((loader.clone(), name.clone()))
}

fn prompt_mc_version() -> Result<String> {
    eprint!("{}", tr!("? Minecraft version: "));
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let version = input.trim();
    if version.is_empty() {
        anyhow::bail!(
            "{}",
            tr!("Minecraft version is required; pass --mc-version")
        );
    }
    Ok(version.to_string())
}

fn select_mc_version(versions: &[orbit_core::McVersion]) -> Result<String> {
    eprintln!("{}", tr!("? Multiple Minecraft versions are available:"));
    for (index, version) in versions.iter().enumerate() {
        eprintln!("  [{}] {}", index + 1, version.id);
    }
    eprint!("{}", tr!("Choose a Minecraft version [1]: "));
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
            .filter(|index| *index < versions.len())
            .ok_or_else(|| anyhow::anyhow!("{}", tr!("Invalid Minecraft version choice")))?
    };
    Ok(versions[index].id.clone())
}

fn choose_loader_version(
    explicit: Option<String>,
    mut detected: Vec<String>,
    loader: &str,
    non_interactive: bool,
) -> Result<String> {
    if let Some(version) = explicit {
        return Ok(version);
    }
    detected.sort();
    detected.dedup();
    if detected.len() == 1 {
        return Ok(detected.remove(0));
    }
    if detected.len() > 1 {
        if non_interactive {
            anyhow::bail!(
                "{}",
                tr!(
                    "Multiple %{loader} loader versions were detected: %{versions}; pass --modloader-version when using --yes",
                    loader = loader,
                    versions = detected.join(", ")
                )
            );
        }
        eprintln!(
            "{}",
            tr!(
                "? Multiple %{loader} loader versions are available:",
                loader = loader
            )
        );
        for (index, version) in detected.iter().enumerate() {
            eprintln!("  [{}] {version}", index + 1);
        }
        eprint!("{}", tr!("Choose a loader version [1]: "));
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
                .filter(|index| *index < detected.len())
                .ok_or_else(|| anyhow::anyhow!("{}", tr!("Invalid loader version choice")))?
        };
        return Ok(detected[index].clone());
    }
    if non_interactive {
        anyhow::bail!(
            "{}",
            tr!(
                "Could not detect the %{loader} loader version; pass --modloader-version when using --yes",
                loader = loader
            )
        );
    }
    eprint!("{}", tr!("? %{loader} loader version: ", loader = loader));
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let version = input.trim();
    if version.is_empty() {
        anyhow::bail!(
            "{}",
            tr!("A loader version is required for reproducible installs; pass --modloader-version")
        );
    }
    Ok(version.to_string())
}
