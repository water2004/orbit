use super::*;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const OK: &str = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}";
const TRUNCATED: &str = "HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{";

struct Server {
    url: String,
    requests: Arc<Mutex<Vec<Vec<u8>>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn new(responses: &[&str]) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/lookup", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let capture = requests.clone();
        let responses: Vec<_> = responses.iter().map(|s| s.as_bytes().to_vec()).collect();
        let task = tokio::spawn(async move {
            for response in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut chunk = [0; 1024];
                    let n = socket.read(&mut chunk).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    request.extend_from_slice(&chunk[..n]);
                    if let Some(header_end) = request.windows(4).position(|s| s == b"\r\n\r\n") {
                        let text = String::from_utf8_lossy(&request[..header_end]);
                        let length = text
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if request.len() >= header_end + 4 + length {
                            break;
                        }
                    }
                }
                capture.lock().unwrap().push(request);
                socket.write_all(&response).await.unwrap();
                socket.shutdown().await.unwrap();
            }
        });
        Self {
            url,
            requests,
            task,
        }
    }
    fn client(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .retry(reqwest::retry::never())
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap()
    }
    fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

#[tokio::test]
async fn truncated_body_and_server_status_share_a_single_retry_budget() {
    let server = Server::new(&[
        TRUNCATED,
        "HTTP/1.1 503 Busy\r\nContent-Length: 0\r\n\r\n",
        OK,
    ])
    .await;
    let response = read(server.client().get(&server.url), 2, 1024)
        .await
        .unwrap();
    assert_eq!(response.body, b"{}");
    assert_eq!(server.count(), 3);
    let server = Server::new(&[TRUNCATED, TRUNCATED, TRUNCATED, OK]).await;
    assert!(matches!(
        read(server.client().get(&server.url), 2, 1024).await,
        Err(Error::Network(_))
    ));
    assert_eq!(
        server.count(),
        3,
        "body retries must not multiply the header retry budget"
    );
}

#[tokio::test]
async fn read_only_post_lookup_replays_identical_body_and_headers() {
    let server = Server::new(&[TRUNCATED, OK]).await;
    let response = read(
        server
            .client()
            .post(&server.url)
            .header("x-api-key", "test-only")
            .body("batch-identities"),
        1,
        1024,
    )
    .await
    .unwrap();
    assert_eq!(response.body, b"{}");
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0], requests[1]);
    assert!(requests[0].ends_with(b"batch-identities"));
}

#[tokio::test]
async fn permanent_http_errors_are_not_retried_and_error_bodies_are_bounded() {
    for status in [400, 401, 403, 404, 501, 505] {
        let reply = format!(
            "HTTP/1.1 {status} Failed\r\nContent-Length: 5000\r\nConnection: close\r\n\r\n{}",
            "x".repeat(5000)
        );
        let server = Server::new(&[&reply, OK]).await;
        let result = read(server.client().get(&server.url), 3, 10000)
            .await
            .unwrap();
        assert_eq!(result.status.as_u16(), status);
        assert_eq!(result.body.len(), 4096);
        assert_eq!(server.count(), 1);
    }
}

#[tokio::test]
async fn body_limits_and_invalid_complete_json_are_not_network_retries() {
    let server = Server::new(&[OK, OK]).await;
    assert!(matches!(
        read(server.client().get(&server.url), 3, 1).await,
        Err(Error::TooLarge(1))
    ));
    assert_eq!(server.count(), 1);
    let server = Server::new(&["HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nbad", OK]).await;
    assert_eq!(
        read(server.client().get(&server.url), 3, 1024)
            .await
            .unwrap()
            .body,
        b"bad"
    );
    assert_eq!(
        server.count(),
        1,
        "JSON parsing is the caller's permanent validation step"
    );
}

#[tokio::test]
async fn retry_after_supports_seconds_dates_and_an_explicit_wait_bound() {
    let server = Server::new(&[
        "HTTP/1.1 429 Slow\r\nRetry-After: 0\r\nContent-Length: 0\r\n\r\n",
        OK,
    ])
    .await;
    assert!(
        read(server.client().get(&server.url), 1, 1024)
            .await
            .unwrap()
            .status
            .is_success()
    );
    assert_eq!(server.count(), 2);
    for header in [
        "3600".to_string(),
        httpdate::fmt_http_date(SystemTime::now() + Duration::from_secs(3600)),
    ] {
        let reply =
            format!("HTTP/1.1 503 Wait\r\nRetry-After: {header}\r\nContent-Length: 0\r\n\r\n");
        let server = Server::new(&[&reply, OK]).await;
        let result = read(server.client().get(&server.url), 3, 1024)
            .await
            .unwrap();
        assert_eq!(result.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            server.count(),
            1,
            "never retry before a server's long Retry-After"
        );
    }
}

#[tokio::test]
async fn cancellation_during_backoff_never_sends_a_second_request() {
    let server = Server::new(&[TRUNCATED, OK]).await;
    let future = read(server.client().get(&server.url), 3, 1024);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), future)
            .await
            .is_err()
    );
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(server.count(), 1);
}

#[tokio::test]
async fn streaming_status_errors_cannot_bypass_a_long_retry_after() {
    let server = Server::new(&[
        "HTTP/1.1 429 Slow\r\nRetry-After: 3600\r\nContent-Length: 0\r\n\r\n",
        OK,
    ])
    .await;
    let mut retry = RetryState::new(3);
    let response = retry.send(server.client().get(&server.url)).await.unwrap();
    let error = response.error_for_status().unwrap_err();
    assert!(!retry.retry_error(&error).await);
    assert_eq!(server.count(), 1);
}
