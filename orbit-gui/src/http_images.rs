use futures_util::AsyncReadExt;
use gpui::http_client::{AsyncBody, HttpClient, Request, Response, Url, http::HeaderValue};

const USER_AGENT: &str = "orbit-gui/0.1.0 (https://github.com/water2004/orbit)";
const MAX_IMAGE_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

pub struct ImageHttpClient {
    client: reqwest::Client,
    user_agent: HeaderValue,
}

impl ImageHttpClient {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .user_agent(USER_AGENT)
                .redirect(reqwest::redirect::Policy::limited(8))
                .timeout(std::time::Duration::from_secs(20))
                .build()?,
            user_agent: HeaderValue::from_static(USER_AGENT),
        })
    }
}

impl HttpClient for ImageHttpClient {
    fn type_name(&self) -> &'static str {
        "orbit_gui::ImageHttpClient"
    }

    fn user_agent(&self) -> Option<&HeaderValue> {
        Some(&self.user_agent)
    }

    fn send(
        &self,
        request: Request<AsyncBody>,
    ) -> futures_util::future::BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
        let client = self.client.clone();
        Box::pin(async move {
            let (parts, mut body) = request.into_parts();
            let url = parts.uri.to_string();
            let parsed = reqwest::Url::parse(&url)?;
            if !matches!(parsed.scheme(), "http" | "https") {
                anyhow::bail!("unsupported image URL scheme '{}'", parsed.scheme());
            }

            let mut body_bytes = Vec::new();
            body.read_to_end(&mut body_bytes).await?;
            let mut request = client.request(
                reqwest::Method::from_bytes(parts.method.as_str().as_bytes())?,
                parsed,
            );
            for (name, value) in &parts.headers {
                request = request.header(name.as_str(), value.as_bytes());
            }
            if !body_bytes.is_empty() {
                request = request.body(body_bytes);
            }

            let response = request.send().await?;
            if response
                .content_length()
                .is_some_and(|length| length > MAX_IMAGE_RESPONSE_BYTES as u64)
            {
                anyhow::bail!("remote image exceeds the 8 MiB presentation limit");
            }
            let status = response.status();
            let headers = response.headers().clone();
            let bytes = response.bytes().await?;
            if bytes.len() > MAX_IMAGE_RESPONSE_BYTES {
                anyhow::bail!("remote image exceeds the 8 MiB presentation limit");
            }

            let mut output = Response::builder().status(status.as_u16());
            for (name, value) in &headers {
                output = output.header(name.as_str(), value.as_bytes());
            }
            Ok(output.body(AsyncBody::from(bytes.to_vec()))?)
        })
    }

    fn proxy(&self) -> Option<&Url> {
        None
    }
}
