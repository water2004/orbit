use crate::error::LauncherError;
use crate::instance::LoaderKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoaderVersion {
    pub version: String,
    pub stable: bool,
    pub recommended: bool,
    pub latest: bool,
    pub minimum_java_major: Option<u32>,
}

pub async fn list_loader_versions(
    client: &reqwest::Client,
    kind: LoaderKind,
    minecraft_version: &str,
) -> Result<Vec<LoaderVersion>, LauncherError> {
    match kind {
        LoaderKind::Fabric | LoaderKind::Quilt => {
            crate::loader::list_profile_loader_versions(client, kind, minecraft_version).await
        }
        LoaderKind::Forge | LoaderKind::Neoforge => {
            crate::installer::list_installer_loader_versions(client, kind, minecraft_version).await
        }
        LoaderKind::Vanilla => Ok(Vec::new()),
    }
}
