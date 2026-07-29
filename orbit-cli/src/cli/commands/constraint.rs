use anyhow::Result;

use super::CliContext;
use crate::cli::output::{
    OutputFormat, PackageConstraintOutput, package_version_policy_view, transaction_view,
};
use crate::cli::{BoundInclusion, ConstraintCommands, ConstraintPolicyCommands};

pub async fn handle(command: ConstraintCommands, ctx: &CliContext) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;
    let (output, text_transaction) = match command {
        ConstraintCommands::Show { package } => {
            let report = orbit_core::package_constraint(&instance_dir, &package)?;
            (
                PackageConstraintOutput {
                    package: report.package,
                    previous: None,
                    current: report.constraint,
                    policy: package_version_policy_view(&report.policy),
                    previous_selected_version: None,
                    selected_version: report.selected_version,
                    selected_satisfies: report.selected_satisfies,
                    changed: false,
                    applied: false,
                    dry_run: false,
                    transaction: None,
                },
                None,
            )
        }
        ConstraintCommands::Set { package, policy } => {
            let policy = core_policy(policy);
            let providers = super::create_instance_providers(&instance_dir, None, &ctx.runtime)?;
            let report = orbit_core::apply_package_constraint(
                &instance_dir,
                &package,
                policy,
                &providers,
                ctx.runtime.jar_cache(),
                ctx.dry_run,
                super::install_interaction(ctx),
            )
            .await?;
            let transaction = report.transaction.clone();
            (
                PackageConstraintOutput {
                    package: report.package,
                    previous: Some(report.previous),
                    current: report.current,
                    policy: package_version_policy_view(&report.policy),
                    previous_selected_version: report.previous_selected_version,
                    selected_version: report.selected_version,
                    selected_satisfies: report.selected_satisfies,
                    changed: report.changed,
                    applied: report.applied,
                    dry_run: report.dry_run,
                    transaction: Some(transaction_view(&report.transaction, report.dry_run)),
                },
                Some(transaction),
            )
        }
    };

    match ctx.output.format {
        OutputFormat::Text => print_text(&output, text_transaction.as_ref()),
        OutputFormat::Json => crate::cli::output::print_json("constraint", &output),
    }
    Ok(())
}

fn core_policy(policy: ConstraintPolicyCommands) -> orbit_core::PackageVersionPolicy {
    use orbit_core::{PackageVersionPolicy, VersionComparison};

    match policy {
        ConstraintPolicyCommands::Any => PackageVersionPolicy::Any,
        ConstraintPolicyCommands::Exact { version } => PackageVersionPolicy::Comparison {
            operator: VersionComparison::Exact,
            version,
        },
        ConstraintPolicyCommands::GreaterThan { version } => PackageVersionPolicy::Comparison {
            operator: VersionComparison::GreaterThan,
            version,
        },
        ConstraintPolicyCommands::AtLeast { version } => PackageVersionPolicy::Comparison {
            operator: VersionComparison::AtLeast,
            version,
        },
        ConstraintPolicyCommands::LessThan { version } => PackageVersionPolicy::Comparison {
            operator: VersionComparison::LessThan,
            version,
        },
        ConstraintPolicyCommands::AtMost { version } => PackageVersionPolicy::Comparison {
            operator: VersionComparison::AtMost,
            version,
        },
        ConstraintPolicyCommands::Range {
            lower,
            upper,
            lower_bound,
            upper_bound,
        } => PackageVersionPolicy::Range {
            lower,
            upper,
            include_lower: lower_bound == BoundInclusion::Inclusive,
            include_upper: upper_bound == BoundInclusion::Inclusive,
        },
    }
}

fn print_text(output: &PackageConstraintOutput, transaction: Option<&orbit_core::InstallReport>) {
    let selected = output
        .selected_version
        .clone()
        .unwrap_or_else(|| tr!("not selected").into_owned());
    let status = match output.selected_satisfies {
        Some(true) => tr!("matches policy"),
        Some(false) => tr!("does not match policy"),
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
    let Some(transaction) = transaction else {
        return;
    };
    super::print_resolution_diagnostics(&transaction.diagnostics);
    super::print_resolution_warnings(&transaction.warnings);
    if output.dry_run {
        println!("\n{}", tr!("constraint preview:"));
        println!(
            "{}",
            crate::cli::output::package_changes_table(&transaction.changes)
        );
    } else if output.applied {
        println!(
            "{}",
            tr!(
                "Applied %{installed} selected package version(s) and removed %{removed} unselected package version(s).",
                installed = transaction.installed.len(),
                removed = transaction.removed.len()
            )
        );
    } else {
        println!("{}", tr!("The version policy was not applied."));
    }
}
