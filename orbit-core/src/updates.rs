use crate::{OrbitError, config::GlobalConfig};
pub use orbit_updates::{Status, UpdateCheck};

pub async fn check(
    config: &GlobalConfig,
    current_version: &str,
) -> Result<UpdateCheck, OrbitError> {
    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(config.network.timeout))
        .retry(reqwest::retry::never());
    if let Some(proxy) = &config.network.proxy {
        builder = builder.proxy(reqwest::Proxy::all(proxy)?);
    }
    let client = builder.build()?;
    orbit_updates::check(&client, current_version, config.network.max_retries)
        .await
        .map_err(|error| match error {
            orbit_updates::Error::Transport(orbit_http::Error::Network(error)) => {
                OrbitError::Network(error)
            }
            error => OrbitError::Other(error.into()),
        })
}
