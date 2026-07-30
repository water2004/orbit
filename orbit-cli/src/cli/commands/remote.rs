use super::CliContext;
use crate::cli::RemoteCommands;
use anyhow::Result;

use crate::cli::output::{OutputFormat, remote_view};

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
            print_report(&report, "add", ctx);
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
                        "{}",
                        tr!(
                            "Remote index %{index} is out of range for package '%{package}' (1..=%{maximum})",
                            index = index,
                            package = package,
                            maximum = listed.remotes.len()
                        )
                    );
                }
                listed.remotes[index - 1].clone()
            } else {
                let provider = provider.as_deref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "{}",
                        tr!("Remote remove requires PROVIDER LOCATOR or --index")
                    )
                })?;
                let locator = locator.as_deref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "{}",
                        tr!("Remote remove requires PROVIDER LOCATOR or --index")
                    )
                })?;
                super::parse_package_remote(provider, locator)?
            };
            let report =
                orbit_core::remove_package_remote(&instance_dir, &package, &remote, ctx.dry_run)?;
            print_report(&report, "remove", ctx);
        }
        RemoteCommands::List { package } => {
            let report = orbit_core::list_package_remotes(&instance_dir, &package)?;
            print_report(&report, "list", ctx);
        }
    }
    Ok(())
}

fn print_report(report: &orbit_core::RemoteReport, subcommand: &str, ctx: &CliContext) {
    match ctx.output.format {
        OutputFormat::Text => {
            let header = if ctx.dry_run {
                tr!(
                    "Would keep %{count} remote(s) for %{package}:",
                    count = report.remotes.len(),
                    package = report.package
                )
            } else {
                tr!(
                    "Package %{package} has %{count} remote(s):",
                    package = report.package,
                    count = report.remotes.len()
                )
            };
            ctx.print_result_line(format_args!(
                "{}",
                crate::cli::output::remote_list_table(report, Some(&header))
            ));
        }
        OutputFormat::Json => {
            let view = remote_view(report, subcommand);
            ctx.print_json("remote", &view);
        }
    }
}
