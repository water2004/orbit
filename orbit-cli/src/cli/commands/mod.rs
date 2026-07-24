pub mod add;
pub mod cache;
pub mod check;
pub mod export;
pub mod import;
pub mod info;
pub mod init;
pub mod install;
pub mod instances;
pub mod list;
pub mod outdated;
pub mod purge;
pub mod remove;
pub mod search;
pub mod sync;
pub mod upgrade;

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// 全局 CLI 上下文，传递给所有命令 handler。
#[derive(Debug, Clone)]
pub struct CliContext {
    pub verbose: bool,
    pub quiet: bool,
    pub yes: bool,
    pub dry_run: bool,
    pub instance: Option<String>,
}

impl CliContext {
    pub fn instance_dir(&self) -> Result<PathBuf> {
        if let Some(name) = &self.instance {
            let registry = orbit_core::InstancesRegistry::load()
                .context("failed to load instances registry")?;
            let entry = registry.find(name).ok_or_else(|| {
                anyhow::anyhow!(
                    "unknown instance '{name}'. Run 'orbit instances list' to see registered instances."
                )
            })?;
            let path = PathBuf::from(&entry.path);
            if self.verbose && !self.quiet {
                eprintln!("Using instance '{name}' at {}", path.display());
            }
            return Ok(path);
        }

        let path = std::env::current_dir().context("failed to get current directory")?;
        if path.join("orbit.toml").exists() {
            if self.verbose && !self.quiet {
                eprintln!("Using current directory as instance: {}", path.display());
            }
            return Ok(path);
        }

        let registry =
            orbit_core::InstancesRegistry::load().context("failed to load instances registry")?;
        if let Some(instance) = registry.default_instance() {
            let default_path = PathBuf::from(&instance.path);
            if self.verbose && !self.quiet {
                eprintln!(
                    "Using default instance '{}' at {}",
                    instance.name,
                    default_path.display()
                );
            }
            return Ok(default_path);
        }

        if self.verbose && !self.quiet {
            eprintln!(
                "No project or default instance found; using {}",
                path.display()
            );
        }
        Ok(path)
    }

    /// Prevent an instance-mutating command from silently using the global
    /// default while the user is standing in an unrelated directory.
    pub fn require_explicit_mutation_target(&self) -> Result<()> {
        if self.instance.is_some() {
            return Ok(());
        }
        let current_dir = std::env::current_dir().context("failed to get current directory")?;
        if current_dir.join("orbit.toml").exists() {
            return Ok(());
        }
        let registry =
            orbit_core::InstancesRegistry::load().context("failed to load instances registry")?;
        if let Some(instance) = registry.default_instance() {
            anyhow::bail!(
                "refusing to modify the default instance '{}' from outside its project \
                 directory; pass --instance '{}' or change to {}",
                instance.name,
                instance.name,
                instance.path
            );
        }
        Ok(())
    }
}

pub trait CommandHandler {
    async fn execute(self, ctx: &CliContext) -> Result<()>;
}

pub use add::handle as handle_add;
pub use check::handle as handle_check;
pub use export::handle as handle_export;
pub use import::handle as handle_import;
pub use info::handle as handle_info;
pub use init::handle as handle_init;
pub use install::handle as handle_install;
pub use list::handle as handle_list;
pub use outdated::handle as handle_outdated;
pub use purge::handle as handle_purge;
pub use remove::handle as handle_remove;
pub use search::handle as handle_search;
pub use sync::handle as handle_sync;
pub use upgrade::handle as handle_upgrade;

