//! NeoForge metadata parser.

use super::{LoaderKind, MetadataParser, ModFileMetadata};
use crate::error::OrbitError;

pub struct NeoForgeParser;

impl MetadataParser for NeoForgeParser {
    fn target_file(&self) -> &str {
        "META-INF/neoforge.mods.toml"
    }

    fn loader_type(&self) -> LoaderKind {
        LoaderKind::NeoForge
    }

    fn parse(&self, content: &str) -> Result<ModFileMetadata, OrbitError> {
        super::forge::parse_for_loader(content, LoaderKind::NeoForge, self.target_file())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{DependencyExpression, DependencyKind, ModDependency};

    #[test]
    fn recognizes_neoforge_dependency_types() {
        let metadata = r#"
license = "MIT"
[[mods]]
modId = "example"
version = "2"
displayName = "Example"
[[dependencies.example]]
modId = "neoforge"
type = "required"
versionRange = "[21,)"
[[dependencies.example]]
modId = "optional_api"
type = "optional"
"#;
        let parsed = super::super::forge::parse_for_loader(
            metadata,
            LoaderKind::NeoForge,
            "META-INF/neoforge.mods.toml",
        )
        .unwrap();

        assert_eq!(parsed.loader, LoaderKind::NeoForge);
        assert!(matches!(
            parsed.mods[0].dependencies[0],
            DependencyExpression::Only(ModDependency {
                kind: DependencyKind::Required,
                ..
            })
        ));
        assert!(matches!(
            parsed.mods[0].dependencies[1],
            DependencyExpression::Only(ModDependency {
                kind: DependencyKind::Optional,
                ..
            })
        ));
    }
}
