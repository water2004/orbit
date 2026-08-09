//! Data-driven launcher profile detection.
//!
//! Every loader traverses the same launcher/layout/profile pipeline. The
//! registry below contains only format evidence and normalization facts. Exact
//! dedicated-server formats remain isolated in `server::formats` and produce
//! the same normalized `LoaderInfo`/runtime model after parsing.

mod profile;
pub(crate) mod server;

use crate::error::OrbitError;
use crate::metadata::LoaderKind;

#[derive(Debug, Clone)]
pub struct LoaderInfo {
    pub loader: LoaderKind,
    pub versions: Vec<String>,
    pub confidence: Confidence,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    None = 0,
    Low = 1,
    Certain = 2,
}

#[derive(Debug, Clone, Copy)]
pub struct ProfileDetector {
    loader: LoaderKind,
}

impl ProfileDetector {
    pub fn name(self) -> &'static str {
        orbit_compatibility::loader::launcher_identity(self.loader).display_name
    }

    pub fn loader_type(self) -> LoaderKind {
        self.loader
    }

    pub fn detect(
        self,
        instance_dir: &std::path::Path,
        mc_version: Option<&str>,
    ) -> Result<LoaderInfo, OrbitError> {
        let identity = orbit_compatibility::loader::launcher_identity(self.loader);
        let mut info =
            profile::detect_profile_loader(instance_dir, mc_version, self.loader, &identity)?;
        if identity.coordinate_may_prefix_minecraft {
            info.versions = info
                .versions
                .into_iter()
                .map(|version| {
                    orbit_compatibility::normalize_launcher_loader_version(&version, None)
                        .into_owned()
                })
                .collect();
            info.versions.sort();
            info.versions.dedup();
        }
        Ok(info)
    }
}

pub struct LoaderDetectionService;

impl LoaderDetectionService {
    pub fn new() -> Self {
        Self
    }

    pub fn detect_all(
        &self,
        instance_dir: &std::path::Path,
        mc_version: Option<&str>,
    ) -> Result<Vec<LoaderInfo>, OrbitError> {
        let mut results = LoaderKind::ALL
            .into_iter()
            .map(|loader| ProfileDetector { loader }.detect(instance_dir, mc_version))
            .collect::<Result<Vec<_>, _>>()?;
        results.sort_by(|left, right| right.confidence.cmp(&left.confidence));
        Ok(results)
    }

    pub fn known_loaders(&self) -> Vec<(LoaderKind, &'static str)> {
        LoaderKind::ALL
            .into_iter()
            .map(|loader| {
                let detector = ProfileDetector { loader };
                (detector.loader_type(), detector.name())
            })
            .collect()
    }

    pub fn find_by_kind(&self, loader: LoaderKind) -> Option<ProfileDetector> {
        LoaderKind::ALL
            .contains(&loader)
            .then_some(ProfileDetector { loader })
    }
}

impl Default for LoaderDetectionService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_one_detector_for_every_loader() {
        let service = LoaderDetectionService::new();
        let registered = service
            .known_loaders()
            .into_iter()
            .map(|(loader, _)| loader)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(registered, LoaderKind::ALL.into_iter().collect());
        assert_eq!(registered.len(), LoaderKind::ALL.len());
    }
}
