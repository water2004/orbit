//! Loader-specific policy at the edge of the shared audit pipeline.
//!
//! Scanning, bytecode interpretation, Mixin analysis, and conflict synthesis
//! remain shared. Each backend selects only the runtime ABI, namespace,
//! metadata-registration, and transformer rules owned by its loader.

mod fabric;
mod forge;
mod neoforge;
mod quilt;

use crate::jar::ScannedArtifacts;
use crate::mixin_config::MixinRegistry;
use crate::model::{AuditRequest, LoaderFamily, NamespaceReport, Readiness};
use crate::progress::AuditProgressReporter;
use crate::transformer::TransformerAnalysis;

pub(crate) trait AuditBackend {
    fn loader(&self) -> LoaderFamily;

    fn probe_readiness(&self, scanned: &ScannedArtifacts) -> Readiness;

    fn align_namespace(&self, scanned: &mut ScannedArtifacts)
    -> Result<NamespaceReport, Readiness>;

    fn discover_mixins(
        &self,
        scanned: &mut ScannedArtifacts,
        request: &AuditRequest,
    ) -> MixinRegistry;

    fn analyze_transformers(
        &self,
        scanned: &mut ScannedArtifacts,
        progress: Option<&AuditProgressReporter>,
    ) -> TransformerAnalysis;
}

pub(crate) fn for_loader(loader: LoaderFamily) -> &'static dyn AuditBackend {
    match loader {
        LoaderFamily::Fabric => &fabric::BACKEND,
        LoaderFamily::Quilt => &quilt::BACKEND,
        LoaderFamily::Forge => &forge::BACKEND,
        LoaderFamily::NeoForge => &neoforge::BACKEND,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_loader_has_its_own_backend() {
        for loader in [
            LoaderFamily::Fabric,
            LoaderFamily::Quilt,
            LoaderFamily::Forge,
            LoaderFamily::NeoForge,
        ] {
            assert_eq!(for_loader(loader).loader(), loader);
        }
    }
}
