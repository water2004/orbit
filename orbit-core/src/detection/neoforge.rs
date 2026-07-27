use super::{LoaderDetector, LoaderInfo};
use crate::error::OrbitError;
use crate::metadata::LoaderKind;

pub struct NeoForgeDetector;

impl LoaderDetector for NeoForgeDetector {
    fn name(&self) -> &'static str {
        "NeoForge"
    }

    fn loader_type(&self) -> LoaderKind {
        LoaderKind::NeoForge
    }

    fn detect(
        &self,
        instance_dir: &std::path::Path,
        mc_version: Option<&str>,
    ) -> Result<LoaderInfo, OrbitError> {
        let mut info = super::profile::detect_profile_loader(
            instance_dir,
            mc_version,
            LoaderKind::NeoForge,
            &super::profile::ProfileSignature {
                group: "net.neoforged",
                artifacts: &["neoforge", "forge"],
                main_class_markers: &["neoforged"],
                component_uids: &["net.neoforged"],
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
