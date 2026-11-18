use super::*;
use axum::body::{Body, to_bytes};
use tower::ServiceExt;

const OWNER: &str = "owner@example.test";
const PI_FIXTURE: &str = "example-pi";

fn fixture(pi_machine: Option<String>) -> Router {
    let (_, status) = watch::channel(Status {
        website: Website::Healthy,
        jetkvm: JetKvm::Unavailable,
    });
    app(App {
        viewers: Arc::new(parse_viewers(OWNER).unwrap()),
        status,
        pi_machine,
    })
}

async fn call(method: &str, path: &str, headers: &[(&str, &str)]) -> Response {
    let mut req = Request::builder().method(method).uri(path);
    for (key, value) in headers {
        req = req.header(*key, *value);
    }
    fixture(Some(PI_FIXTURE.into()))
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[test]
fn rejects_missing_or_ambiguous_configuration() {
    for name in [
        "",
        "-bad",
        "bad-",
        "a.b",
        " x",
        "not/a/name",
        &"x".repeat(64),
    ] {
        assert!(pi_machine(name.into()).is_err(), "{name}");
    }
    for name in ["1", "pi-2", &"a".repeat(63)] {
        assert_eq!(pi_machine(name.into()).unwrap(), name);
    }
    for value in [
        "",
        " ",
        "admin",
        "*@example.test",
        "x@a,",
        "@tailnet",
        "a@b@c",
    ] {
        assert!(parse_viewers(value).is_err(), "{value}");
    }
    assert_eq!(parse_viewers("a@b, admin@c").unwrap().len(), 2);
    for (ip, port) in [
        ("127.0.0.1", "443"),
        ("example.test", "80"),
        ("::1", "22"),
        ("100.63.0.1", "80"),
        ("100.128.0.1", "80"),
        ("100.102.0.1", "0"),
        ("fd7a:115c:a1e1::1", "443"),
    ] {
        assert!(jetkvm_endpoint(ip, port).is_err(), "{ip} {port}");
    }
    for ip in ["100.82.77.66", "fd7a:115c:a1e0::1"] {
        assert_eq!(jetkvm_endpoint(ip, "443").unwrap().port(), 443);
    }
}

#[tokio::test]
async fn health_check_requires_exact_response_and_bounded_body() {
    for (status, body, expected) in [
        (200, "ok", true),
        (200, "ok\n", true),
        (200, "ok\r\n", true),
        (200, " ok", false),
        (200, "<html>login</html>", false),
        (503, "ok", false),
        (301, "ok", false),
    ] {
        let response = axum::http::Response::builder()
            .status(status)
            .body(body)
            .unwrap();
        assert_eq!(health_response(response.into()).await, expected);
    }
}

#[tokio::test]
async fn every_private_route_and_error_requires_one_trusted_viewer() {
    for path in [
        "/",
        "/dashboard.js",
        "/dashboard.css",
        "/api/status",
        "/embed/status",
        "/missing",
    ] {
        for headers in [
            vec![],
            vec![
                ("x-forwarded-for", "100.64.0.1"),
                ("cf-connecting-ip", "100.64.0.1"),
            ],
            vec![("tailscale-user-login", "visitor@example.test")],
            vec![
                ("tailscale-user-login", OWNER),
                ("tailscale-user-login", OWNER),
            ],
            vec![(
                "tailscale-user-login",
                "owner@example.test, visitor@example.test",
            )],
        ] {
            let response = call("GET", path, &headers).await;
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
            assert_eq!(
                response.headers()[header::CACHE_CONTROL],
                "private, no-store"
            );
            assert_eq!(response.headers()[header::X_FRAME_OPTIONS], "DENY");
            assert_eq!(
                response.headers()[header::CONTENT_SECURITY_POLICY],
                DENIAL_CSP
            );
            assert_eq!(
                response.headers()[header::CONTENT_TYPE],
                "text/html; charset=utf-8"
            );
            assert_eq!(
                to_bytes(response.into_body(), 4096).await.unwrap().as_ref(),
                include_bytes!("denied.html")
            );
        }
    }
}

#[tokio::test]
async fn only_fixed_status_is_returned_without_writes_or_cors() {
    let headers = [("tailscale-user-login", OWNER)];
    let response = call("GET", "/api/status", &headers).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_SECURITY_POLICY], CSP);
    assert!(
        !response
            .headers()
            .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN)
    );
    let bytes = to_bytes(response.into_body(), 1024).await.unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
        serde_json::json!({"website": "healthy", "jetkvm": "unavailable", "pi_machine": PI_FIXTURE})
    );
    for path in ["/", "/dashboard.css", "/dashboard.js"] {
        let response = call("GET", path, &headers).await;
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(response.headers()[header::X_FRAME_OPTIONS], "DENY");
    }
    assert_eq!(
        call("POST", "/api/status", &headers).await.status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(
        call("GET", "/api/admin", &headers).await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn omitting_the_optional_pi_label_preserves_status() {
    let request = Request::builder()
        .uri("/api/status")
        .header("tailscale-user-login", OWNER)
        .body(Body::empty())
        .unwrap();
    let response = fixture(None).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 1024).await.unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
        serde_json::json!({"website": "healthy", "jetkvm": "unavailable"})
    );
}

#[tokio::test]
async fn frames_are_limited_to_the_two_statuses_and_exact_parent() {
    let headers = [
        ("tailscale-user-login", OWNER),
        ("sec-fetch-site", "cross-site"),
        ("sec-fetch-mode", "navigate"),
        ("sec-fetch-dest", "iframe"),
    ];
    let response = call("GET", "/embed/status", &headers).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_SECURITY_POLICY],
        EMBED_CSP
    );
    assert!(EMBED_CSP.contains("frame-ancestors https://treymouledoux.me; sandbox;"));
    assert!(!response.headers().contains_key(header::X_FRAME_OPTIONS));
    let body = String::from_utf8(
        to_bytes(response.into_body(), 65_536)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(body.contains("Healthy") && body.contains("Unavailable"));
    assert!(
        !body.contains("{{")
            && !body.contains("<script")
            && !body.contains(PI_FIXTURE)
            && !body.contains("login.tailscale.com")
    );
    for path in ["/api/status", "/dashboard.js"] {
        assert_eq!(
            call("GET", path, &headers).await.status(),
            StatusCode::FORBIDDEN
        );
    }
    let fetch = [
        ("tailscale-user-login", OWNER),
        ("sec-fetch-site", "cross-site"),
        ("sec-fetch-mode", "cors"),
    ];
    assert_eq!(
        call("GET", "/embed/status", &fetch).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call("POST", "/embed/status", &headers).await.headers()[header::X_FRAME_OPTIONS],
        "DENY"
    );
}
