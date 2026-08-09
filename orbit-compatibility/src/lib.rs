//! Shared compatibility facts selected by explicit version ranges.
//!
//! This crate deliberately contains no filesystem, archive, network, or
//! bytecode inspection. Callers normalize their real inputs, select a verified
//! rule here, and then validate the selected capability against the actual
//! artifact shape. A missing rule is an unsupported combination, never a cue
//! to guess or fall back to a nearby loader.

use std::borrow::Cow;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModLoader {
    Fabric,
    Quilt,
    Forge,
    NeoForge,
}

impl ModLoader {
    pub const ALL: [Self; 4] = [Self::Fabric, Self::Quilt, Self::Forge, Self::NeoForge];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fabric => "fabric",
            Self::Quilt => "quilt",
            Self::Forge => "forge",
            Self::NeoForge => "neoforge",
        }
    }
}

impl std::fmt::Display for ModLoader {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unsupported loader '{value}'")]
pub struct ParseLoaderError {
    value: String,
}

impl FromStr for ModLoader {
    type Err = ParseLoaderError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "fabric" => Ok(Self::Fabric),
            "quilt" => Ok(Self::Quilt),
            "forge" => Ok(Self::Forge),
            "neoforge" => Ok(Self::NeoForge),
            _ => Err(ParseLoaderError {
                value: value.to_string(),
            }),
        }
    }
}

/// A numeric version axis used only for published compatibility boundaries.
///
/// Four components cover current Minecraft, Forge, and NeoForge schemes. A
/// qualifier terminates numeric parsing, so `26.2.0.24-beta` and `26.2.0.24`
/// select the same ABI/layout range. Product/package version ordering remains
/// owned by each loader and is intentionally not implemented here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct NumericVersion([u32; 4]);

impl NumericVersion {
    pub const MIN: Self = Self([0; 4]);
    pub const MAX: Self = Self([u32::MAX; 4]);

    pub const fn new(parts: [u32; 4]) -> Self {
        Self(parts)
    }

    pub fn parse(value: &str) -> Option<Self> {
        let mut output = [0_u32; 4];
        let mut count = 0;
        for part in value.trim().split('.') {
            if count == output.len() {
                break;
            }
            let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
            if digits.is_empty() {
                break;
            }
            output[count] = digits.parse().ok()?;
            count += 1;
            if digits.len() != part.len() {
                break;
            }
        }
        (count >= 2).then_some(Self(output))
    }

