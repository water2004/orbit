//! Core domain and persistence APIs for `orbit-launcher`.
//!
//! This crate owns launcher business logic and filesystem I/O. It does not
//! render terminal output and has no dependency on Orbit's mod-management
//! crates.

mod atomic_io;
pub mod config;
pub mod context;
pub mod error;
pub mod instance;
pub mod operations;
pub mod registry;
pub mod runtime;

pub use config::{
    ConfigEntry, ConfigKey, ConfigMutation, GlobalConfig, JavaProvider, UiPreference, get_config,
    list_config, set_config, unset_config,
};
pub use context::{ContextIntent, ContextSource, ResolvedInstance, resolve_instance};
pub use error::LauncherError;
pub use instance::{
    INSTANCE_MANIFEST_FILE, InstanceKind, InstanceManifest, JavaPolicy, LoaderKind, ManifestFile,
    RestartPolicy,
};
pub use operations::{
    CreateInstanceRequest, CreateInstanceResult, ImportInstanceResult, RemoveInstanceResult,
    RenameInstanceResult, create_instance, import_instance, remove_instance, rename_instance,
    resolve_instance_root, set_default_instance,
};
pub use registry::{InstanceRegistry, RegistryEntry};
pub use runtime::{
    NativeRuntimeEnvironment, RuntimeContext, RuntimeEnvironment, RuntimePathOptions, RuntimePaths,
};
