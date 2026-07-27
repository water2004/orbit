use super::AuditBackend;
use crate::jar::{RuntimeAbiProfile, ScannedArtifacts};
use crate::mixin_config::{MixinDiscoveryPlan, MixinRegistry};
use crate::model::{AuditRequest, LoaderFamily, NamespaceReport, Readiness};
use crate::progress::AuditProgressReporter;
use crate::transformer::TransformerAnalysis;

pub(super) static BACKEND: ForgeBackend = ForgeBackend;

pub(super) struct ForgeBackend;

impl AuditBackend for ForgeBackend {
    fn loader(&self) -> LoaderFamily {
        LoaderFamily::Forge
    }

    fn probe_readiness(&self, scanned: &ScannedArtifacts) -> Readiness {
        crate::jar::probe_runtime_abi(scanned, LoaderFamily::Forge, RuntimeAbiProfile::ModLauncher)
    }

    fn align_namespace(
        &self,
        scanned: &mut ScannedArtifacts,
    ) -> Result<NamespaceReport, Readiness> {
        crate::namespace::align_modlauncher_runtime(scanned, LoaderFamily::Forge)
    }

    fn discover_mixins(
        &self,
        scanned: &mut ScannedArtifacts,
        request: &AuditRequest,
    ) -> MixinRegistry {
        crate::mixin_config::discover(scanned, request, MixinDiscoveryPlan::FORGE)
    }

    fn analyze_transformers(
        &self,
        scanned: &mut ScannedArtifacts,
        progress: Option<&AuditProgressReporter>,
    ) -> TransformerAnalysis {
        crate::transformer::analyze_modlauncher_with_progress(scanned, progress)
    }
}
