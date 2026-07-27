//! Validates an already-installed mod set through the shared solver graph.

use std::collections::HashMap;

use crate::identification::IdentifiedMod;
use crate::lockfile::{LockMeta, OrbitLockfile};
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
        packages: local_mods
            .iter()
            .map(IdentifiedMod::to_package_entry)
            .collect(),
    };
    let graph = build_solver_graph(manifest, &lockfile, &HashMap::new(), None)?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockfile::ArtifactSource;
    use crate::manifest::PackageRemote;
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
            remotes: vec![PackageRemote::File {
                path: format!("{mod_id}.jar"),
            }],
            artifact_sources: vec![ArtifactSource::File {
                path: format!("{mod_id}.jar"),
            }],
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

[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }
runtime_jars = []
physical_environment = "client"

[dependencies]
missing-mod = { version = "*", remotes = [{ type = "file", path = "missing.jar" }] }
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

[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }
runtime_jars = []
physical_environment = "client"

[dependencies]
a = { version = "*", remotes = [{ type = "file", path = "a.jar" }] }

[overrides]
b = { version = "=1" }
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

[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }
runtime_jars = []
physical_environment = "client"

[dependencies]
a = { version = "*", exclude = ["b"], remotes = [{ type = "file", path = "a.jar" }] }
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

[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }
runtime_jars = []
physical_environment = "client"

[dependencies]
a = { version = ">=2", remotes = [{ type = "file", path = "a.jar" }] }
"#,
        )
        .unwrap();
        let mods = vec![local_mod("a", "1", Vec::new())];

        let error = check_local_graph(&manifest, &mods).unwrap_err();

        assert!(error.starts_with("dependency resolution failed"));
        assert!(error.contains('a'));
    }
}