pub fn prompt_install_report(report: &orbit_core::InstallReport, yes: bool) -> bool {
    if report.installed.is_empty() {
        return true;
    }
    eprintln!("\nThe following mods will be installed/upgraded:");
    for m in &report.installed {
        eprintln!("  + {} v{}", m.mod_id, m.version);
        for expression in &m.dependencies {
            for dependency in expression.relations() {
                eprintln!(
                    "      ↳ {} {} ({:?})",
                    dependency.id, dependency.requirement, dependency.kind
                );
            }
        }
        for bundled in &m.bundled {
            print_bundled_mod(bundled, 1);
        }
    }
    if !report.already_satisfied.is_empty() {
        eprintln!(
            "\nAlready satisfied: {}",
            report.already_satisfied.join(", ")
        );
    }
    if yes {
        return true;
    }
    eprint!("\nDo you want to continue? [Y/n] ");
    use std::io::Write;
    std::io::stdout().flush().ok();
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).ok();
    let input = input.trim().to_lowercase();
    input.is_empty() || input == "y" || input == "yes"
}

fn print_bundled_mod(bundled: &orbit_core::BundledMod, depth: usize) {
    let indent = "    ".repeat(depth);
    eprintln!(
        "      ↳ {indent}[bundled] {} {}",
        bundled.mod_id, bundled.version
    );
    for child in &bundled.bundled {
        print_bundled_mod(child, depth + 1);
    }
}

pub fn print_resolution_diagnostics(
    diagnostics: &[orbit_core::resolver::types::CandidateDiagnostic],
) {
    for diagnostic in diagnostics {
        eprintln!("{diagnostic}");
    }
}

pub fn print_resolution_warnings(warnings: &[String]) {
    for warning in warnings {
        eprintln!("warning: {warning}");
    }
}

pub fn create_instance_providers(
    instance_dir: &Path,
    platform: Option<&str>,
) -> Result<Vec<Box<dyn orbit_core::ModProvider>>> {
    let mut platforms = if let Some(platform) = platform {
        vec![normalize_platform(platform).to_string()]
    } else {
        match orbit_core::ManifestFile::open(instance_dir) {
            Ok(manifest) => manifest.inner.resolver.platforms,
            Err(orbit_core::OrbitError::ManifestNotFound) => vec!["modrinth".to_string()],
            Err(error) => return Err(error).context("failed to read orbit.toml"),
        }
    };
    if platform.is_none()
        && let Ok(lockfile) = orbit_core::Lockfile::open(instance_dir)
    {
        for entry in &lockfile.inner.packages {
            if entry.provider != "file" && !platforms.contains(&entry.provider) {
                platforms.push(entry.provider.clone());
            }
        }
    }
    orbit_core::providers::create_providers(&platforms).context("failed to create providers")
}

pub fn resolve_platform_target<'a>(
    input: &'a str,
    requested_platform: Option<&str>,
) -> Result<(Option<String>, &'a str)> {
    let (prefixed_platform, target) = if let Some(target) = input.strip_prefix("mr:") {
        (Some("modrinth"), target)
    } else if let Some(target) = input.strip_prefix("cf:") {
        (Some("curseforge"), target)
    } else {
        (None, input)
    };
    if target.is_empty() {
        anyhow::bail!("platform prefix must be followed by a project slug or ID");
    }

    let requested_platform = requested_platform.map(normalize_platform);
    if let (Some(prefixed), Some(requested)) = (prefixed_platform, requested_platform)
        && prefixed != requested
    {
        anyhow::bail!(
            "'{input}' selects {prefixed}, but --platform selects {requested}; use one platform"
        );
    }
    Ok((
        prefixed_platform
            .or(requested_platform)
            .map(ToString::to_string),
        target,
    ))
}

fn normalize_platform(platform: &str) -> &str {
    match platform {
        "mr" => "modrinth",
        "cf" => "curseforge",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_platform_target;

    #[test]
    fn platform_prefix_selects_one_provider() {
        assert_eq!(
            resolve_platform_target("cf:jei", None).unwrap(),
            (Some("curseforge".to_string()), "jei")
        );
        assert_eq!(
            resolve_platform_target("mr:sodium", Some("modrinth")).unwrap(),
            (Some("modrinth".to_string()), "sodium")
        );
    }

    #[test]
    fn conflicting_platform_selectors_are_rejected() {
        let error = resolve_platform_target("cf:jei", Some("modrinth")).unwrap_err();
        assert!(error.to_string().contains("selects curseforge"));
    }
}
