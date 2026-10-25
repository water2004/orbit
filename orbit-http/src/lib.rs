//! Transport only. Callers explicitly establish replay safety: reads, read-only
//! POST lookups or protocol-approved proof exchanges. OAuth redemption, token
//! rotation and arbitrary mutations must not opt in based on their HTTP method.
use reqwest::{RequestBuilder, Response, StatusCode, header::RETRY_AFTER};
use std::time::{Duration, SystemTime};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Network(#[from] reqwest::Error),
    #[error("response exceeds its {0} byte limit")]
    TooLarge(u64),
    #[error("request body cannot be safely replayed")]
    NotReplayable,
}

pub struct BufferedResponse {
    pub status: StatusCode,
    pub url: reqwest::Url,
    pub headers: reqwest::header::HeaderMap,
    pub body: Vec<u8>,
}

/// One shared retry budget for response headers and body, not a nested loop at
/// each layer. Dropping the future also cancels its asynchronous backoff.
pub struct RetryState {
    remaining: u32,
    attempt: u32,
}
impl Default for RetryState {
    fn default() -> Self {
        Self::new(3)
    }
}
impl RetryState {
    pub const fn new(max_retries: u32) -> Self {
        Self {
            remaining: max_retries,
            attempt: 0,
        }
    }

    async fn wait(&mut self, retry_after: Option<Duration>) -> bool {
        if self.remaining == 0 {
            return false;
        }
        // Do not retry earlier than a server's long Retry-After. Return the
        // response to the caller instead of waiting unboundedly or ignoring it.
        if retry_after.is_some_and(|delay| delay > Duration::from_secs(30)) {
            return false;
        }
        let base = 200_u64.saturating_mul(1 << self.attempt.min(4));
        let delay = Duration::from_millis(base + fastrand::u64(0..=100))
            .max(retry_after.unwrap_or_default());
        self.remaining -= 1;
        self.attempt = self.attempt.saturating_add(1);
        tokio::time::sleep(delay).await;
        true
    }

    pub async fn retry_error(&mut self, error: &reqwest::Error) -> bool {
        let retryable = error.status().map(retryable_status).unwrap_or_else(|| {
            error.is_timeout()
                || error.is_connect()
                || error.is_body()
                || error.is_decode()
                || error.is_request()
        });
        retryable && self.wait(None).await
    }

    /// The caller must establish replay safety. Request bodies must be buffered;
    /// redirects and credentials remain governed by the caller's HTTP client.
    pub async fn send(&mut self, request: RequestBuilder) -> Result<Response, Error> {
        let (client, request) = request.build_split();
        let request = request?;
        loop {
            match client
                .execute(request.try_clone().ok_or(Error::NotReplayable)?)
                .await
            {
                Ok(response) => {
                    if retryable_status(response.status()) {
                        if self.wait(retry_after(&response)).await {
                            continue;
                        }
                        // Streaming callers may convert this final status to a
                        // reqwest error. They must not bypass a long Retry-After
                        // by retrying it again without its response headers.
                        self.remaining = 0;
                    }
                    return Ok(response);
                }
                Err(error) => {
                    if self.retry_error(&error).await {
                        continue;
                    }
                    return Err(Error::Network(error));
                }
            }
        }
    }
}

fn retryable_status(status: StatusCode) -> bool {
    matches!(status.as_u16(), 408 | 429 | 500 | 502 | 503 | 504)
}

fn retry_after(response: &Response) -> Option<Duration> {
    let raw = response.headers().get(RETRY_AFTER)?.to_str().ok()?;
    raw.parse::<u64>()
        .ok()
        .map(Duration::from_secs)
        .or_else(|| {
            httpdate::parse_http_date(raw)
                .ok()
                .map(|date| date.duration_since(SystemTime::now()).unwrap_or_default())
        })
}

/// Reads the complete bounded body inside the same retry budget. JSON parsing
/// belongs after this function and is therefore not mistaken for a network fault.
pub async fn read(
    request: RequestBuilder,
    max_retries: u32,
    limit: u64,
) -> Result<BufferedResponse, Error> {
    let mut retry = RetryState::new(max_retries);
    loop {
        let mut response = retry
            .send(request.try_clone().ok_or(Error::NotReplayable)?)
            .await?;
        let status = response.status();
        let url = response.url().clone();
        let headers = response.headers().clone();
        let success = status.is_success();
        let maximum = if success { limit } else { limit.min(4096) };
        if success && response.content_length().is_some_and(|size| size > maximum) {
            return Err(Error::TooLarge(maximum));
        }
        let mut body = Vec::new();
        let body_result = async {
            while let Some(chunk) = response.chunk().await? {
                if body.len() as u64 + chunk.len() as u64 > maximum {
                    if success {
                        return Err(Error::TooLarge(maximum));
                    }
                    body.extend_from_slice(&chunk[..(maximum as usize).saturating_sub(body.len())]);
                    break;
                }
                body.extend_from_slice(&chunk);
            }
            Ok::<_, Error>(())
        }
        .await;
        match body_result {
            Err(Error::Network(error)) if success => {
                if retry.retry_error(&error).await {
                    continue;
                }
                return Err(Error::Network(error));
            }
            Err(error) if success => return Err(error),
            _ => {
                return Ok(BufferedResponse {
                    status,
                    url,
                    headers,
                    body,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests;
