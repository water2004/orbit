use super::{LoaderDetector, LoaderInfo};
use crate::error::OrbitError;
use crate::metadata::ModLoader;

pub struct QuiltDetector;

impl LoaderDetector for QuiltDetector {
    fn name(&self) -> &'static str {
        "Quilt"
    }

    fn loader_type(&self) -> ModLoader {
        ModLoader::Quilt
    }

    fn detect(&self, instance_dir: &std::path::Path) -> Result<LoaderInfo, OrbitError> {
        super::profile::detect_profile_loader(
            instance_dir,
            ModLoader::Quilt,
            &super::profile::ProfileSignature {
                group: "org.quiltmc",
                artifacts: &["quilt-loader"],
                main_class_markers: &["quiltmc"],
            },
        )
    }
}
