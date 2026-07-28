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
    .context("failed to read installed packages")?;

    if output.packages.is_empty() {
        if ctx.output.format == OutputFormat::Text {
            println!("No mods installed.");
        } else {
            crate::cli::output::print_json(
                "list",
                &list_view(&output.packages, target.as_deref(), tree, None),
            );
        }
        return Ok(());
    }

    if tree {
        print_tree(&output, target.as_deref(), ctx)?;
    } else {
        match ctx.output.format {
            OutputFormat::Text => {
                println!(
                    "{}",
                    crate::cli::output::installed_packages_table(&output.packages)
                );
            }
            OutputFormat::Json => {
                let view = list_view(&output.packages, target.as_deref(), false, None);
                crate::cli::output::print_json("list", &view);
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

    let top_level: Vec<&str> = output.roots.iter().map(String::as_str).collect();

    let mut visited = HashSet::new();
    let mut roots = Vec::new();

    if ctx.output.format == OutputFormat::Json {
        let mut text_lines: Vec<String> = Vec::new();
        for &root in &top_level {
            roots.push(root.to_string());
            if let Some(pkg) = index.get(root) {
                collect_tree(pkg, "", true, &index, &mut visited, &mut text_lines);
            } else {
                text_lines.push(format!("{root} (not installed)"));
            }
        }
        let known: HashSet<&str> = top_level.iter().copied().collect();
        for pkg in &output.packages {
            if !known.contains(pkg.mod_id.as_str()) && !visited.contains(pkg.mod_id.as_str()) {
                collect_tree(pkg, "", true, &index, &mut visited, &mut text_lines);
            }
        }
        let view = list_view(&output.packages, target, true, Some(roots));
        crate::cli::output::print_json("list", &view);
        return Ok(());
    }

    for &root in &top_level {
        if let Some(pkg) = index.get(root) {
            print_node(pkg, "", true, &index, &mut visited);
        } else {
            println!("{} (not installed)", root);
        }
    }

    let known: HashSet<&str> = top_level.iter().copied().collect();
    for pkg in &output.packages {
        if !known.contains(pkg.mod_id.as_str()) && !visited.contains(pkg.mod_id.as_str()) {
            print_node(pkg, "", true, &index, &mut visited);
        }
    }

    Ok(())
}

fn collect_tree(
    pkg: &orbit_core::ListedPackage,
    prefix: &str,
    _is_last: bool,
    index: &HashMap<&str, &orbit_core::ListedPackage>,
    visited: &mut HashSet<String>,
    lines: &mut Vec<String>,
) {
    if !visited.insert(pkg.mod_id.clone()) {
        lines.push(format!("{prefix}{} v{} (*)", pkg.mod_id, pkg.version));
        return;
    }
    let optional = if pkg.optional { ", optional" } else { "" };
    lines.push(format!(
        "{prefix}{} v{} ({}, {}{})",
        pkg.mod_id,
        pkg.version,
        pkg.remotes.join(", "),
        pkg.environment,
        optional
    ));
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
            let line = format!(
                "{prefix}{connector}{} v{} ({}, {}{})",
                child.mod_id,
                child.version,
                child.remotes.join(", "),
                child.environment,
                if child.optional { ", optional" } else { "" }
            );
            lines.push(line);
            collect_tree(child, &child_prefix, last, index, visited, lines);
        }
    }
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

    for (name, ver) in &pkg.bundled {
        println!("{prefix}  + bundled: {name} v{ver}");
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
    let optional = if package.optional { ", optional" } else { "" };
    println!(
        "{prefix}{} v{} ({}, {}{})",
        package.mod_id,
        package.version,
        package.remotes.join(", "),
        package.environment,
        optional
    );
}
