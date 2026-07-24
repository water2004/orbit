//! NeoForge metadata parser.

use super::{MetadataParser, ModLoader, ModMetadata};
use crate::error::OrbitError;

pub struct NeoForgeParser;

impl MetadataParser for NeoForgeParser {
    fn target_file(&self) -> &str {
        "META-INF/neoforge.mods.toml"
    }

    fn loader_type(&self) -> ModLoader {
        ModLoader::NeoForge
    }

    fn parse(&self, content: &str) -> Result<ModMetadata, OrbitError> {
        Ok(
            super::forge::parse_for_loader(content, ModLoader::NeoForge, self.target_file())?
                .metadata,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
modId = "optional-api"
type = "optional"
"#;
        let parsed = super::super::forge::parse_for_loader(
            metadata,
            ModLoader::NeoForge,
            "META-INF/neoforge.mods.toml",
        )
        .unwrap();

        assert_eq!(parsed.metadata.loader, ModLoader::NeoForge);
        assert!(parsed.dependencies[0].2);
        assert!(!parsed.dependencies[1].2);
    }
}
