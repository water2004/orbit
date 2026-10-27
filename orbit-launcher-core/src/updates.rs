use crate::{GlobalConfig, LauncherError};
pub use orbit_updates::{Status, UpdateCheck};

pub async fn check(
    config: &GlobalConfig,
    current_version: &str,
) -> Result<UpdateCheck, LauncherError> {
    orbit_updates::check(&config.http_client()?, current_version, 3)
        .await
        .map_err(|error| match error {
            orbit_updates::Error::Transport(orbit_http::Error::Network(error)) => {
                LauncherError::Network(error)
            }
            error => LauncherError::InvalidRemoteData(error.to_string()),
        })
}
