//! Core domain and persistence APIs for `orbit-launcher`.
//!
//! This crate owns launcher business logic and filesystem I/O. It does not
//! render terminal output and has no dependency on Orbit's mod-management
//! crates.

pub mod artifact;
mod atomic_io;
pub mod client;
pub mod config;
pub mod context;
pub mod error;
pub mod eula;
pub mod install;
pub mod instance;
pub mod java;
pub mod loader;
pub mod lockfile;
mod maven;
pub mod mojang;
pub mod operations;
pub mod platform;
pub mod registry;
pub mod runtime;

pub use artifact::{
    ArtifactCache, ArtifactRequest, ArtifactTransferEvent, CachedArtifact, ExpectedHash,
    hash_file_sha256,
};
pub use client::{
    AssetMapping, ClientDownload, NativeExtract, ResolvedVanillaClient, resolve_vanilla_client,
};
pub use config::{
    ConfigEntry, ConfigKey, ConfigMutation, GlobalConfig, JavaProvider, UiPreference, get_config,
    list_config, set_config, unset_config,
};
pub use context::{ContextIntent, ContextSource, ResolvedInstance, resolve_instance};
pub use error::LauncherError;
pub use eula::{
    EulaAcceptance, EulaAcceptanceMethod, EulaDocument, MINECRAFT_EULA_URL, accept_shown_eula,
    require_current_acceptance, show_current_eula,
};
pub use install::{
    InstallProgressEvent, InstallResult, ProfileLoaderInstallPlan, VanillaClientInstallPlan,
    VanillaServerInstallPlan, execute_vanilla_client_install, execute_vanilla_server_install,
    prepare_profile_loader_install, prepare_vanilla_client_install, prepare_vanilla_server_install,
};
pub use instance::{
    INSTANCE_MANIFEST_FILE, InstanceKind, InstanceManifest, JavaPolicy, LoaderKind, ManifestFile,
    RestartPolicy,
};
pub use java::{
    JavaProgressEvent, JavaTarget, MOJANG_RUNTIME_MANIFEST_URL, ManagedJavaRuntime, MojangJavaPlan,
    install_mojang_java, plan_mojang_java,
};
pub use loader::{LoaderSide, ResolvedLoaderProfile, resolve_loader_profile};
pub use lockfile::{
    ArtifactOwner, INSTANCE_LOCK_FILE, LOCK_SCHEMA, LauncherLock, LockFile, LockedArguments,
    LockedArtifact, LockedEntrypoint, LockedJavaRuntime, LockedLoader, LockedMinecraft,
    portable_relative_path,
};
pub use mojang::{
    MojangClient, MojangJavaRequirement, ResolvedVanillaServer, VERSION_MANIFEST_V2_URL,
};
pub use operations::{
    CreateInstanceRequest, CreateInstanceResult, ImportInstanceResult, RemoveInstanceResult,
    RenameInstanceResult, create_instance, import_instance, remove_instance, rename_instance,
    resolve_instance_root, rollback_created_instance, set_default_instance,
};
pub use platform::{Architecture, HostPlatform, OperatingSystem};
pub use registry::{InstanceRegistry, RegistryEntry};
pub use runtime::{
    NativeRuntimeEnvironment, RuntimeContext, RuntimeEnvironment, RuntimePathOptions, RuntimePaths,
};
