//! Validates an already-installed mod set without consulting remote providers.

use std::collections::HashSet;

use crate::identification::IdentifiedMod;
use crate::manifest::OrbitManifest;
use crate::resolver::diagnostics::describe_no_solution;
use crate::resolver::graph::{
    ROOT_PACKAGE, dependency_constraint, is_ignored_runtime_dependency, manifest_exclusions,
    manifest_overrides, parse_required_dependencies, register_platform_packages,
};
use crate::resolver::provider::OrbitDependencyProvider;
use crate::versions::Version;

pub(crate) fn check_local_graph(
    manifest: &OrbitManifest,
    local_mods: &[IdentifiedMod],
) -> Result<(), String> {
    let loader = &manifest.project.modloader;
    let mut provider = OrbitDependencyProvider::new();
    let exclusions = manifest_exclusions(manifest);
    let overrides = manifest_overrides(manifest);
    register_platform_packages(&mut provider, manifest);

    for local_mod in local_mods {
        let package = package_name(local_mod);
        let version = Version::parse(&local_mod.version, loader);
        let dependencies =
            parse_required_dependencies(&local_mod.deps, &package, loader, &exclusions, &overrides);

        provider.add_package_versions(package.clone(), vec![version.clone()]);
        provider.add_package_deps(package.clone(), version.clone(), dependencies);
    }

    register_missing_dependencies(&mut provider, manifest, local_mods);

    let root_package = ROOT_PACKAGE.to_string();
    let root_version = Version::zero();
    let root_dependencies = manifest
        .dependencies
        .iter()
        .map(|(package, spec)| {
            (
                package.clone(),
                dependency_constraint(
                    package,
                    spec.version_constraint().unwrap_or("*"),
                    loader,
                    &overrides,
                ),
            )
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
                && !is_ignored_runtime_dependency(package)
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
    use crate::identification::IdentifiedSource;

    fn local_mod(mod_id: &str, version: &str, deps: Vec<(String, String, bool)>) -> IdentifiedMod {
        IdentifiedMod {
            filename: format!("{mod_id}.jar"),
            mod_id: mod_id.to_string(),
            mod_name: mod_id.to_string(),
            version: version.to_string(),
            modrinth_version: String::new(),
            sha1: String::new(),
            sha256: String::new(),
            sha512: String::new(),
            source: IdentifiedSource::File {
                path: format!("{mod_id}.jar"),
            },
            deps,
        }
    }

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

    #[test]
    fn local_graph_uses_the_same_override_rules_as_candidate_resolution() {
        let manifest: OrbitManifest = toml::from_str(
            r#"
[project]
name = "test"
mc_version = "1.21.1"
modloader = "fabric"
modloader_version = "0.16.0"

[dependencies]
a = "*"

[overrides]
b = "=1"
"#,
        )
        .unwrap();
        let mods = vec![
            local_mod("a", "1", vec![("b".to_string(), ">=2".to_string(), true)]),
            local_mod("b", "1", Vec::new()),
        ];

        check_local_graph(&manifest, &mods).unwrap();
    }

    #[test]
    fn local_graph_uses_the_same_exclusion_rules_as_candidate_resolution() {
        let manifest: OrbitManifest = toml::from_str(
            r#"
[project]
name = "test"
mc_version = "1.21.1"
modloader = "fabric"
modloader_version = "0.16.0"

[dependencies]
a = { version = "*", exclude = ["b"] }
"#,
        )
        .unwrap();
        let mods = vec![local_mod(
            "a",
            "1",
            vec![("b".to_string(), "*".to_string(), true)],
        )];

        check_local_graph(&manifest, &mods).unwrap();
    }

    #[test]
    fn local_graph_checks_manifest_version_constraints() {
        let manifest: OrbitManifest = toml::from_str(
            r#"
[project]
name = "test"
mc_version = "1.21.1"
modloader = "fabric"
modloader_version = "0.16.0"

[dependencies]
a = ">=2"
"#,
        )
        .unwrap();
        let mods = vec![local_mod("a", "1", Vec::new())];

        let error = check_local_graph(&manifest, &mods).unwrap_err();

        assert!(error.starts_with("dependency resolution failed"));
        assert!(error.contains('a'));
    }
}