    pub const fn major(self) -> u32 {
        self.0[0]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionRange {
    pub minimum: NumericVersion,
    pub maximum: NumericVersion,
}

impl VersionRange {
    pub const ANY: Self = Self {
        minimum: NumericVersion::MIN,
        maximum: NumericVersion::MAX,
    };

    pub const fn inclusive(minimum: [u32; 4], maximum: [u32; 4]) -> Self {
        Self {
            minimum: NumericVersion::new(minimum),
            maximum: NumericVersion::new(maximum),
        }
    }

    pub const fn exact(version: [u32; 4]) -> Self {
        Self::inclusive(version, version)
    }

    pub fn contains(self, version: NumericVersion) -> bool {
        self.minimum <= version && version <= self.maximum
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionSelector {
    Any,
    Numeric(VersionRange),
    /// Official snapshots do not have a stable numeric axis. This selector is
    /// deliberately narrow and does not classify arbitrary opaque strings.
    MinecraftSnapshot,
}

impl VersionSelector {
    fn matches(self, value: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Numeric(range) => NumericVersion::parse(value).is_some_and(|v| range.contains(v)),
            Self::MinecraftSnapshot => is_minecraft_snapshot(value),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CompatibilityRule<T: Copy> {
    pub loader: ModLoader,
    pub minecraft: VersionSelector,
    pub loader_version: VersionSelector,
    pub value: T,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CompatibilityError {
    #[error(
        "{loader} {loader_version} on Minecraft {minecraft_version} has no verified {capability} rule"
    )]
    Unsupported {
        capability: &'static str,
        loader: ModLoader,
        minecraft_version: String,
        loader_version: String,
    },
    #[error(
        "{loader} {loader_version} on Minecraft {minecraft_version} matches multiple {capability} rules"
    )]
    Ambiguous {
        capability: &'static str,
        loader: ModLoader,
        minecraft_version: String,
        loader_version: String,
    },
}

pub fn select_rule<T: Copy>(
    capability: &'static str,
    loader: ModLoader,
    minecraft_version: &str,
    loader_version: &str,
    rules: &[CompatibilityRule<T>],
) -> Result<T, CompatibilityError> {
    let normalized_loader =
        normalize_launcher_loader_version(loader_version, Some(minecraft_version));
    let mut matches = rules.iter().filter(|rule| {
        rule.loader == loader
            && rule.minecraft.matches(minecraft_version)
            && rule.loader_version.matches(&normalized_loader)
    });
    let Some(selected) = matches.next() else {
        return Err(CompatibilityError::Unsupported {
            capability,
            loader,
            minecraft_version: minecraft_version.to_string(),
            loader_version: loader_version.to_string(),
        });
    };
    if matches.next().is_some() {
        return Err(CompatibilityError::Ambiguous {
            capability,
            loader,
            minecraft_version: minecraft_version.to_string(),
            loader_version: loader_version.to_string(),
        });
    }
    Ok(selected.value)
}

fn is_minecraft_snapshot(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 6
        && bytes[0].is_ascii_digit()
        && bytes[1].is_ascii_digit()
        && matches!(bytes[2], b'w' | b'W')
        && bytes[3].is_ascii_digit()
        && bytes[4].is_ascii_digit()
        && bytes[5].is_ascii_alphabetic()
}

/// Normalize the Minecraft prefix used by launcher Maven coordinates.
///
/// With an expected Minecraft version only that exact prefix is accepted.
/// Detection code that does not yet know the version may use the conservative
/// numeric-prefix shape; a normal prerelease such as `21.1.0-beta` is left
/// intact because the suffix does not start with a digit.
pub fn normalize_launcher_loader_version<'a>(
    value: &'a str,
    minecraft_version: Option<&str>,
) -> Cow<'a, str> {
    if let Some(minecraft) = minecraft_version {
        return value
            .strip_prefix(minecraft)
            .and_then(|suffix| suffix.strip_prefix('-'))
            .filter(|loader| {
                loader
                    .chars()
                    .next()
                    .is_some_and(|character| character.is_ascii_digit())
            })
            .map(Cow::Borrowed)
            .unwrap_or(Cow::Borrowed(value));
    }
    value
        .split_once('-')
        .filter(|(minecraft, loader)| {
            minecraft.contains('.')
                && minecraft
                    .chars()
                    .all(|character| character.is_ascii_digit() || character == '.')
                && loader
                    .chars()
                    .next()
                    .is_some_and(|character| character.is_ascii_digit())
        })
        .map(|(_, loader)| Cow::Borrowed(loader))
        .unwrap_or(Cow::Borrowed(value))
}

pub mod loader {
    use super::ModLoader;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct LauncherIdentity {
        pub display_name: &'static str,
        pub maven_group: &'static str,
        pub artifacts: &'static [&'static str],
        pub main_class_markers: &'static [&'static str],
        pub component_uids: &'static [&'static str],
        pub coordinate_may_prefix_minecraft: bool,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum DependencyVersionScheme {
        FabricPredicate,
        MavenRange,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum NestedPriorityPolicy {
        ParentOrder,
        Independent,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum PlatformCapabilityScheme {
        MirrorLoader { package: &'static str },
        ForgeMajor,
        NeoForgeFmlOne,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct LoaderSemantics {
        pub version_scheme: DependencyVersionScheme,
        pub nested_priority: NestedPriorityPolicy,
        pub canonical_package: &'static str,
        pub platform_capabilities: PlatformCapabilityScheme,
    }

    const SEMANTICS: &[(ModLoader, LoaderSemantics, LauncherIdentity)] = &[
        (
            ModLoader::Fabric,
            LoaderSemantics {
                version_scheme: DependencyVersionScheme::FabricPredicate,
                nested_priority: NestedPriorityPolicy::ParentOrder,
                canonical_package: "fabricloader",
                platform_capabilities: PlatformCapabilityScheme::MirrorLoader { package: "fabric" },
            },
            LauncherIdentity {
                display_name: "Fabric",
                maven_group: "net.fabricmc",
                artifacts: &["fabric-loader"],
                main_class_markers: &["fabricmc"],
                component_uids: &["net.fabricmc.fabric-loader"],
                coordinate_may_prefix_minecraft: false,
            },
        ),
        (
            ModLoader::Quilt,
            LoaderSemantics {
                version_scheme: DependencyVersionScheme::FabricPredicate,
                nested_priority: NestedPriorityPolicy::Independent,
                canonical_package: "quilt_loader",
                platform_capabilities: PlatformCapabilityScheme::MirrorLoader {
                    package: "quiltloader",
                },
            },
            LauncherIdentity {
                display_name: "Quilt",
                maven_group: "org.quiltmc",
                artifacts: &["quilt-loader"],
                main_class_markers: &["quiltmc"],
                component_uids: &["org.quiltmc.quilt-loader"],
                coordinate_may_prefix_minecraft: false,
            },
        ),
        (
            ModLoader::Forge,
            LoaderSemantics {
                version_scheme: DependencyVersionScheme::MavenRange,
                nested_priority: NestedPriorityPolicy::Independent,
                canonical_package: "forge",
                platform_capabilities: PlatformCapabilityScheme::ForgeMajor,
            },
            LauncherIdentity {
                display_name: "Forge",
                maven_group: "net.minecraftforge",
                artifacts: &["forge"],
                main_class_markers: &["minecraftforge"],
                component_uids: &["net.minecraftforge"],
                coordinate_may_prefix_minecraft: true,
            },
        ),
        (
            ModLoader::NeoForge,
            LoaderSemantics {
                version_scheme: DependencyVersionScheme::MavenRange,
                nested_priority: NestedPriorityPolicy::Independent,
                canonical_package: "neoforge",
                platform_capabilities: PlatformCapabilityScheme::NeoForgeFmlOne,
            },
            LauncherIdentity {
                display_name: "NeoForge",
                maven_group: "net.neoforged",
                artifacts: &["neoforge", "forge"],
                main_class_markers: &["neoforged"],
                component_uids: &["net.neoforged"],
                coordinate_may_prefix_minecraft: true,
            },
        ),
    ];

    pub fn semantics(loader: ModLoader) -> LoaderSemantics {
        SEMANTICS
            .iter()
            .find_map(|(candidate, semantics, _)| (*candidate == loader).then_some(*semantics))
            .expect("every closed ModLoader variant has one semantics row")
    }

    pub fn launcher_identity(loader: ModLoader) -> LauncherIdentity {
        SEMANTICS
            .iter()
            .find_map(|(candidate, _, identity)| (*candidate == loader).then_some(*identity))
            .expect("every closed ModLoader variant has one launcher identity row")
    }
}

pub mod minecraft {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum PackVersionSchema {
        SharedInteger,
        SeparateInteger,
        MajorMinor,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum JavaVersionPolicy {
        ImplicitFeature(u32),
        Declared,
    }

    #[derive(Debug, Clone, Copy)]
    struct WorldVersionRule<T: Copy> {
        minimum: u32,
        maximum: u32,
        value: T,
    }

    #[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
    pub enum MinecraftFormatError {
        #[error("world version {world_version} has no registered {capability} format rule")]
        Unsupported {
            capability: &'static str,
            world_version: u32,
        },
        #[error("world version {world_version} matches multiple {capability} format rules")]
        Ambiguous {
            capability: &'static str,
            world_version: u32,
        },
    }

    // Immutable facts from version.json in Mojang's published server JARs.
    // Gaps between rows contain no published version and remain unsupported.
    const PACK_VERSION_RULES: &[WorldVersionRule<PackVersionSchema>] = &[
        WorldVersionRule {
            minimum: 1913, // 18w47b
            maximum: 2586, // 1.16.5
            value: PackVersionSchema::SharedInteger,
        },
        WorldVersionRule {
            minimum: 2681, // 20w45a
            maximum: 4440, // 1.21.8
            value: PackVersionSchema::SeparateInteger,
        },
        WorldVersionRule {
            minimum: 4534, // 25w31a; first release 1.21.9
            maximum: u32::MAX,
            value: PackVersionSchema::MajorMinor,
        },
    ];

    const JAVA_VERSION_RULES: &[WorldVersionRule<JavaVersionPolicy>] = &[
        WorldVersionRule {
            minimum: 1913,
            maximum: 2713,
            value: JavaVersionPolicy::ImplicitFeature(8),
        },
        WorldVersionRule {
            minimum: 2714, // 21w19a introduced java_version
            maximum: u32::MAX,
            value: JavaVersionPolicy::Declared,
        },
    ];

    pub fn pack_version_schema(
        world_version: u32,
    ) -> Result<PackVersionSchema, MinecraftFormatError> {
        select("pack_version schema", world_version, PACK_VERSION_RULES)
    }

    pub fn java_version_policy(
        world_version: u32,
    ) -> Result<JavaVersionPolicy, MinecraftFormatError> {
        select("java_version policy", world_version, JAVA_VERSION_RULES)
    }

    fn select<T: Copy>(
        capability: &'static str,
        world_version: u32,
        rules: &[WorldVersionRule<T>],
    ) -> Result<T, MinecraftFormatError> {
        let mut matches = rules
            .iter()
            .filter(|rule| rule.minimum <= world_version && world_version <= rule.maximum);
        let Some(selected) = matches.next() else {
            return Err(MinecraftFormatError::Unsupported {
                capability,
                world_version,
            });
        };
        if matches.next().is_some() {
            return Err(MinecraftFormatError::Ambiguous {
                capability,
                world_version,
            });
        }
        Ok(selected.value)
    }
}

pub mod runtime_agent {
    use super::{CompatibilityError, CompatibilityRule, ModLoader, VersionRange, VersionSelector};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum CodeSourceCapability {
        File,
        Union,
    }

    impl CodeSourceCapability {
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::File => "file",
                Self::Union => "union",
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ModuleIdentityCapability {
        Quilt,
    }

    impl ModuleIdentityCapability {
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::Quilt => "quilt",
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Capabilities {
        pub code_sources: &'static [CodeSourceCapability],
        pub module_identity: Option<ModuleIdentityCapability>,
        pub system_library_property: Option<&'static str>,
        pub java_range: [u32; 2],
    }

    const JAVA_8_TO_25: [u32; 2] = [8, 25];
    const FILE: &[CodeSourceCapability] = &[CodeSourceCapability::File];
    const FILE_AND_UNION: &[CodeSourceCapability] =
        &[CodeSourceCapability::File, CodeSourceCapability::Union];
    const ANY_MC: VersionSelector = VersionSelector::Any;

    const RULES: &[CompatibilityRule<Capabilities>] = &[
        rule(
            ModLoader::Fabric,
            [0, 4, 0, 0],
            [0, 19, u32::MAX, u32::MAX],
            Capabilities {
                code_sources: FILE,
                module_identity: None,
                system_library_property: Some("fabric.systemLibraries"),
                java_range: JAVA_8_TO_25,
            },
        ),
        rule(
            ModLoader::Quilt,
            [0, 12, 0, 0],
            [0, 17, u32::MAX, u32::MAX],
            Capabilities {
                code_sources: FILE,
                module_identity: None,
                system_library_property: Some("loader.systemLibraries"),
                java_range: JAVA_8_TO_25,
            },
        ),
        rule(
            ModLoader::Quilt,
            [0, 18, 0, 0],
            [0, 30, u32::MAX, u32::MAX],
            Capabilities {
                code_sources: FILE,
                module_identity: Some(ModuleIdentityCapability::Quilt),
                system_library_property: Some("loader.systemLibraries"),
                java_range: JAVA_8_TO_25,
            },
        ),
        rule(
            ModLoader::Forge,
            [14, 0, 0, 0],
            [36, u32::MAX, u32::MAX, u32::MAX],
            Capabilities {
                code_sources: FILE,
                module_identity: None,
                system_library_property: None,
                java_range: JAVA_8_TO_25,
            },
        ),
        rule(
            ModLoader::Forge,
            [37, 0, 0, 0],
            [64, u32::MAX, u32::MAX, u32::MAX],
            Capabilities {
                code_sources: FILE_AND_UNION,
                module_identity: None,
                system_library_property: None,
                java_range: JAVA_8_TO_25,
            },
        ),
        rule(
            ModLoader::NeoForge,
            [20, 2, 0, 0],
            [21, 11, u32::MAX, u32::MAX],
            Capabilities {
                code_sources: FILE_AND_UNION,
                module_identity: None,
                system_library_property: None,
                java_range: JAVA_8_TO_25,
            },
        ),
        rule(
            ModLoader::NeoForge,
            [26, 1, 0, 0],
            [26, 2, u32::MAX, u32::MAX],
            Capabilities {
                code_sources: FILE_AND_UNION,
                module_identity: None,
                system_library_property: None,
                java_range: JAVA_8_TO_25,
            },
        ),
        rule(
            ModLoader::NeoForge,
            [47, 1, 0, 0],
            [47, 1, u32::MAX, u32::MAX],
            Capabilities {
                code_sources: FILE_AND_UNION,
                module_identity: None,
                system_library_property: None,
                java_range: JAVA_8_TO_25,
            },
        ),
    ];

    const fn rule(
        loader: ModLoader,
        minimum: [u32; 4],
        maximum: [u32; 4],
        value: Capabilities,
    ) -> CompatibilityRule<Capabilities> {
        CompatibilityRule {
            loader,
            minecraft: ANY_MC,
            loader_version: VersionSelector::Numeric(VersionRange::inclusive(minimum, maximum)),
            value,
        }
    }

    pub fn select(
        loader: ModLoader,
        minecraft_version: &str,
        loader_version: &str,
    ) -> Result<Capabilities, CompatibilityError> {
        super::select_rule(
            "runtime observation",
            loader,
            minecraft_version,
            loader_version,
            RULES,
        )
    }
}

pub mod audit {
    use super::{CompatibilityError, CompatibilityRule, ModLoader, VersionRange, VersionSelector};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum RuntimeAbi {
        Mixin,
        FmlTransformation,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum NamespaceStrategy {
        Fabric,
        Quilt,
        ModLauncher,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum MixinRegistration {
        Fabric,
        Quilt,
        Forge,
        NeoForge,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum TransformerStrategy {
        None,
        Fml,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Capabilities {
        pub runtime_abi: RuntimeAbi,
        pub namespace: NamespaceStrategy,
        pub mixin_registration: MixinRegistration,
        pub transformers: TransformerStrategy,
    }

    const ANY_MC: VersionSelector = VersionSelector::Any;
    const FABRIC: Capabilities = Capabilities {
        runtime_abi: RuntimeAbi::Mixin,
        namespace: NamespaceStrategy::Fabric,
        mixin_registration: MixinRegistration::Fabric,
        transformers: TransformerStrategy::None,
    };
    const QUILT: Capabilities = Capabilities {
        runtime_abi: RuntimeAbi::Mixin,
        namespace: NamespaceStrategy::Quilt,
        mixin_registration: MixinRegistration::Quilt,
        transformers: TransformerStrategy::None,
    };
    const FORGE: Capabilities = Capabilities {
        runtime_abi: RuntimeAbi::FmlTransformation,
        namespace: NamespaceStrategy::ModLauncher,
        mixin_registration: MixinRegistration::Forge,
        transformers: TransformerStrategy::Fml,
    };
    const NEOFORGE: Capabilities = Capabilities {
        runtime_abi: RuntimeAbi::FmlTransformation,
        namespace: NamespaceStrategy::ModLauncher,
        mixin_registration: MixinRegistration::NeoForge,
        transformers: TransformerStrategy::Fml,
    };

    const RULES: &[CompatibilityRule<Capabilities>] = &[
        rule(
            ModLoader::Fabric,
            [0, 4, 0, 0],
            [0, 19, u32::MAX, u32::MAX],
            FABRIC,
        ),
        rule(
            ModLoader::Quilt,
            [0, 12, 0, 0],
            [0, 30, u32::MAX, u32::MAX],
            QUILT,
        ),
        // Selecting a pipeline is not declaring readiness. The actual ABI
        // probe recognizes and rejects the Forge 14-36 LaunchWrapper line with
        // a precise diagnostic instead of a generic missing-range error.
        rule(
            ModLoader::Forge,
            [14, 0, 0, 0],
            [36, u32::MAX, u32::MAX, u32::MAX],
            FORGE,
        ),
        rule(
            ModLoader::Forge,
            [37, 0, 0, 0],
            [64, u32::MAX, u32::MAX, u32::MAX],
            FORGE,
        ),
        rule(
            ModLoader::NeoForge,
            [20, 2, 0, 0],
            [21, 11, u32::MAX, u32::MAX],
            NEOFORGE,
        ),
        rule(
            ModLoader::NeoForge,
            [26, 1, 0, 0],
            [26, 2, u32::MAX, u32::MAX],
            NEOFORGE,
        ),
        rule(
            ModLoader::NeoForge,
            [47, 1, 0, 0],
            [47, 1, u32::MAX, u32::MAX],
            NEOFORGE,
        ),
    ];

    const fn rule(
        loader: ModLoader,
        minimum: [u32; 4],
        maximum: [u32; 4],
        value: Capabilities,
    ) -> CompatibilityRule<Capabilities> {
        CompatibilityRule {
            loader,
            minecraft: ANY_MC,
            loader_version: VersionSelector::Numeric(VersionRange::inclusive(minimum, maximum)),
            value,
        }
    }

    pub fn select(
        loader: ModLoader,
        minecraft_version: &str,
        loader_version: &str,
    ) -> Result<Capabilities, CompatibilityError> {
        super::select_rule(
            "bytecode audit",
            loader,
            minecraft_version,
            loader_version,
            RULES,
        )
    }
}

pub mod neoforge {
    use super::{VersionRange, VersionSelector};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Distribution {
        /// NeoForge's original 1.20.1 releases use net.neoforged:forge 47.1.x.
        LegacyForge,
        /// 1.20.2 through 1.21.x encode Minecraft as major/minor.
        ShortVersion,
        /// 26.x and later encode the complete Minecraft version before build.
        FullMinecraftVersion,
        /// Official snapshot releases use NeoForge's `0.<snapshot>.<build>` form.
        Snapshot,
    }

    #[derive(Debug, Clone, Copy)]
    pub struct Layout {
        distribution: Distribution,
        artifact: &'static str,
        maven_group_path: &'static str,
        minecraft: VersionSelector,
        loader_versions: VersionSelector,
    }

    impl Layout {
        pub const fn distribution(self) -> Distribution {
            self.distribution
        }

        pub const fn artifact(self) -> &'static str {
            self.artifact
        }

        pub const fn maven_group_path(self) -> &'static str {
            self.maven_group_path
        }

        /// Decode a published NeoForge/legacy-NeoForge version using the
        /// scheme selected for the requested Minecraft line. The target line
        /// is returned only when both the structural encoding and any verified
        /// loader-version boundary agree.
        pub fn release_minecraft_version(
            self,
            requested_minecraft: &str,
            release: &str,
        ) -> Option<String> {
            match self.distribution {
                Distribution::LegacyForge => {
                    let prefix = format!("{requested_minecraft}-");
                    let loader = release.strip_prefix(&prefix).unwrap_or(release);
                    self.loader_versions
                        .matches(loader)
                        .then(|| requested_minecraft.to_string())
                }
                Distribution::Snapshot => {
                    let rest = release.strip_prefix("0.")?;
                    let snapshot = rest.split('.').next()?;
                    (!snapshot.is_empty()).then(|| snapshot.to_string())
                }
                Distribution::FullMinecraftVersion => {
                    let numeric = release.split(['-', '+']).next()?;
                    let parts = numeric.split('.').collect::<Vec<_>>();
                    if parts.len() < 4 {
                        return None;
                    }
                    let mut minecraft = parts[..parts.len() - 1].to_vec();
                    if minecraft.last() == Some(&"0") {
                        minecraft.pop();
                    }
                    Some(minecraft.join("."))
                }
                Distribution::ShortVersion => {
                    let numeric = release.split(['-', '+']).next()?;
                    let parts = numeric.split('.').collect::<Vec<_>>();
                    let major = parts.first()?.parse::<u32>().ok()?;
                    let minor = parts.get(1)?.parse::<u32>().ok()?;
                    Some(if minor == 0 {
                        format!("1.{major}")
                    } else {
                        format!("1.{major}.{minor}")
                    })
                }
            }
        }
    }

    const LAYOUTS: &[Layout] = &[
        Layout {
            distribution: Distribution::LegacyForge,
            artifact: "forge",
            maven_group_path: "net/neoforged/forge",
            minecraft: VersionSelector::Numeric(VersionRange::exact([1, 20, 1, 0])),
            loader_versions: VersionSelector::Numeric(VersionRange::inclusive(
                [47, 1, 0, 0],
                [47, 1, u32::MAX, u32::MAX],
            )),
        },
        Layout {
            distribution: Distribution::ShortVersion,
            artifact: "neoforge",
            maven_group_path: "net/neoforged/neoforge",
            minecraft: VersionSelector::Numeric(VersionRange::inclusive(
                [1, 20, 2, 0],
                [1, u32::MAX, u32::MAX, u32::MAX],
            )),
            loader_versions: VersionSelector::Any,
        },
        Layout {
            distribution: Distribution::FullMinecraftVersion,
            artifact: "neoforge",
            maven_group_path: "net/neoforged/neoforge",
            minecraft: VersionSelector::Numeric(VersionRange::inclusive(
                [26, 0, 0, 0],
                [u32::MAX, u32::MAX, u32::MAX, u32::MAX],
            )),
            loader_versions: VersionSelector::Any,
        },
        Layout {
            distribution: Distribution::Snapshot,
            artifact: "neoforge",
            maven_group_path: "net/neoforged/neoforge",
            minecraft: VersionSelector::MinecraftSnapshot,
            loader_versions: VersionSelector::Any,
        },
    ];

    pub fn layout_for_minecraft(minecraft_version: &str) -> Option<Layout> {
        let mut layouts = LAYOUTS
            .iter()
            .filter(|layout| layout.minecraft.matches(minecraft_version));
        let selected = layouts.next().copied()?;
        layouts.next().is_none().then_some(selected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loader_names_roundtrip() {
        for loader in ModLoader::ALL {
            assert_eq!(loader.as_str().parse::<ModLoader>().unwrap(), loader);
            assert_eq!(
                serde_json::from_str::<ModLoader>(&serde_json::to_string(&loader).unwrap())
                    .unwrap(),
                loader
            );
        }
    }

    #[test]
    fn numeric_parser_keeps_four_components_and_ignores_qualifier() {
        assert_eq!(
            NumericVersion::parse("26.2.0.24-beta"),
            Some(NumericVersion::new([26, 2, 0, 24]))
        );
        assert_eq!(NumericVersion::parse("25w14a"), None);
    }

    #[test]
    fn selection_strips_launcher_minecraft_prefix() {
        assert!(runtime_agent::select(ModLoader::NeoForge, "1.20.1", "1.20.1-47.1.106").is_ok());
    }

    #[test]
    fn launcher_prefix_normalization_does_not_damage_prereleases() {
        assert_eq!(
            normalize_launcher_loader_version("1.21.1-52.0.0", None),
            "52.0.0"
        );
        assert_eq!(
            normalize_launcher_loader_version("21.1.0-beta", None),
            "21.1.0-beta"
        );
        assert_eq!(
            normalize_launcher_loader_version("1.20.1-47.1.106", Some("1.20.1")),
            "47.1.106"
        );
    }

    #[test]
    fn unknown_future_lines_are_rejected() {
        assert!(runtime_agent::select(ModLoader::Forge, "27.1", "65.0.0").is_err());
        assert!(audit::select(ModLoader::Fabric, "27.1", "0.20.0").is_err());
    }

    #[test]
    fn registered_loader_boundaries_do_not_bleed_into_adjacent_lines() {
        assert!(runtime_agent::select(ModLoader::Fabric, "1.14", "0.4.0").is_ok());
        assert!(runtime_agent::select(ModLoader::Fabric, "26.2", "0.19.99").is_ok());
        assert!(runtime_agent::select(ModLoader::Fabric, "1.14", "0.3.99").is_err());
        assert!(runtime_agent::select(ModLoader::Fabric, "27.1", "0.20.0").is_err());

        let quilt_old = runtime_agent::select(ModLoader::Quilt, "1.19.2", "0.17.99").unwrap();
        let quilt_new = runtime_agent::select(ModLoader::Quilt, "1.20.1", "0.18.0").unwrap();
        assert_eq!(quilt_old.module_identity, None);
        assert_eq!(
            quilt_new.module_identity,
            Some(runtime_agent::ModuleIdentityCapability::Quilt)
        );

        assert!(runtime_agent::select(ModLoader::Forge, "1.16.5", "36.2.42").is_ok());
        assert!(runtime_agent::select(ModLoader::Forge, "1.17.1", "37.0.0").is_ok());
        assert!(runtime_agent::select(ModLoader::NeoForge, "1.20.1", "47.1.999").is_ok());
        assert!(runtime_agent::select(ModLoader::NeoForge, "1.20.1", "47.2.0").is_err());
    }

    #[test]
    fn minecraft_format_boundaries_and_unpublished_gaps_are_explicit() {
        use minecraft::{JavaVersionPolicy, PackVersionSchema};
        assert_eq!(
            minecraft::pack_version_schema(2586).unwrap(),
            PackVersionSchema::SharedInteger
        );
        assert!(minecraft::pack_version_schema(2600).is_err());
        assert_eq!(
            minecraft::pack_version_schema(2681).unwrap(),
            PackVersionSchema::SeparateInteger
        );
        assert_eq!(
            minecraft::pack_version_schema(4534).unwrap(),
            PackVersionSchema::MajorMinor
        );
        assert_eq!(
            minecraft::java_version_policy(2713).unwrap(),
            JavaVersionPolicy::ImplicitFeature(8)
        );
        assert_eq!(
            minecraft::java_version_policy(2714).unwrap(),
            JavaVersionPolicy::Declared
        );
    }

    #[test]
    fn neoforge_layout_is_selected_by_minecraft_range() {
        use neoforge::Distribution;
        assert_eq!(
            neoforge::layout_for_minecraft("1.20.1").map(|layout| layout.distribution()),
            Some(Distribution::LegacyForge),
        );
        assert_eq!(
            neoforge::layout_for_minecraft("1.21.1").map(|layout| layout.distribution()),
            Some(Distribution::ShortVersion),
        );
        assert_eq!(
            neoforge::layout_for_minecraft("26.2").map(|layout| layout.distribution()),
            Some(Distribution::FullMinecraftVersion),
        );
        assert_eq!(
            neoforge::layout_for_minecraft("25w14craftmine").map(|layout| layout.distribution()),
            Some(Distribution::Snapshot),
        );
        assert!(neoforge::layout_for_minecraft("25.1").is_none());
    }

    #[test]
    fn neoforge_release_decoding_uses_the_selected_layout() {
        let legacy = neoforge::layout_for_minecraft("1.20.1").unwrap();
        assert_eq!(
            legacy.release_minecraft_version("1.20.1", "1.20.1-47.1.106"),
            Some("1.20.1".to_string())
        );
        assert_eq!(legacy.release_minecraft_version("1.20.1", "47.2.0"), None);
        let current = neoforge::layout_for_minecraft("26.2").unwrap();
        assert_eq!(
            current.release_minecraft_version("26.2", "26.2.0.24-beta"),
            Some("26.2".to_string())
        );
    }
}
