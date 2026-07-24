use super::{LoaderDetector, LoaderInfo};
use crate::error::OrbitError;
use crate::metadata::ModLoader;

pub struct ForgeDetector;

impl LoaderDetector for ForgeDetector {
    fn name(&self) -> &'static str {
        "Forge"
    }

    fn loader_type(&self) -> ModLoader {
        ModLoader::Forge
    }

    fn detect(&self, instance_dir: &std::path::Path) -> Result<LoaderInfo, OrbitError> {
        let mut info = super::profile::detect_profile_loader(
            instance_dir,
            ModLoader::Forge,
            &super::profile::ProfileSignature {
                group: "net.minecraftforge",
                artifacts: &["forge"],
                main_class_markers: &["minecraftforge"],
            },
        )?;
        info.version = info
            .version
            .map(super::profile::strip_minecraft_version_prefix);
        Ok(info)
    }
}
