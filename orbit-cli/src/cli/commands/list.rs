use anyhow::{Context, Result};
use orbit_core::{list_installed, OrbitManifest};
use std::collections::{HashMap, HashSet};
use super::CliContext;

pub async fn handle(tree: bool, _target: Option<String>, _ctx: &CliContext) -> Result<()> {
    let dir = std::env::current_dir().context("failed to get current directory")?;
    let output = list_installed(&dir).context("failed to read lockfile")?;

    if output.packages.is_empty() {
        println!("No mods installed.");
        return Ok(());
    }

    if tree {
        print_tree(&dir, &output)?;
    } else {
        print_flat(&output);
    }

    Ok(())
}

fn print_flat(output: &orbit_core::ListOutput) {
    for pkg in &output.packages {
        let slug = pkg.slug.as_deref().unwrap_or(&pkg.mod_id);
        let provider = if pkg.provider == "file" { "file" } else { slug };
        println!("{} v{} ({})", pkg.mod_id, pkg.version, provider);
        for (name, ver) in &pkg.implanted {
            println!("  + embedded: {name} v{ver}");
        }
    }
}

fn print_tree(dir: &std::path::Path, output: &orbit_core::ListOutput) -> Result<()> {
    // 构建 mod_id → Package 的索引
    let index: HashMap<&str, &orbit_core::ListedPackage> = output.packages.iter()
        .map(|p| (p.mod_id.as_str(), p))
        .collect();

    // 找出顶层包：在 manifest 中声明的
    let manifest = OrbitManifest::from_dir(dir)
        .context("failed to read orbit.toml")?;
    let top_level: Vec<&str> = manifest.dependencies.keys()
        .map(|k| k.as_str())
        .collect();

    let mut visited = HashSet::new();

    for &root in &top_level {
        if let Some(pkg) = index.get(root) {
            print_node(pkg, "", true, &index, &mut visited);
        } else {
            println!("{} (not installed)", root);
        }
    }

    // 显示未被任何顶层依赖引用的包（如有）
    let known: HashSet<&str> = top_level.iter().copied().collect();
    for pkg in &output.packages {
        if !known.contains(pkg.mod_id.as_str()) && !visited.contains(pkg.mod_id.as_str()) {
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

    println!("{prefix}{} v{}", pkg.mod_id, pkg.version);

    for (name, ver) in &pkg.implanted {
        println!("{prefix}  + embedded: {name} v{ver}");
    }

    let deps: Vec<&str> = pkg.dependencies.iter()
        .filter(|d| index.contains_key(d.as_str()))
        .map(|d| d.as_str())
        .collect();

    for (i, dep_name) in deps.iter().enumerate() {
        let last = i == deps.len() - 1;
        let connector = if last { "  +-- " } else { "  |-- " };
        let child_prefix = format!("{prefix}{}", if last { "      " } else { "  |   " });

        if let Some(child) = index.get(dep_name) {
            println!("{prefix}{connector}{} v{}", dep_name, child.version);
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

    let deps: Vec<&str> = pkg.dependencies.iter()
        .filter(|d| index.contains_key(d.as_str()))
        .map(|d| d.as_str())
        .collect();

    for (i, dep_name) in deps.iter().enumerate() {
        let last = i == deps.len() - 1;
        let connector = if last { "+-- " } else { "|-- " };
        let child_prefix = format!("{prefix}{}", if last { "    " } else { "|   " });

        if let Some(child) = index.get(dep_name) {
            println!("{prefix}{connector}{} v{}", dep_name, child.version);
            print_children(child, &child_prefix, index, visited);
        }
    }
}
