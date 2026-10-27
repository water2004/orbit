use super::*;

fn remote(tag: &str) -> GitHubRelease {
    GitHubRelease {
        tag_name: tag.into(),
        name: None,
        html_url: format!("{REPOSITORY}/releases/tag/{tag}"),
        published_at: "2026-10-27T00:00:00Z".into(),
        body: Some("Release notes".into()),
        draft: false,
        prerelease: false,
    }
}

#[test]
fn semantic_comparison_includes_prerelease_but_ignores_build_metadata() {
    for (current, latest, expected) in [
        ("0.9.0", "v0.10.0", Status::UpdateAvailable),
        ("0.9.0", "v0.9.0", Status::UpToDate),
        ("0.9.0+local", "v0.9.0", Status::UpToDate),
        ("0.9.0-beta.1", "v0.9.0", Status::UpdateAvailable),
        ("0.10.0", "v0.9.0", Status::Ahead),
    ] {
        assert_eq!(
            interpret(Version::parse(current).unwrap(), remote(latest))
                .unwrap()
                .status,
            expected
        );
    }
}

#[test]
fn rejects_unpublished_invalid_or_untrusted_releases() {
    let installed = Version::new(0, 9, 0);
    for tag in ["latest", "v0.9.1-beta.1"] {
        assert!(interpret(installed.clone(), remote(tag)).is_err());
    }
    let mut draft = remote("v0.9.1");
    draft.draft = true;
    assert!(interpret(installed.clone(), draft).is_err());
    let mut pre = remote("v0.9.1");
    pre.prerelease = true;
    assert!(interpret(installed.clone(), pre).is_err());
    for link in [
        "https://evil.example/releases/tag/v1",
        "http://github.com/water2004/orbit/releases/tag/v1",
        "https://github.com/other/repo/releases/tag/v1",
    ] {
        let mut release = remote("v0.9.1");
        release.html_url = link.into();
        assert!(interpret(installed.clone(), release).is_err());
    }
}

async fn serve(responses: Vec<&'static str>) -> (String, tokio::task::JoinHandle<()>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        for response in responses {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 4096];
            let n = stream.read(&mut buffer).await.unwrap();
            let request = std::str::from_utf8(&buffer[..n]).unwrap();
            assert!(!request.to_ascii_lowercase().contains("authorization:"));
            stream.write_all(response.as_bytes()).await.unwrap();
        }
    });
    (format!("http://{address}/latest"), handle)
}

#[tokio::test]
async fn no_release_is_distinct_from_rate_limit_invalid_json_and_network_error() {
    for (response, no_release) in [
        (
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            true,
        ),
        (
            "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            false,
        ),
        (
            "HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\n{xx",
            false,
        ),
    ] {
        let (url, server) = serve(vec![response]).await;
        let result = check_at(&Client::new(), "0.9.0", 0, &url).await;
        if no_release {
            assert_eq!(result.unwrap().status, Status::NoRelease);
        } else {
            assert!(result.is_err());
        }
        server.await.unwrap();
    }
}

#[tokio::test]
async fn retries_transient_http_errors_with_one_budget() {
    let response = "HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    let (url, server) = serve(vec![response, response]).await;
    let result = check_at(&Client::new(), "0.9.0", 1, &url).await;
    assert!(matches!(result, Err(Error::Http { status: 503, .. })));
    server.await.unwrap();
}

#[tokio::test]
async fn invalid_comparison_version_fails_before_network_access() {
    assert!(matches!(
        check_at(
            &Client::new(),
            "not-a-version",
            0,
            "http://127.0.0.1:1/latest"
        )
        .await,
        Err(Error::InvalidVersion(_))
    ));
}

#[tokio::test]
async fn reads_successful_release_response_and_compares_against_explicit_baseline() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let body = r#"{"tag_name":"v0.10.0","name":"Orbit 0.10.0","html_url":"https://github.com/water2004/orbit/releases/tag/v0.10.0","published_at":"2026-10-27T00:00:00Z","body":"Notes","draft":false,"prerelease":false}"#;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/latest", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buffer = [0; 4096];
        let n = stream.read(&mut buffer).await.unwrap();
        let request = std::str::from_utf8(&buffer[..n])
            .unwrap()
            .to_ascii_lowercase();
        assert!(request.contains("accept: application/vnd.github+json"));
        assert!(request.contains("user-agent: orbit-updates/"));
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    });
    let report = check_at(&Client::new(), "0.9.0", 0, &endpoint)
        .await
        .unwrap();
    assert_eq!(report.current_version, "0.9.0");
    assert_eq!(report.status, Status::UpdateAvailable);
    assert_eq!(report.release.unwrap().notes, "Notes");
    server.await.unwrap();
}
