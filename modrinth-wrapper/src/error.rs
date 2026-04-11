use thiserror::Error;

#[derive(Error, Debug)]
pub enum ModrinthError {
    #[error("HTTP request failed: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("Serialization/Deserialization failed: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("API returned error: {0}")]
    Api(String),
}

pub type Result<T> = std::result::Result<T, ModrinthError>;
