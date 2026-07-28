//! Core domain and persistence APIs for `orbit-launcher`.
//!
//! This crate owns launcher business logic and filesystem I/O. It does not
//! render terminal output and has no dependency on Orbit's mod-management
//! crates.

pub mod account;
pub mod artifact;
mod atomic_io;
pub mod authlib_injector;
pub mod client;
pub mod config;
pub mod context;
pub mod error;
pub mod eula;
pub mod install;
pub mod installer;
pub mod instance;
pub mod java;
pub mod launch;
pub mod loader;
pub mod lockfile;
mod maven;
pub mod mojang;
pub mod operations;
pub mod platform;
pub mod registry;
pub mod runtime;
pub mod secret_store;
pub mod versions;

pub use account::{
    AccountLaunchIdentity, AccountMetadata, AccountProvider, AccountRepository,
    ExternalYggdrasilLoginRequest, MicrosoftDeviceSession, MicrosoftLoginProgressEvent,
    begin_microsoft_device_login, complete_microsoft_device_login, create_offline_account,
    login_external_yggdrasil, resolve_launch_identity,
};
pub use artifact::{
    ArtifactCache, ArtifactRequest, ArtifactTransferEvent, CachedArtifact, ExpectedHash,
    hash_file_sha256,
};
pub use authlib_injector::{
    AUTHLIB_INJECTOR_LATEST_URL, ResolvedAuthlibInjector, resolve_authlib_injector,
    verify_authlib_injector,
};
pub use client::{
    AssetMapping, ClientDownload, NativeExtract, ResolvedVanillaClient, resolve_vanilla_client,
};
pub use config::{
    ConfigEntry, ConfigKey, ConfigMutation, GlobalConfig, InstallerConfig, JavaProvider,
    UiPreference, YggdrasilProviderConfig, add_yggdrasil_provider, get_config, list_config,
    remove_yggdrasil_provider, set_config, unset_config,
};
pub use context::{ContextIntent, ContextSource, ResolvedInstance, resolve_instance};
pub use error::LauncherError;
pub use eula::{
    EulaAcceptance, EulaAcceptanceMethod, EulaDocument, MINECRAFT_EULA_URL, accept_shown_eula,
    require_current_acceptance, show_current_eula,
};
pub use install::{
    ClientInstallPlan, InstallPlan, InstallProgressEvent, InstallResult, ServerInstallPlan,
    apply_install_plan, prepare_install,
};
pub use installer::{
    INSTALLER_STAGING_NAME, InspectedLoaderInstaller, InstalledClientProfile,
    InstallerOutputStream, InstallerSide, LoaderInstallerEvent, ResolvedLoaderInstaller,
    inspect_loader_installer, installed_server_argument_file, read_installed_client_profile,
    resolve_loader_installer, run_loader_installer,
};
pub use instance::{
    INSTANCE_MANIFEST_FILE, InstanceKind, InstanceManifest, JavaPolicy, LoaderKind, ManifestFile,
    RestartPolicy,
};
pub use java::{
    InstalledJavaRuntime, JavaProgressEvent, JavaTarget, MOJANG_RUNTIME_MANIFEST_URL,
    ManagedJavaRuntime, MojangJavaPlan, install_mojang_java, list_managed_java_runtimes,
    plan_mojang_java, remove_managed_java_runtime, verify_locked_java_runtime,
    verify_managed_java_runtime,
};
pub use launch::{
    LaunchOutputStream, LaunchPlan, LaunchPlanSummary, LaunchPreparationEvent, LaunchProcessEvent,
    LaunchResult, SupervisorControl, SupervisorEvent, SupervisorResult, prepare_launch, run_launch,
    supervise_server,
};
pub use loader::{LoaderSide, ResolvedLoaderProfile, resolve_loader_profile};
pub use lockfile::{
    ArtifactOwner, INSTANCE_LOCK_FILE, LOCK_SCHEMA, LauncherLock, LockFile, LockedArguments,
    LockedArtifact, LockedArtifactSource, LockedAuthlibInjector, LockedEntrypoint,
    LockedJavaRuntime, LockedLoader, LockedLoaderSource, LockedMinecraft, portable_relative_path,
};
pub use mojang::{
    MinecraftVersion, MinecraftVersionCatalog, MojangClient, MojangJavaRequirement,
    ResolvedVanillaServer, VERSION_MANIFEST_V2_URL,
};
pub use operations::{
    ConfigureInstanceRequest, ConfigureInstanceResult, CreateInstanceRequest, CreateInstanceResult,
    ImportInstanceResult, RemoveInstanceResult, RenameInstanceResult, configure_instance,
    create_instance, import_instance, remove_instance, rename_instance, resolve_instance_root,
    rollback_created_instance, set_default_instance,
};
pub use platform::{Architecture, HostPlatform, OperatingSystem};
pub use registry::{InstanceRegistry, RegistryEntry};
pub use runtime::{
    NativeRuntimeEnvironment, RuntimeContext, RuntimeEnvironment, RuntimePathOptions, RuntimePaths,
};
pub use secret_store::{SecretStore, native_secret_store};
pub use versions::{LoaderVersion, list_loader_versions};
