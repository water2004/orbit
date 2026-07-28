use super::AuditBackend;
use crate::jar::{RuntimeAbiProfile, ScannedArtifacts};
use crate::mixin_config::{MixinDiscoveryPlan, MixinRegistry};
use crate::model::{AuditRequest, LoaderFamily, NamespaceReport, Readiness};
use crate::progress::AuditProgressReporter;
use crate::transformer::TransformerAnalysis;

pub(super) static BACKEND: FabricBackend = FabricBackend;

pub(super) struct FabricBackend;

impl AuditBackend for FabricBackend {
    fn loader(&self) -> LoaderFamily {
        LoaderFamily::Fabric
    }

    fn probe_readiness(&self, scanned: &ScannedArtifacts) -> Readiness {
        crate::jar::probe_runtime_abi(scanned, LoaderFamily::Fabric, RuntimeAbiProfile::MixinOnly)
    }

    fn align_namespace(
        &self,
        scanned: &mut ScannedArtifacts,
        _request: &AuditRequest,
    ) -> Result<NamespaceReport, Readiness> {
        crate::namespace::align_fabric_runtime(scanned)
    }

    fn discover_mixins(
        &self,
        scanned: &mut ScannedArtifacts,
        request: &AuditRequest,
    ) -> MixinRegistry {
        crate::mixin_config::discover(scanned, request, MixinDiscoveryPlan::FABRIC)
    }

    fn analyze_transformers(
        &self,
        _scanned: &mut ScannedArtifacts,
        progress: Option<&AuditProgressReporter>,
    ) -> TransformerAnalysis {
        crate::transformer::skip_with_progress(progress)
    }
}
