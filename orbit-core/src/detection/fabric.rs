//! FabricDetector — 检测 Fabric 加载器环境。

use super::{LoaderDetector, LoaderInfo};
use crate::error::OrbitError;
use crate::metadata::ModLoader;

pub struct FabricDetector;

impl LoaderDetector for FabricDetector {
    fn name(&self) -> &'static str {
        "Fabric"
    }

    fn loader_type(&self) -> ModLoader {
        ModLoader::Fabric
    }

    fn detect(
        &self,
        instance_dir: &std::path::Path,
        mc_version: Option<&str>,
    ) -> Result<LoaderInfo, OrbitError> {
        super::profile::detect_profile_loader(
            instance_dir,
            mc_version,
            ModLoader::Fabric,
            &super::profile::ProfileSignature {
                group: "net.fabricmc",
                artifacts: &["fabric-loader"],
                main_class_markers: &["fabricmc"],
                component_uids: &["net.fabricmc.fabric-loader"],
            },
        )
    }
}
