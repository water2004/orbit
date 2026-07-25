//! Local-file frontend for the unified package transaction.
//!
//! A file is a package remote just like a Modrinth or CurseForge project. This
//! module performs only path validation; discovery, candidate collection,
//! PubGrub resolution, confirmation and materialization all use `install_mod`.

use std::path::Path;

use crate::error::OrbitError;
use crate::providers::ModProvider;

use super::{
    InstallInteraction, InstallOptions, InstallReport, InstallTarget, install_to_instance,
};

pub async fn install_local_file_to_instance(
    source: &Path,
    constraint: Option<&str>,
    instance_dir: &Path,
    providers: &[Box<dyn ModProvider>],
    jar_cache: &crate::jar_cache::JarCache,
    options: InstallOptions,
    interaction: InstallInteraction,
) -> Result<InstallReport, OrbitError> {
    if !source
        .extension()
        .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("jar"))
    {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "local mod must be a .jar file: {}",
            source.display()
        )));
    }
    let source = std::fs::canonicalize(source).map_err(|error| {
        OrbitError::Other(anyhow::anyhow!(
            "cannot open local mod {}: {error}",
            source.display()
        ))
    })?;
    if !source.is_file() {
        return Err(OrbitError::Other(anyhow::anyhow!(
            "local mod is not a file: {}",
            source.display()
        )));
    }
    let sha512 = crate::jar::compute_sha512(&source)?;
    let remote = crate::source_store::preserve_if_instance_output(instance_dir, &source, &sha512)?;
    install_to_instance(
        InstallTarget::Remote(remote),
        constraint.unwrap_or("*"),
        instance_dir,
        providers,
        jar_cache,
        options,
        interaction,
    )
    .await
}
