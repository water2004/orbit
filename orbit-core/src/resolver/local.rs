//! Validates an already-installed mod set through the shared solver graph.

use std::collections::HashMap;

use crate::identification::IdentifiedMod;
use crate::lockfile::{LockMeta, OrbitLockfile, PackageEntry};
use crate::manifest::OrbitManifest;
use crate::resolver::diagnostics::describe_no_solution;
use crate::resolver::graph::build_solver_graph;

pub(crate) fn check_local_graph(
    manifest: &OrbitManifest,
    local_mods: &[IdentifiedMod],
) -> Result<(), String> {
    let lockfile = OrbitLockfile {
        meta: LockMeta {
            mc_version: manifest.project.mc_version.clone(),
            modloader: manifest.project.modloader.clone(),
            modloader_version: manifest.project.modloader_version.clone(),
        },
        packages: local_mods.iter().map(package_entry).collect(),
    };
    let graph = build_solver_graph(manifest, &lockfile, &HashMap::new());

    match pubgrub::resolve(&graph.provider, graph.root_package, graph.root_version) {
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

fn package_entry(local_mod: &IdentifiedMod) -> PackageEntry {
    PackageEntry {
        mod_id: package_name(local_mod),
        version: local_mod.version.clone(),
        sha1: local_mod.sha1.clone(),
        sha256: local_mod.sha256.clone(),
        sha512: local_mod.sha512.clone(),
        filename: local_mod.filename.clone(),
        provider: "file".to_string(),
        modrinth: None,
        curseforge: None,
        file: None,
        dependencies: local_mod.dependencies.clone(),
        environment: local_mod.environment,
        provides: local_mod.provides.clone(),
        language_loader: local_mod.language_loader.clone(),
        embedded_artifacts: local_mod.embedded_artifacts.clone(),
        bundled: local_mod.bundled.clone(),
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
    use crate::metadata::{DependencyExpression, ModDependency};

    fn local_mod(
        mod_id: &str,
        version: &str,
        dependencies: Vec<DependencyExpression>,
    ) -> IdentifiedMod {
        IdentifiedMod {
            filename: format!("{mod_id}.jar"),
            mod_id: mod_id.to_string(),
            mod_name: mod_id.to_string(),
            version: version.to_string(),
            sha1: String::new(),
            sha256: String::new(),
            sha512: String::new(),
            source: IdentifiedSource::File {
                path: format!("{mod_id}.jar"),
            },
            dependencies,
            environment: crate::metadata::Environment::Both,
            provides: Vec::new(),
            language_loader: None,
            embedded_artifacts: Vec::new(),
            bundled: Vec::new(),
        }
    }

    fn required(mod_id: &str, requirement: &str) -> DependencyExpression {
        ModDependency::required(mod_id, requirement).into()
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
            local_mod("a", "1", vec![required("b", ">=2")]),
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
        let mods = vec![local_mod("a", "1", vec![required("b", "*")])];

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
