use super::{LoaderDetector, LoaderInfo};
use crate::error::OrbitError;
use crate::metadata::LoaderKind;

pub struct QuiltDetector;

impl LoaderDetector for QuiltDetector {
    fn name(&self) -> &'static str {
        "Quilt"
    }

    fn loader_type(&self) -> LoaderKind {
        LoaderKind::Quilt
    }

    fn detect(
        &self,
        instance_dir: &std::path::Path,
        mc_version: Option<&str>,
    ) -> Result<LoaderInfo, OrbitError> {
        super::profile::detect_profile_loader(
            instance_dir,
            mc_version,
            LoaderKind::Quilt,
            &super::profile::ProfileSignature {
                group: "org.quiltmc",
                artifacts: &["quilt-loader"],
                main_class_markers: &["quiltmc"],
                component_uids: &["org.quiltmc.quilt-loader"],
            },
        )
    }
}
