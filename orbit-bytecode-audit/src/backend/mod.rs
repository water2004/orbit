//! One audit pipeline configured by shared, version-ranged compatibility data.
//!
//! Loader differences are values selected at this boundary. Scanning,
//! readiness, namespace alignment, Mixin recovery, transformer recovery, and
//! report construction each execute exactly once in `lib::analyze`; there are
//! no loader-specific orchestration backends.

use orbit_compatibility::audit::{
    Capabilities, MixinRegistration, NamespaceStrategy, RuntimeAbi, TransformerStrategy,
};

use crate::jar::{RuntimeAbiProfile, ScannedArtifacts};
use crate::mixin_config::{MixinDiscoveryPlan, MixinRegistry};
use crate::model::{
    AuditEnvironment, AuditRequest, LoaderFamily, NamespaceReport, Readiness, ReadinessStatus,
};
use crate::progress::AuditProgressReporter;
use crate::transformer::TransformerAnalysis;

#[derive(Debug, Clone, Copy)]
pub(crate) struct AuditPolicy {
    loader: LoaderFamily,
    capabilities: Capabilities,
}

impl AuditPolicy {
    pub(crate) fn select(environment: &AuditEnvironment) -> Result<Self, Readiness> {
        let capabilities = orbit_compatibility::audit::select(
            environment.loader,
            &environment.minecraft_version,
            &environment.loader_version,
        )
        .map_err(|error| Readiness {
            status: ReadinessStatus::Unsupported,
            loader: Some(environment.loader),
            message: error.to_string(),
            capabilities: Vec::new(),
        })?;
        Ok(Self {
            loader: environment.loader,
            capabilities,
        })
    }

    pub(crate) fn probe_readiness(self, scanned: &ScannedArtifacts) -> Readiness {
        let profile = match self.capabilities.runtime_abi {
            RuntimeAbi::Mixin => RuntimeAbiProfile::MixinOnly,
            RuntimeAbi::FmlTransformation => RuntimeAbiProfile::FmlTransformation,
        };
        crate::jar::probe_runtime_abi(scanned, self.loader, profile)
    }

    pub(crate) fn align_namespace(
        self,
        scanned: &mut ScannedArtifacts,
    ) -> Result<NamespaceReport, Readiness> {
        match self.capabilities.namespace {
            NamespaceStrategy::Fabric => crate::namespace::align_fabric_runtime(scanned),
            NamespaceStrategy::Quilt => crate::namespace::align_quilt_runtime(scanned),
            NamespaceStrategy::ModLauncher => {
                crate::namespace::align_modlauncher_runtime(scanned, self.loader)
            }
        }
    }

    pub(crate) fn discover_mixins(
        self,
        scanned: &mut ScannedArtifacts,
        request: &AuditRequest,
    ) -> MixinRegistry {
        let plan = match self.capabilities.mixin_registration {
            MixinRegistration::Fabric => MixinDiscoveryPlan::FABRIC,
            MixinRegistration::Quilt => MixinDiscoveryPlan::QUILT,
            MixinRegistration::Forge => MixinDiscoveryPlan::FORGE,
            MixinRegistration::NeoForge => MixinDiscoveryPlan::NEOFORGE,
        };
        crate::mixin_config::discover(scanned, request, plan)
    }

    pub(crate) fn analyze_transformers(
        self,
        scanned: &mut ScannedArtifacts,
        progress: Option<&AuditProgressReporter>,
    ) -> TransformerAnalysis {
        match self.capabilities.transformers {
            TransformerStrategy::None => crate::transformer::skip_with_progress(progress),
            TransformerStrategy::Fml => {
                crate::transformer::analyze_fml_with_progress(scanned, progress)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PhysicalSide;

    #[test]
    fn verified_loader_lines_select_configuration_for_one_pipeline() {
        for (loader, minecraft, version) in [
            (LoaderFamily::Fabric, "26.2", "0.19.2"),
            (LoaderFamily::Quilt, "1.21.1", "0.27.1"),
            (LoaderFamily::Forge, "1.21.1", "52.1.0"),
            (LoaderFamily::NeoForge, "26.2", "26.2.0.24-beta"),
        ] {
            let environment = AuditEnvironment {
                minecraft_version: minecraft.to_string(),
                loader,
                loader_version: version.to_string(),
                physical_side: PhysicalSide::Unknown,
                java_feature: 21,
            };
            let policy = AuditPolicy::select(&environment).unwrap();
            assert_eq!(policy.loader, loader);
        }
    }

    #[test]
    fn an_unknown_future_line_is_not_routed_to_a_nearby_backend() {
        let environment = AuditEnvironment {
            minecraft_version: "27.1".to_string(),
            loader: LoaderFamily::Fabric,
            loader_version: "0.20.0".to_string(),
            physical_side: PhysicalSide::Unknown,
            java_feature: 25,
        };
        let readiness = AuditPolicy::select(&environment).unwrap_err();
        assert_eq!(readiness.status, ReadinessStatus::Unsupported);
        assert!(
            readiness
                .message
                .contains("no verified bytecode audit rule")
        );
    }
}
