use reqwest::{Client as ReqwestClient, header};
use crate::error::Result;

pub struct Client {
    pub(crate) http: ReqwestClient,
    pub(crate) base_url: String,
}

impl Client {
    pub fn new(user_agent: &str) -> Result<Self> {
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::USER_AGENT,
            header::HeaderValue::from_str(user_agent)
                .map_err(|_| crate::error::ModrinthError::Api("Invalid User-Agent".into()))?,
        );

        let http = ReqwestClient::builder()
            .default_headers(headers)
            .build()?;

        Ok(Self {
            http,
            base_url: "https://api.modrinth.com/v2".to_string(),
        })
    }
}
