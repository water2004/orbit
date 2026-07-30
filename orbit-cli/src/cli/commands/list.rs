use super::CliContext;
use anyhow::{Context, Result};
use orbit_core::{list_installed, list_installed_for_target};
use std::collections::{HashMap, HashSet};

use crate::cli::output::{OutputFormat, list_view};

pub async fn handle(tree: bool, target: Option<String>, ctx: &CliContext) -> Result<()> {
    let dir = ctx.instance_dir()?;
    let output = match target.as_deref() {
        Some(target) => list_installed_for_target(&dir, target),
        None => list_installed(&dir),
    }
    .with_context(|| tr!("Failed to read installed packages").into_owned())?;

    if output.packages.is_empty() {
        if ctx.output.format == OutputFormat::Text {
            ctx.print_result_line(format_args!("{}", tr!("No mods installed.")));
        } else {
            ctx.print_json(
                "list",
                &list_view(
                    &output.packages,
                    target.as_deref(),
                    tree,
                    Some(ctx.runtime.paths().cache_dir()),
                ),
            );
        }
        return Ok(());
    }

    if tree {
        print_tree(&output, target.as_deref(), ctx)?;
    } else {
        match ctx.output.format {
            OutputFormat::Text => {
                ctx.print_result_line(format_args!(
                    "{}",
                    crate::cli::output::installed_packages_table(&output.packages)
                ));
            }
            OutputFormat::Json => {
                let view = list_view(
                    &output.packages,
                    target.as_deref(),
                    false,
                    Some(ctx.runtime.paths().cache_dir()),
                );
                ctx.print_json("list", &view);
            }
        }
    }

    Ok(())
}

fn print_tree(
    output: &orbit_core::ListOutput,
    target: Option<&str>,
    ctx: &CliContext,
) -> Result<()> {
    let index: HashMap<&str, &orbit_core::ListedPackage> = output
        .packages
        .iter()
        .map(|p| (p.mod_id.as_str(), p))
        .collect();

    let mut visited = HashSet::new();

    if ctx.output.format == OutputFormat::Json {
        let view = list_view(
            &output.packages,
            target,
            true,
            Some(ctx.runtime.paths().cache_dir()),
        );
        ctx.print_json("list", &view);
        return Ok(());
    }
    if ctx.quiet {
        return Ok(());
    }

    for pkg in &output.packages {
        if !visited.contains(pkg.mod_id.as_str()) {
            print_node(pkg, "", true, &index, &mut visited);
        }
    }

    Ok(())
}

fn print_node(
    pkg: &orbit_core::ListedPackage,
    prefix: &str,
    _is_last: bool,
    index: &HashMap<&str, &orbit_core::ListedPackage>,
    visited: &mut HashSet<String>,
) {
    if !visited.insert(pkg.mod_id.clone()) {
        println!("{prefix}{} v{} (*)", pkg.mod_id, pkg.version);
        return;
    }

    print_package_line(prefix, pkg);

    if !pkg.bundled.is_empty() {
        println!(
            "{prefix}  + {}",
            tr!("%{count} bundled module(s)", count = pkg.bundled.len())
        );
    }

    let deps: Vec<&str> = pkg
        .dependencies
        .iter()
        .filter(|d| index.contains_key(d.as_str()))
        .map(|d| d.as_str())
        .collect();

    for (i, dep_name) in deps.iter().enumerate() {
        let last = i == deps.len() - 1;
        let connector = if last { "  +-- " } else { "  |-- " };
        let child_prefix = format!("{prefix}{}", if last { "      " } else { "  |   " });

        if let Some(child) = index.get(dep_name) {
            print_package_line(&format!("{prefix}{connector}"), child);
            print_children(child, &child_prefix, index, visited);
        }
    }
}

fn print_children(
    pkg: &orbit_core::ListedPackage,
    prefix: &str,
    index: &HashMap<&str, &orbit_core::ListedPackage>,
    visited: &mut HashSet<String>,
) {
    if !visited.insert(pkg.mod_id.clone()) {
        println!("{prefix}(*)");
        return;
    }

    let deps: Vec<&str> = pkg
        .dependencies
        .iter()
        .filter(|d| index.contains_key(d.as_str()))
        .map(|d| d.as_str())
        .collect();

    for (i, dep_name) in deps.iter().enumerate() {
        let last = i == deps.len() - 1;
        let connector = if last { "+-- " } else { "|-- " };
        let child_prefix = format!("{prefix}{}", if last { "    " } else { "|   " });

        if let Some(child) = index.get(dep_name) {
            print_package_line(&format!("{prefix}{connector}"), child);
            print_children(child, &child_prefix, index, visited);
        }
    }
}

fn print_package_line(prefix: &str, package: &orbit_core::ListedPackage) {
    let optional = if package.optional {
        format!(", {}", tr!("optional"))
    } else {
        String::new()
    };
    println!(
        "{prefix}{} v{} [{}] ({}, {}{})",
        package.mod_id,
        package.version,
        package.version_constraint,
        package.remotes.join(", "),
        package.environment,
        optional
    );
}
