use super::AuditBackend;
use crate::jar::{RuntimeAbiProfile, ScannedArtifacts};
use crate::mixin_config::{MixinDiscoveryPlan, MixinRegistry};
use crate::model::{AuditRequest, LoaderFamily, NamespaceReport, Readiness};
use crate::progress::AuditProgressReporter;
use crate::transformer::TransformerAnalysis;

pub(super) static BACKEND: QuiltBackend = QuiltBackend;

pub(super) struct QuiltBackend;

impl AuditBackend for QuiltBackend {
    fn loader(&self) -> LoaderFamily {
        LoaderFamily::Quilt
    }

    fn probe_readiness(&self, scanned: &ScannedArtifacts) -> Readiness {
        crate::jar::probe_runtime_abi(scanned, LoaderFamily::Quilt, RuntimeAbiProfile::MixinOnly)
    }

    fn align_namespace(
        &self,
        scanned: &mut ScannedArtifacts,
    ) -> Result<NamespaceReport, Readiness> {
        crate::namespace::align_fabric_runtime(scanned, LoaderFamily::Quilt)
    }

    fn discover_mixins(
        &self,
        scanned: &mut ScannedArtifacts,
        request: &AuditRequest,
    ) -> MixinRegistry {
        crate::mixin_config::discover(scanned, request, MixinDiscoveryPlan::QUILT)
    }

    fn analyze_transformers(
        &self,
        _scanned: &mut ScannedArtifacts,
        progress: Option<&AuditProgressReporter>,
    ) -> TransformerAnalysis {
        crate::transformer::skip_with_progress(progress)
    }
}
