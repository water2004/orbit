use super::*;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn microsoft_client_id_is_the_registered_public_application() {
    assert_eq!(MICROSOFT_CLIENT_ID, "a1b083c0-2c7f-47c8-a351-451279131fc7");
}

#[test]
fn device_session_path_is_derived_only_from_a_uuid() {
    let paths = RuntimePaths::resolve(&crate::runtime::RuntimePathOptions {
        config_dir: Some(PathBuf::from("config")),
        data_dir: Some(PathBuf::from("data")),
        cache_dir: Some(PathBuf::from("cache")),
    })
    .unwrap();
    assert_eq!(
        device_session_path(&paths, Uuid::nil()),
        PathBuf::from("data/auth-sessions/00000000-0000-0000-0000-000000000000.json")
    );
}

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
    async fn new(responses: Vec<String>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/authenticate", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let capture = requests.clone();
        let task = tokio::spawn(async move {
            for response in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut buffer = [0; 4096];
                    let read = socket.read(&mut buffer).await.unwrap();
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                    if let Some(end) = request.windows(4).position(|s| s == b"\r\n\r\n") {
                        let length = String::from_utf8_lossy(&request[..end])
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                capture.lock().unwrap().push(request);
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });
        Self {
            url,
            requests,
            task,
        }
    }
    fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
    fn client(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .retry(reqwest::retry::never())
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap()
    }
}

fn response(status: u16, body: &str) -> String {
    format!(
        "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

async fn proof(server: &Server) -> Result<serde_json::Value, LauncherError> {
    exchange_proof(
        &server.client(),
        "Xbox Live authentication",
        &server.url,
        &serde_json::json!({ "Properties": { "RpsTicket": "d=fake-proof-for-test" } }),
    )
    .await
}

#[tokio::test]
async fn xbox_408_retries_only_the_same_proof_request() {
    let server = Server::new(vec![
        response(408, ""),
        response(200, r#"{"Token":"fake-xbox-token"}"#),
    ])
    .await;
    let result = proof(&server).await.unwrap();
    assert_eq!(result["Token"], "fake-xbox-token");
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0], requests[1]);
    assert!(String::from_utf8_lossy(&requests[0]).contains("accept: application/json"));
}

#[tokio::test]
async fn proof_status_and_interrupted_body_share_a_bounded_budget() {
    let server = Server::new(vec![
        response(503, ""),
        "HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{".into(),
        response(200, "{}"),
    ])
    .await;
    proof(&server).await.unwrap();
    assert_eq!(server.count(), 3);
    let server = Server::new(vec![response(408, ""); 5]).await;
    let error = proof(&server).await.unwrap_err();
    assert!(error.to_string().contains("HTTP 408"));
    assert_eq!(server.count(), 4);
}

#[tokio::test]
async fn rejected_credentials_xerr_and_invalid_json_do_not_retry() {
    for (status, body, detail) in [
        (400, r#"{"error":"invalid_request"}"#, "invalid_request"),
        (
            401,
            r#"{"XErr":2148916233,"Message":"account restriction"}"#,
            "2148916233",
        ),
        (403, "{}", "HTTP 403"),
        (200, "not json", "invalid JSON"),
    ] {
        let server = Server::new(vec![response(status, body), response(200, "{}")]).await;
        let error = proof(&server).await.unwrap_err();
        assert!(error.to_string().contains(detail), "{error}");
        assert_eq!(server.count(), 1);
        assert!(!error.to_string().contains("fake-proof-for-test"));
    }
}

#[tokio::test]
async fn oauth_code_redemption_and_token_rotation_are_not_automatically_replayed() {
    for raw in [
        response(408, ""),
        response(503, ""),
        "HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{".into(),
    ] {
        let server = Server::new(vec![raw, response(200, "{}")]).await;
        let _ = read_auth_response(
            "Microsoft token",
            server
                .client()
                .post(&server.url)
                .form(&[("refresh_token", "fake-refresh")]),
            AuthReplay::Never,
        )
        .await;
        assert_eq!(server.count(), 1);
    }
    let server = Server::new(vec![response(400, r#"{"error":"slow_down"}"#)]).await;
    let response = read_auth_response(
        "Microsoft device token",
        server.client().post(&server.url),
        AuthReplay::Never,
    )
    .await
    .unwrap();
    let token: OAuthTokenResponse = decode_json("Microsoft device token", &response.body).unwrap();
    assert_eq!(token.error.as_deref(), Some("slow_down"));
}

#[tokio::test]
async fn microsoft_read_retries_and_all_auth_responses_remain_bounded() {
    let server = Server::new(vec![response(503, ""), response(200, "{}")]).await;
    read_auth_response(
        "Minecraft profile",
        server.client().get(&server.url),
        AuthReplay::Read,
    )
    .await
    .unwrap();
    assert_eq!(server.count(), 2);
    let large = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        MAX_AUTH_RESPONSE_BYTES + 1
    );
    let server = Server::new(vec![large, response(200, "{}")]).await;
    assert!(
        proof(&server)
            .await
            .unwrap_err()
            .to_string()
            .contains("byte limit")
    );
    assert_eq!(server.count(), 1);
}
