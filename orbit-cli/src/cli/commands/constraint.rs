use anyhow::Result;

use super::CliContext;
use crate::cli::ConstraintCommands;
use crate::cli::output::{OutputFormat, PackageConstraintOutput};

pub fn handle(command: ConstraintCommands, ctx: &CliContext) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;
    let report = match command {
        ConstraintCommands::Show { package } => {
            orbit_core::package_constraint(&instance_dir, &package)?
        }
        ConstraintCommands::Set {
            package,
            requirement,
        } => {
            orbit_core::set_package_constraint(&instance_dir, &package, &requirement, ctx.dry_run)?
        }
        ConstraintCommands::Clear { package } => {
            orbit_core::set_package_constraint(&instance_dir, &package, "*", ctx.dry_run)?
        }
    };
    let output = PackageConstraintOutput {
        package: report.package,
        previous: report.previous,
        current: report.current,
        selected_version: report.selected_version,
        selected_satisfies: report.selected_satisfies,
        changed: report.changed,
        dry_run: report.dry_run,
    };
    match ctx.output.format {
        OutputFormat::Text => {
            let selected = output
                .selected_version
                .clone()
                .unwrap_or_else(|| tr!("not selected").into_owned());
            let status = match output.selected_satisfies {
                Some(true) => tr!("matches policy"),
                Some(false) => tr!("does not match policy; run 'orbit fix' to resolve it"),
                None => tr!("no lock selection"),
            };
            println!(
                "{}",
                tr!(
                    "%{package}: %{constraint} (selected: %{selected}; %{status})",
                    package = output.package,
                    constraint = output.current,
                    selected = selected,
                    status = status
                )
            );
        }
        OutputFormat::Json => crate::cli::output::print_json("constraint", &output),
    }
    Ok(())
}
