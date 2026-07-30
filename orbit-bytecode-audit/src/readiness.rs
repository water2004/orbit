use crate::AuditError;
use crate::model::{ArtifactKind, AuditRequest, Readiness, ReadinessStatus};

pub fn probe_readiness(request: &AuditRequest) -> Result<Readiness, AuditError> {
    match preflight(request) {
        Ok(()) => {}
        Err(readiness) => return Ok(readiness),
    }
    let scanned = crate::jar::scan_artifacts_with_progress(request, None)?;
    Ok(crate::backend::for_loader(request.environment.loader).probe_readiness(&scanned))
}

pub(crate) fn preflight(request: &AuditRequest) -> Result<(), Readiness> {
    if !request
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == ArtifactKind::Minecraft && artifact.path.is_file())
    {
        return Err(Readiness {
            status: ReadinessStatus::Incomplete,
            loader: None,
            message: "the actual Minecraft JAR is missing".to_string(),
            capabilities: Vec::new(),
        });
    }
    if !request
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == ArtifactKind::Loader && artifact.path.is_file())
    {
        return Err(Readiness {
            status: ReadinessStatus::Incomplete,
            loader: None,
            message: "the actual loader JAR is missing".to_string(),
            capabilities: Vec::new(),
        });
    }
    if !request
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == ArtifactKind::Mod && artifact.path.is_file())
    {
        return Err(Readiness {
            status: ReadinessStatus::Incomplete,
            loader: None,
            message: "the instance contains no analyzable Mod JAR".to_string(),
            capabilities: Vec::new(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::jar::test_support::{
        class_with_abstract_methods, minimal_class, write_class_entries, write_jar,
    };
    use crate::model::{
        AnalysisLimits, ArtifactInput, ArtifactKind, AuditEnvironment, AuditRequest,
        NestedJarPolicy, PhysicalSide, ReadinessStatus,
    };

    use super::*;

    #[test]
    fn missing_minecraft_jar_is_incomplete() {
        let directory = tempfile::tempdir().unwrap();
        let loader = directory.path().join("loader.jar");
        let mod_jar = directory.path().join("mod.jar");
        write_jar(
            &loader,
            &[
                "net/fabricmc/loader/impl/FabricLoaderImpl",
                "org/spongepowered/asm/mixin/Mixin",
            ],
        );
        write_jar(&mod_jar, &["example/Mod"]);
        let request = request(
            "fabric",
            vec![
                input(
                    "minecraft",
                    directory.path().join("missing.jar"),
                    ArtifactKind::Minecraft,
                ),
                input("loader", loader, ArtifactKind::Loader),
                input("mod", mod_jar, ArtifactKind::Mod),
            ],
        );
        let readiness = probe_readiness(&request).unwrap();
        assert_eq!(readiness.status, ReadinessStatus::Incomplete);
        assert!(readiness.message.contains("Minecraft JAR"));
    }

    #[test]
    fn legacy_launchwrapper_is_rejected_with_the_required_message() {
        let directory = tempfile::tempdir().unwrap();
        let minecraft = directory.path().join("minecraft.jar");
        let loader = directory.path().join("loader.jar");
        let mod_jar = directory.path().join("mod.jar");
        write_jar(&minecraft, &["net/minecraft/client/Minecraft"]);
        write_jar(
            &loader,
            &[
                "net/minecraftforge/fml/loading/FMLLoader",
                "net/minecraft/launchwrapper/Launch",
                "org/spongepowered/asm/mixin/Mixin",
            ],
        );
        write_jar(&mod_jar, &["example/Mod"]);
        let readiness = probe_readiness(&request(
            "forge",
            vec![
                input("minecraft", minecraft, ArtifactKind::Minecraft),
                input("loader", loader, ArtifactKind::Loader),
                input("mod", mod_jar, ArtifactKind::Mod),
            ],
        ))
        .unwrap();
        assert_eq!(readiness.status, ReadinessStatus::Unsupported);
        assert_eq!(
            readiness.message,
            "当前实例使用 Legacy Forge/LaunchWrapper。\n\
             字节码风险分析仅支持现代 FML 的 ModLauncher ITransformer 或 NeoForge ClassProcessor。"
        );
    }

    #[test]
    fn missing_modlauncher_is_incomplete_but_unknown_abi_is_unsupported() {
        let directory = tempfile::tempdir().unwrap();
        let minecraft = directory.path().join("minecraft.jar");
        let loader = directory.path().join("loader.jar");
        let unknown = directory.path().join("unknown.jar");
        let mod_jar = directory.path().join("mod.jar");
        write_jar(&minecraft, &["net/minecraft/client/Minecraft"]);
        write_jar(
            &loader,
            &[
                "net/minecraftforge/fml/loading/FMLLoader",
                "org/spongepowered/asm/mixin/Mixin",
            ],
        );
        write_jar(
            &unknown,
            &[
                "net/minecraftforge/fml/loading/FMLLoader",
                "org/spongepowered/asm/mixin/Mixin",
                "cpw/mods/modlauncher/api/ITransformer",
            ],
        );
        write_jar(&mod_jar, &["example/Mod"]);
        let missing = probe_readiness(&request(
            "forge",
            vec![
                input("minecraft", minecraft.clone(), ArtifactKind::Minecraft),
                input("loader", loader, ArtifactKind::Loader),
                input("mod", mod_jar.clone(), ArtifactKind::Mod),
            ],
        ))
        .unwrap();
        assert_eq!(missing.status, ReadinessStatus::Incomplete);

        let unsupported = probe_readiness(&request(
            "forge",
            vec![
                input("minecraft", minecraft, ArtifactKind::Minecraft),
                input("loader", unknown, ArtifactKind::Loader),
                input("mod", mod_jar, ArtifactKind::Mod),
            ],
        ))
        .unwrap();
        assert_eq!(unsupported.status, ReadinessStatus::Unsupported);
        assert!(unsupported.message.contains("ABI"));
    }

    #[test]
    fn actual_modlauncher_abi_can_be_ready_without_a_version_table() {
        let directory = tempfile::tempdir().unwrap();
        let minecraft = directory.path().join("minecraft.jar");
        let loader = directory.path().join("loader.jar");
        let mod_jar = directory.path().join("mod.jar");
        write_jar(&minecraft, &["net/minecraft/client/Minecraft"]);
        write_jar(&mod_jar, &["example/Mod"]);
        let transformer_methods = [
            ("targets", "()Ljava/util/Set;"),
            (
                "transform",
                "(Ljava/lang/Object;Lcpw/mods/modlauncher/api/ITransformerVotingContext;)Ljava/lang/Object;",
            ),
            (
                "getTargetType",
                "()Lcpw/mods/modlauncher/api/ITransformer$TargetType;",
            ),
            (
                "castVote",
                "(Lcpw/mods/modlauncher/api/ITransformerVotingContext;)Lcpw/mods/modlauncher/api/TransformerVoteResult;",
            ),
        ];
        let target_methods = [
            (
                "targetClass",
                "(Ljava/lang/String;)Lcpw/mods/modlauncher/api/ITransformer$Target;",
            ),
            (
                "targetMethod",
                "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)Lcpw/mods/modlauncher/api/ITransformer$Target;",
            ),
            (
                "targetField",
                "(Ljava/lang/String;Ljava/lang/String;)Lcpw/mods/modlauncher/api/ITransformer$Target;",
            ),
        ];
        write_class_entries(
            &loader,
            vec![
                (
                    "net/minecraftforge/fml/loading/FMLLoader.class".to_string(),
                    minimal_class("net/minecraftforge/fml/loading/FMLLoader"),
                ),
                (
                    "org/spongepowered/asm/mixin/Mixin.class".to_string(),
                    minimal_class("org/spongepowered/asm/mixin/Mixin"),
                ),
                (
                    "cpw/mods/modlauncher/api/ITransformer.class".to_string(),
                    class_with_abstract_methods(
                        "cpw/mods/modlauncher/api/ITransformer",
                        true,
                        &transformer_methods,
                    ),
                ),
                (
                    "cpw/mods/modlauncher/api/ITransformer$Target.class".to_string(),
                    class_with_abstract_methods(
                        "cpw/mods/modlauncher/api/ITransformer$Target",
                        false,
                        &target_methods,
                    ),
                ),
                (
                    "cpw/mods/modlauncher/api/ITransformer$TargetType.class".to_string(),
                    minimal_class("cpw/mods/modlauncher/api/ITransformer$TargetType"),
                ),
                (
                    "cpw/mods/modlauncher/api/ITransformationService.class".to_string(),
                    class_with_abstract_methods(
                        "cpw/mods/modlauncher/api/ITransformationService",
                        true,
                        &[("transformers", "()Ljava/util/List;")],
                    ),
                ),
            ],
        );

        let readiness = probe_readiness(&request(
            "forge",
            vec![
                input("minecraft", minecraft, ArtifactKind::Minecraft),
                input("loader", loader, ArtifactKind::Loader),
                input("mod", mod_jar, ArtifactKind::Mod),
            ],
        ))
        .unwrap();

        assert_eq!(readiness.status, ReadinessStatus::Ready);
        assert!(
            readiness
                .capabilities
                .contains(&"modlauncher_itransformer".to_string())
        );
    }

    #[test]
    fn actual_neoforge_class_processor_spi_is_selected_by_shape() {
        let directory = tempfile::tempdir().unwrap();
        let minecraft = directory.path().join("minecraft.jar");
        let loader = directory.path().join("loader.jar");
        let mod_jar = directory.path().join("mod.jar");
        write_jar(&minecraft, &["net/minecraft/client/Minecraft"]);
        write_jar(&mod_jar, &["example/Mod"]);
        write_class_entries(
            &loader,
            vec![
                (
                    "net/neoforged/fml/loading/FMLLoader.class".to_string(),
                    minimal_class("net/neoforged/fml/loading/FMLLoader"),
                ),
                (
                    "org/spongepowered/asm/mixin/Mixin.class".to_string(),
                    minimal_class("org/spongepowered/asm/mixin/Mixin"),
                ),
                (
                    "net/neoforged/neoforgespi/transformation/ClassProcessor.class".to_string(),
                    class_with_abstract_methods(
                        "net/neoforged/neoforgespi/transformation/ClassProcessor",
                        true,
                        &[
                            (
                                "handlesClass",
                                "(Lnet/neoforged/neoforgespi/transformation/ClassProcessor$SelectionContext;)Z",
                            ),
                            (
                                "processClass",
                                "(Lnet/neoforged/neoforgespi/transformation/ClassProcessor$TransformationContext;)Lnet/neoforged/neoforgespi/transformation/ClassProcessor$ComputeFlags;",
                            ),
                        ],
                    ),
                ),
            ],
        );

        let readiness = probe_readiness(&request(
            "neoforge",
            vec![
                input("minecraft", minecraft, ArtifactKind::Minecraft),
                input("loader", loader, ArtifactKind::Loader),
                input("mod", mod_jar, ArtifactKind::Mod),
            ],
        ))
        .unwrap();

        assert_eq!(readiness.status, ReadinessStatus::Ready);
        assert!(
            readiness
                .capabilities
                .contains(&"neoforge_class_processor".to_string())
        );
    }

    fn request(loader: &str, artifacts: Vec<ArtifactInput>) -> AuditRequest {
        AuditRequest {
            environment: AuditEnvironment {
                minecraft_version: "test".to_string(),
                loader: match loader {
                    "fabric" => crate::model::LoaderFamily::Fabric,
                    "quilt" => crate::model::LoaderFamily::Quilt,
                    "forge" => crate::model::LoaderFamily::Forge,
                    "neoforge" => crate::model::LoaderFamily::NeoForge,
                    _ => panic!("unsupported test loader"),
                },
                loader_version: "test".to_string(),
                physical_side: PhysicalSide::Unknown,
                java_feature: 17,
            },
            artifacts,
            active_mod_ids: Default::default(),
            limits: AnalysisLimits::default(),
        }
    }

    fn input(id: &str, path: std::path::PathBuf, kind: ArtifactKind) -> ArtifactInput {
        ArtifactInput {
            id: id.to_string(),
            display_name: id.to_string(),
            path,
            kind,
            nested_jars: NestedJarPolicy::All,
        }
    }
}
