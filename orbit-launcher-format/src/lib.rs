//! Installed-runtime storage contract, compiled into each independent program.
//! No I/O, configuration management, migrations or launcher business logic.
use serde::Deserialize;

pub const MANIFEST_FILE: &str = "orbit-launcher.toml";
pub const LOCK_FILE: &str = "orbit-launcher.lock";
pub const MANIFEST_SCHEMA: u32 = 2;
pub const LOCK_SCHEMA: u32 = 7;
pub const V0_7_MANIFEST_SCHEMA: u32 = 1;
pub const V0_7_LOCK_SCHEMA: u32 = 6;

// Both published formats have the same runtime projection. Reading these facts
// does not migrate documents or permit writing older formats.
pub const READABLE_MANIFEST_SCHEMAS: &[u32] = &[V0_7_MANIFEST_SCHEMA, MANIFEST_SCHEMA];
pub const READABLE_LOCK_SCHEMAS: &[u32] = &[V0_7_LOCK_SCHEMA, LOCK_SCHEMA];

// Icons, Java, accounts, download URLs and all unrelated fields remain entirely
// Launcher-owned. Consumers do not deserialize a second Launcher settings model.
#[derive(Deserialize)]
pub struct RuntimeManifest {
    pub schema: u32,
    pub id: String,
    pub kind: String,
}
#[derive(Deserialize)]
pub struct RuntimeLock {
    pub schema: u32,
    pub instance_id: String,
    pub kind: String,
    pub minecraft: RuntimeMinecraft,
    pub loader: RuntimeLoader,
    pub entrypoint: RuntimeEntrypoint,
    pub artifacts: Vec<RuntimeArtifact>,
}
#[derive(Deserialize)]
pub struct RuntimeMinecraft {
    pub version: String,
}
#[derive(Deserialize)]
pub struct RuntimeLoader {
    pub kind: String,
    pub version: Option<String>,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RuntimeEntrypoint {
    Classpath {
        classpath: Vec<String>,
    },
    #[serde(other)]
    Other,
}
#[derive(Deserialize)]
pub struct RuntimeArtifact {
    pub owner: RuntimeArtifactOwner,
    pub sha256: String,
    pub path: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeArtifactOwner {
    Minecraft,
    Loader,
    Java,
    AuthlibInjector,
}
impl RuntimeArtifactOwner {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Minecraft => "minecraft",
            Self::Loader => "loader",
            Self::Java => "java",
            Self::AuthlibInjector => "authlib-injector",
        }
    }
}
