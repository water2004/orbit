use super::AuditBackend;
use crate::jar::{RuntimeAbiProfile, ScannedArtifacts};
use crate::mixin_config::{MixinDiscoveryPlan, MixinRegistry};
use crate::model::{AuditRequest, LoaderFamily, NamespaceReport, Readiness};
use crate::progress::AuditProgressReporter;
use crate::transformer::TransformerAnalysis;

pub(super) static BACKEND: NeoForgeBackend = NeoForgeBackend;

pub(super) struct NeoForgeBackend;

impl AuditBackend for NeoForgeBackend {
    fn loader(&self) -> LoaderFamily {
        LoaderFamily::NeoForge
    }

    fn probe_readiness(&self, scanned: &ScannedArtifacts) -> Readiness {
        crate::jar::probe_runtime_abi(
            scanned,
            LoaderFamily::NeoForge,
            RuntimeAbiProfile::ModLauncher,
        )
    }

    fn align_namespace(
        &self,
        scanned: &mut ScannedArtifacts,
        _request: &AuditRequest,
    ) -> Result<NamespaceReport, Readiness> {
        crate::namespace::align_modlauncher_runtime(scanned, LoaderFamily::NeoForge)
    }

    fn discover_mixins(
        &self,
        scanned: &mut ScannedArtifacts,
        request: &AuditRequest,
    ) -> MixinRegistry {
        crate::mixin_config::discover(scanned, request, MixinDiscoveryPlan::NEOFORGE)
    }

    fn analyze_transformers(
        &self,
        scanned: &mut ScannedArtifacts,
        progress: Option<&AuditProgressReporter>,
    ) -> TransformerAnalysis {
        crate::transformer::analyze_modlauncher_with_progress(scanned, progress)
    }
}
