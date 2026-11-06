use super::*;
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use tower::ServiceExt;

async fn fixture() -> (tempfile::TempDir, AppState, Router) {
    let temp = tempfile::tempdir().unwrap();
    let assets = temp.path().join("assets");
    std::fs::create_dir_all(assets.join("_next/static/chunks")).unwrap();
    std::fs::write(assets.join("index.html"), "<html>portfolio</html>").unwrap();
    std::fs::write(assets.join("resume.pdf"), b"%PDF-1.7\nresume fixture").unwrap();
    std::fs::write(assets.join("picture.png"), b"image").unwrap();
    for (name, bytes) in [
        (
            "app-123.js",
            include_bytes!("../tests/fixtures/app.js").as_slice(),
        ),
        (
            "app-123.js.gz",
            include_bytes!("../tests/fixtures/app.js.gz").as_slice(),
        ),
        (
            "app-123.js.br",
            include_bytes!("../tests/fixtures/app.js.br").as_slice(),
        ),
    ] {
        std::fs::write(assets.join("_next/static/chunks").join(name), bytes).unwrap();
    }
    let state = open_state(&temp.path().join("data"), false).await.unwrap();
    let router = app(&assets, state.clone());
    (temp, state, router)
}
fn request(method: &str, path: &str, body: String, ip: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .extension(ConnectInfo(
            format!("{ip}:1234").parse::<SocketAddr>().unwrap(),
        ))
        .body(Body::from(body))
        .unwrap()
}
fn valid() -> String {
    serde_json::json!({"name":"Trey Test","email":"test@example.test","message":"A useful test message. ".repeat(8)}).to_string()
}
#[tokio::test]
async fn redirects_resume_and_cache_policies() {
    let (_temp, _state, router) = fixture().await;
    for (path, target) in [
        ("/github", "https://github.com/treymouledoux"),
        ("/instagram", "https://www.instagram.com/treymouledoux/"),
        ("/linkedin", "https://www.linkedin.com/in/trey-mouledoux/"),
    ] {
        let response = router
            .clone()
            .oneshot(request("GET", path, String::new(), "127.0.0.1"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(response.headers()[header::LOCATION], target);
    }
    for (path, policy) in [
        ("/", "no-cache"),
        ("/resume", "no-cache"),
        ("/resume.pdf", "no-cache"),
        ("/picture.png", "public, max-age=3600"),
        (
            "/_next/static/chunks/app-123.js",
            "public, max-age=31536000, immutable",
        ),
        ("/_next/static/missing.js", "no-store"),
        ("/healthz", "no-store"),
    ] {
        let response = router
            .clone()
            .oneshot(request("GET", path, String::new(), "127.0.0.1"))
            .await
            .unwrap();
        assert_eq!(response.headers()[header::CACHE_CONTROL], policy, "{path}");
        if path == "/resume" {
            assert_eq!(response.headers()[header::CONTENT_TYPE], "application/pdf");
            assert!(
                response.headers()[header::CONTENT_DISPOSITION]
                    .to_str()
                    .unwrap()
                    .starts_with("inline;")
            );
            assert!(
                to_bytes(response.into_body(), BODY_LIMIT)
                    .await
                    .unwrap()
                    .starts_with(b"%PDF")
            );
        }
    }
    let response = router
        .oneshot(request("HEAD", "/resume", String::new(), "127.0.0.1"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        to_bytes(response.into_body(), BODY_LIMIT)
            .await
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn serves_precompressed_assets_and_identity() {
    let (_temp, _, router) = fixture().await;
    for (encoding, expected) in [
        (
            "br",
            include_bytes!("../tests/fixtures/app.js.br").as_slice(),
        ),
        (
            "gzip",
            include_bytes!("../tests/fixtures/app.js.gz").as_slice(),
        ),
        (
            "identity",
            include_bytes!("../tests/fixtures/app.js").as_slice(),
        ),
    ] {
        let mut req = request(
            "GET",
            "/_next/static/chunks/app-123.js",
            String::new(),
            "127.0.0.1",
        );
        req.headers_mut()
            .insert(header::ACCEPT_ENCODING, HeaderValue::from_static(encoding));
        let response = router.clone().oneshot(req).await.unwrap();
        if encoding != "identity" {
            assert_eq!(response.headers()[header::CONTENT_ENCODING], encoding);
        }
        assert!(
            response
                .headers()
                .get_all(header::VARY)
                .iter()
                .any(|v| v.to_str().unwrap().contains("Accept-Encoding"))
        );
        assert_eq!(
            to_bytes(response.into_body(), BODY_LIMIT)
                .await
                .unwrap()
                .as_ref(),
            expected
        );
    }
}
#[tokio::test]
async fn rejects_invalid_and_large_json_without_writing() {
    let (temp, _, router) = fixture().await;
    for (body, status) in [
        ("{broken".to_owned(), StatusCode::BAD_REQUEST),
        ("{}".to_owned(), StatusCode::UNPROCESSABLE_ENTITY),
        (
            serde_json::json!({"name":"A","email":"a@b.c","message":"short"}).to_string(),
            StatusCode::BAD_REQUEST,
        ),
        (
            serde_json::json!({"name":"A","email":"a@b.c","message":"a".repeat(BODY_LIMIT)})
                .to_string(),
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
    ] {
        let response = router
            .clone()
            .oneshot(request("POST", "/api/contact", body, "127.0.0.1"))
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    }
    assert_eq!(
        std::fs::metadata(temp.path().join("data/submissions.jsonl"))
            .unwrap()
            .len(),
        0
    );
}
#[tokio::test]
async fn simultaneous_submissions_enforce_cooldown_and_persist_one_record() {
    let (temp, _, router) = fixture().await;
    let (a, b) = tokio::join!(
        router
            .clone()
            .oneshot(request("POST", "/api/contact", valid(), "127.0.0.1")),
        router.oneshot(request("POST", "/api/contact", valid(), "127.0.0.1"))
    );
    let statuses = [a.unwrap().status(), b.unwrap().status()];
    assert!(statuses.contains(&StatusCode::OK));
    assert!(statuses.contains(&StatusCode::TOO_MANY_REQUESTS));
    let saved = std::fs::read_to_string(temp.path().join("data/submissions.jsonl")).unwrap();
    assert_eq!(saved.lines().count(), 1);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&saved).unwrap()["email"],
        "test@example.test"
    );
}
#[tokio::test]
async fn failed_validation_is_attempt_limited_and_limit_expires() {
    let (_temp, state, router) = fixture().await;
    for _ in 0..5 {
        assert_eq!(
            router
                .clone()
                .oneshot(request("POST", "/api/contact", "{}".into(), "127.0.0.1"))
                .await
                .unwrap()
                .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let response = router
        .clone()
        .oneshot(request("POST", "/api/contact", valid(), "127.0.0.1"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(response.headers().contains_key(header::RETRY_AFTER));
    state
        .store
        .lock()
        .await
        .clients
        .get_mut(&"127.0.0.1".parse().unwrap())
        .unwrap()
        .window = Instant::now() - ATTEMPT_WINDOW;
    assert_eq!(
        router
            .oneshot(request("POST", "/api/contact", valid(), "127.0.0.1"))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
}
#[tokio::test]
async fn concurrent_clients_write_complete_records() {
    let (temp, _, router) = fixture().await;
    let mut tasks = tokio::task::JoinSet::new();
    for i in 1..=12 {
        let router = router.clone();
        tasks.spawn(async move {
            router
                .oneshot(request(
                    "POST",
                    "/api/contact",
                    valid(),
                    &format!("192.0.2.{i}"),
                ))
                .await
                .unwrap()
                .status()
        });
    }
    while let Some(status) = tasks.join_next().await {
        assert_eq!(status.unwrap(), StatusCode::OK);
    }
    let records = std::fs::read_to_string(temp.path().join("data/submissions.jsonl")).unwrap();
    assert_eq!(records.lines().count(), 12);
    for line in records.lines() {
        assert!(serde_json::from_str::<serde_json::Value>(line).is_ok());
    }
}
#[test]
fn forwarded_addresses_require_explicit_local_proxy_trust() {
    let mut headers = HeaderMap::new();
    headers.insert("cf-connecting-ip", HeaderValue::from_static("203.0.113.9"));
    for (peer, trust, expected) in [
        ("127.0.0.1:10", false, "127.0.0.1"),
        ("192.0.2.1:10", true, "192.0.2.1"),
        ("127.0.0.1:10", true, "203.0.113.9"),
    ] {
        assert_eq!(
            client_ip(peer.parse().unwrap(), &headers, trust),
            expected.parse::<IpAddr>().unwrap()
        );
    }
}

#[tokio::test]
async fn cooldown_expires_and_storage_failure_is_visible() {
    let (_temp, state, router) = fixture().await;
    assert_eq!(
        router
            .clone()
            .oneshot(request("POST", "/api/contact", valid(), "127.0.0.1"))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    state
        .store
        .lock()
        .await
        .clients
        .get_mut(&"127.0.0.1".parse().unwrap())
        .unwrap()
        .last_success = Some(Instant::now() - COOLDOWN);
    assert_eq!(
        router
            .clone()
            .oneshot(request("POST", "/api/contact", valid(), "127.0.0.1"))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    state.store.lock().await.healthy = false;
    assert_eq!(
        router
            .clone()
            .oneshot(request("GET", "/healthz", String::new(), "127.0.0.1"))
            .await
            .unwrap()
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        router
            .oneshot(request("POST", "/api/contact", valid(), "192.0.2.1"))
            .await
            .unwrap()
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}
#[tokio::test]
async fn html_revalidation_keeps_no_cache_policy() {
    let (_temp, _, router) = fixture().await;
    let response = router
        .clone()
        .oneshot(request("GET", "/", String::new(), "127.0.0.1"))
        .await
        .unwrap();
    let modified = response.headers()[header::LAST_MODIFIED].clone();
    let mut req = request("GET", "/", String::new(), "127.0.0.1");
    req.headers_mut()
        .insert(header::IF_MODIFIED_SINCE, modified);
    let response = router.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-cache");
}
