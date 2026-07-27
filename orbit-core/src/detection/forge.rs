use super::{LoaderDetector, LoaderInfo};
use crate::error::OrbitError;
use crate::metadata::LoaderKind;

pub struct ForgeDetector;

impl LoaderDetector for ForgeDetector {
    fn name(&self) -> &'static str {
        "Forge"
    }

    fn loader_type(&self) -> LoaderKind {
        LoaderKind::Forge
    }

    fn detect(
        &self,
        instance_dir: &std::path::Path,
        mc_version: Option<&str>,
    ) -> Result<LoaderInfo, OrbitError> {
        let mut info = super::profile::detect_profile_loader(
            instance_dir,
            mc_version,
            LoaderKind::Forge,
            &super::profile::ProfileSignature {
                group: "net.minecraftforge",
                artifacts: &["forge"],
                main_class_markers: &["minecraftforge"],
                component_uids: &["net.minecraftforge"],
            },
        )?;
        info.versions = info
            .versions
            .into_iter()
            .map(super::profile::strip_minecraft_version_prefix)
            .collect();
        info.versions.sort();
        info.versions.dedup();
        Ok(info)
    }
}
