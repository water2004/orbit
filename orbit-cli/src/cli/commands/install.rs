use anyhow::{Context, Result};
use orbit_core::OrbitError;
use orbit_core::providers::{ModProvider, modrinth::ModrinthProvider};

pub async fn handle(
    mod_name: Option<String>,
    constraint: Option<String>,
    target: Option<String>,
    group: Option<String>,
    no_optional: bool,
    locked: bool,
) -> Result<()> {
    match mod_name {
        Some(slug) => handle_install_mod(slug, constraint.unwrap_or_else(|| "*".into()), locked).await,
        None => {
            // 全量还原（deferred to Phase 2）
            let target = target.as_deref().unwrap_or("both");
            println!("Installing all mods (target={target}, group={group:?}, no_optional={no_optional}, locked={locked})...");
            println!("Note: full environment restore not yet implemented. Use 'orbit install <slug>' to install a single mod.");
            Ok(())
        }
    }
}

pub async fn handle_install_mod(
    slug: String,
    constraint: String,
    _locked: bool,
) -> Result<()> {
    // 1. 加载 orbit.toml
    let manifest_path = std::path::Path::new("orbit.toml");
    if !manifest_path.exists() {
        anyhow::bail!(
            "orbit.toml not found in this directory.\n\
             Run 'orbit init <name>' first to initialize a project."
        );
    }
    let mut manifest = orbit_core::OrbitManifest::from_path(manifest_path)
        .context("failed to parse orbit.toml")?;

    // 2. 加载或创建 orbit.lock
    let lock_path = std::path::Path::new("orbit.lock");
    let mut lockfile = if lock_path.exists() {
        orbit_core::OrbitLockfile::from_path(lock_path)
            .context("failed to parse orbit.lock")?
    } else {
        orbit_core::OrbitLockfile {
            meta: orbit_core::LockMeta {
                mc_version: manifest.project.mc_version.clone(),
                modloader: manifest.project.modloader.clone(),
                modloader_version: manifest.project.modloader_version.clone(),
            },
            entries: vec![],
        }
    };

    let mods_dir = std::path::Path::new("mods");
    if !mods_dir.exists() {
        std::fs::create_dir_all(mods_dir).context("failed to create mods/ directory")?;
    }

    let provider = ModrinthProvider::new("orbit", 3)
        .context("failed to create Modrinth provider")?;

    // 3. 尝试安装
    match orbit_core::installer::install_mod(
        &slug,
        &constraint,
        &provider,
        &mut manifest,
        &mut lockfile,
        mods_dir,
        false, // no_deps: resolve deps by default
        false, // existing_ok: error if already exists
    ).await {
        Ok(report) => {
            // 4. 写入 orbit.toml + orbit.lock
            let toml_content = manifest.to_toml_string()
                .context("failed to serialize orbit.toml")?;
            std::fs::write(manifest_path, toml_content)
                .context("failed to write orbit.toml")?;

            let lock_content = lockfile.to_toml_string()
                .context("failed to serialize orbit.lock")?;
            std::fs::write(lock_path, lock_content)
                .context("failed to write orbit.lock")?;

            // 5. 输出报告
            for m in &report.installed {
                println!("  + installed {} v{}", m.key, m.version);
                for (dep_id, dep_ver, _) in &m.jar_deps {
                    println!("      ↳ {dep_id} {dep_ver}");
                }
            }
            for dep in &report.already_satisfied {
                println!("  ✓ {dep} (already satisfied)");
            }
            for dep in &report.skipped_optional {
                println!("  ~ {dep} (optional, skipped)");
            }
            println!();
            println!(
                "Installed {} mod(s), {} already satisfied, {} optional skipped.",
                report.installed.len(),
                report.already_satisfied.len(),
                report.skipped_optional.len(),
            );
            Ok(())
        }
        Err(OrbitError::ModNotFound(_)) => {
            // 搜索回退
            let results = provider.search(&slug, None, None, 5).await
                .context("search failed")?;

            if results.is_empty() {
                anyhow::bail!("No mod found for '{slug}' on any platform.");
            }

            eprintln!("Could not find '{slug}'. Did you mean:");
            for (i, item) in results.iter().enumerate() {
                let dl = format_downloads(item.downloads);
                eprintln!(
                    "  [{i}] {slug} — {name}  ⬇ {dl}  mc [{mc}]",
                    slug = item.slug,
                    name = item.name,
                    dl = dl,
                    mc = item.mc_versions.iter().rev().take(3).map(|s| s.as_str()).collect::<Vec<_>>().join(", "),
                );
            }

            // 交互式选择
            eprint!("\nChoose a number (or press Enter to cancel): ");
            let mut input = String::new();
            std::io::stdin().read_line(&mut input).ok();
            let trimmed = input.trim();
            if trimmed.is_empty() {
                anyhow::bail!("Install cancelled.");
            }

            match trimmed.parse::<usize>() {
                Ok(idx) if idx < results.len() => {
                    let chosen = &results[idx];
                    eprintln!("Installing {}...", chosen.slug);
                    // 递归重试
                    Box::pin(handle_install_mod(chosen.slug.clone(), constraint, _locked)).await
                }
                _ => anyhow::bail!("Invalid choice."),
            }
        }
        Err(OrbitError::Conflict(msg)) => {
            anyhow::bail!("Dependency conflict:\n\n  {msg}");
        }
        Err(e) => {
            anyhow::bail!("Install failed: {e}");
        }
    }
}

fn format_downloads(d: u64) -> String {
    if d >= 1_000_000 {
        format!("{:.1}M", d as f64 / 1_000_000.0)
    } else if d >= 1_000 {
        format!("{:.1}K", d as f64 / 1_000.0)
    } else {
        d.to_string()
    }
}
