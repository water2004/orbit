use super::{LoaderDetector, LoaderInfo};
use crate::error::OrbitError;
use crate::metadata::ModLoader;

pub struct NeoForgeDetector;

impl LoaderDetector for NeoForgeDetector {
    fn name(&self) -> &'static str {
        "NeoForge"
    }

    fn loader_type(&self) -> ModLoader {
        ModLoader::NeoForge
    }

    fn detect(&self, instance_dir: &std::path::Path) -> Result<LoaderInfo, OrbitError> {
        let mut info = super::profile::detect_profile_loader(
            instance_dir,
            ModLoader::NeoForge,
            &super::profile::ProfileSignature {
                group: "net.neoforged",
                artifacts: &["neoforge", "forge"],
                main_class_markers: &["neoforged"],
            },
        )?;
        info.version = info
            .version
            .map(super::profile::strip_minecraft_version_prefix);
        Ok(info)
    }
}
