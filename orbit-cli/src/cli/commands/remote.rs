use super::CliContext;
use crate::cli::RemoteCommands;
use anyhow::Result;

pub async fn handle(command: RemoteCommands, ctx: &CliContext) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;
    match command {
        RemoteCommands::Add {
            package,
            provider,
            locator,
        } => {
            let remote = super::parse_package_remote(&provider, &locator)?;
            let providers = if remote.provider() == "file" {
                Vec::new()
            } else {
                super::create_instance_providers(
                    &instance_dir,
                    Some(remote.provider()),
                    &ctx.runtime,
                )?
            };
            let report = orbit_core::add_package_remote(
                &instance_dir,
                &package,
                remote,
                &providers,
                ctx.runtime.jar_cache(),
                ctx.dry_run,
                super::operation_progress(ctx),
            )
            .await?;
            print_report(&report, ctx.dry_run);
        }
        RemoteCommands::Remove {
            package,
            provider,
            locator,
            index,
        } => {
            let remote = if let Some(index) = index {
                let listed = orbit_core::list_package_remotes(&instance_dir, &package)?;
                if index == 0 || index > listed.remotes.len() {
                    anyhow::bail!(
                        "remote index {index} is out of range for package '{package}' (1..={})",
                        listed.remotes.len()
                    );
                }
                listed.remotes[index - 1].clone()
            } else {
                let provider = provider.as_deref().ok_or_else(|| {
                    anyhow::anyhow!("remote remove requires PROVIDER LOCATOR or --index")
                })?;
                let locator = locator.as_deref().ok_or_else(|| {
                    anyhow::anyhow!("remote remove requires PROVIDER LOCATOR or --index")
                })?;
                super::parse_package_remote(provider, locator)?
            };
            let report =
                orbit_core::remove_package_remote(&instance_dir, &package, &remote, ctx.dry_run)?;
            print_report(&report, ctx.dry_run);
        }
        RemoteCommands::List { package } => {
            let report = orbit_core::list_package_remotes(&instance_dir, &package)?;
            print_report(&report, false);
        }
    }
    Ok(())
}

fn print_report(report: &orbit_core::RemoteReport, dry_run: bool) {
    let action = if dry_run { "Would keep" } else { "Package has" };
    println!(
        "{action} {} remote(s) for {}:",
        report.remotes.len(),
        report.package
    );
    for (index, remote) in report.remotes.iter().enumerate() {
        println!("  {}. {}", index + 1, remote.display_locator());
    }
}
