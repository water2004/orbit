//! Validates an already-installed mod set without consulting remote providers.

use std::collections::{HashMap, HashSet};

use pubgrub::Ranges;

use crate::identification::IdentifiedMod;
use crate::manifest::OrbitManifest;
use crate::resolver::diagnostics::describe_no_solution;
use crate::resolver::graph::{ROOT_PACKAGE, register_platform_packages};
use crate::resolver::provider::OrbitDependencyProvider;
use crate::versions::Version;

pub(crate) fn check_local_graph(
    manifest: &OrbitManifest,
    local_mods: &[IdentifiedMod],
) -> Result<(), String> {
    let loader = &manifest.project.modloader;
    let mut provider = OrbitDependencyProvider::new();
    register_platform_packages(&mut provider, manifest);

    let mut local_versions = HashMap::new();
    for local_mod in local_mods {
        let package = package_name(local_mod);
        let version = Version::parse(&local_mod.version, loader);
        let dependencies = local_mod
            .deps
            .iter()
            .filter(|(package, _, required)| {
                *required && package != "java" && package != "mixinextras"
            })
            .map(|(package, constraint, _)| {
                (
                    package.clone(),
                    Version::parse_constraint(constraint, loader),
                )
            })
            .collect();

        provider.add_package_versions(package.clone(), vec![version.clone()]);
        provider.add_package_deps(package.clone(), version.clone(), dependencies);
        local_versions.insert(package, version);
    }

    register_missing_dependencies(&mut provider, manifest, local_mods);

    let root_package = ROOT_PACKAGE.to_string();
    let root_version = Version::zero();
    let root_dependencies = manifest
        .dependencies
        .keys()
        .map(|package| {
            let constraint = local_versions
                .get(package)
                .cloned()
                .map(Ranges::singleton)
                .unwrap_or_else(Ranges::full);
            (package.clone(), constraint)
        })
        .collect();
    provider.add_package_versions(root_package.clone(), vec![root_version.clone()]);
    provider.add_package_deps(
        root_package.clone(),
        root_version.clone(),
        root_dependencies,
    );

    match pubgrub::resolve(&provider, root_package, root_version) {
        Ok(_) => Ok(()),
        Err(pubgrub::PubGrubError::NoSolution(derivation_tree)) => {
            Err(describe_no_solution(&derivation_tree))
        }
        Err(pubgrub::PubGrubError::ErrorChoosingVersion { source, .. })
        | Err(pubgrub::PubGrubError::ErrorRetrievingDependencies { source, .. }) => {
            Err(format!("internal resolver error: {source}"))
        }
        Err(error) => Err(error.to_string()),
    }
}

fn register_missing_dependencies(
    provider: &mut OrbitDependencyProvider,
    manifest: &OrbitManifest,
    local_mods: &[IdentifiedMod],
) {
    let mut missing = HashSet::new();
    for local_mod in local_mods {
        for (package, _, required) in &local_mod.deps {
            if *required
                && package != "java"
                && package != "mixinextras"
                && !provider.versions.contains_key(package)
            {
                missing.insert(package.clone());
            }
        }
    }
    for package in manifest.dependencies.keys() {
        if !provider.versions.contains_key(package) {
            missing.insert(package.clone());
        }
    }
    for package in missing {
        provider.add_package_versions(package, Vec::new());
    }
}

fn package_name(local_mod: &IdentifiedMod) -> String {
    if !local_mod.mod_id.is_empty() {
        local_mod.mod_id.clone()
    } else if !local_mod.mod_name.is_empty() {
        local_mod.mod_name.clone()
    } else {
        local_mod.filename.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_dependency_missing_from_local_mods_is_an_error() {
        let manifest: OrbitManifest = toml::from_str(
            r#"
[project]
name = "test"
mc_version = "1.21.1"
modloader = "fabric"
modloader_version = "0.16.0"

[dependencies]
missing-mod = "*"
"#,
        )
        .unwrap();

        let error = check_local_graph(&manifest, &[]).unwrap_err();

        assert!(error.starts_with("dependency resolution failed"));
        assert!(error.contains("missing-mod"));
    }
}
