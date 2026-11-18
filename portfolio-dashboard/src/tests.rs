use super::*;
use axum::body::{Body, to_bytes};
use tower::ServiceExt;

const OWNER: &str = "owner@example.test";
const PI_FIXTURE: &str = "example-pi";
const SSH_USER: &str = "example-operator";

fn fixture(pi_machine: Option<String>) -> Router {
    let (_, status) = watch::channel(Status {
        website: Website::Healthy,
        jetkvm: JetKvm::Unavailable,
    });
    app(App {
        viewers: Arc::new(parse_viewers(OWNER).unwrap()),
        inbox_admins: Arc::new(HashSet::new()),
        admin_origin: None,
        inbox_readers: Arc::new(HashSet::new()),
        status,
        pi_ssh_user: pi_machine.as_ref().map(|_| SSH_USER.to_owned()),
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
        "pi.ts.net",
        "pi.example.com",
        "pi.bad-.ts.net",
        "pi.test.ts.net/other",
        &"x".repeat(64),
    ] {
        assert!(pi_machine(name.into()).is_err(), "{name}");
    }
    for name in ["1", "pi-2", "pi.test.ts.net", &"a".repeat(63)] {
        assert_eq!(pi_machine(name.into()).unwrap(), name);
    }
    for name in [
        "",
        "root",
        "-o",
        "operator@host",
        "two words",
        &"x".repeat(33),
    ] {
        assert!(pi_ssh_user(name.into()).is_err(), "{name}");
    }
    assert_eq!(pi_ssh_user(SSH_USER.into()).unwrap(), SSH_USER);
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
        "/messages",
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
        serde_json::json!({"website": "healthy", "jetkvm": "unavailable", "pi_machine": PI_FIXTURE, "pi_ssh_user": SSH_USER})
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
            && !body.contains(SSH_USER)
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

#[tokio::test]
async fn inbox_requires_reader_role_and_only_accepts_cross_site_frame_navigation() {
    let (_, status) = watch::channel(Status {
        website: Website::Healthy,
        jetkvm: JetKvm::Reachable,
    });
    let state = App {
        viewers: Arc::new(parse_viewers("owner@example.test,viewer@example.test").unwrap()),
        inbox_admins: Arc::new(HashSet::new()),
        admin_origin: None,
        inbox_readers: Arc::new(parse_viewers("owner@example.test,outsider@example.test").unwrap()),
        status,
        pi_machine: None,
        pi_ssh_user: None,
    };
    // Use a fixed response to test the access boundary without opening any host message data.
    let test_app = Router::new()
        .route("/messages", get(|| async { Html("fixture") }))
        .layer(middleware::from_fn_with_state(state, protect));
    for (login, mode, dest, expected) in [
        (OWNER, "navigate", "iframe", StatusCode::OK),
        (OWNER, "cors", "empty", StatusCode::FORBIDDEN),
        (OWNER, "no-cors", "image", StatusCode::FORBIDDEN),
        (
            "viewer@example.test",
            "navigate",
            "iframe",
            StatusCode::FORBIDDEN,
        ),
        (
            "outsider@example.test",
            "navigate",
            "iframe",
            StatusCode::FORBIDDEN,
        ),
    ] {
        let request = Request::builder()
            .uri("/messages?before=100")
            .header("tailscale-user-login", login)
            .header("sec-fetch-site", "cross-site")
            .header("sec-fetch-mode", mode)
            .header("sec-fetch-dest", dest)
            .body(Body::empty())
            .unwrap();
        let response = test_app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), expected);
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "private, no-store"
        );
        assert!(
            !response
                .headers()
                .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        );
        if expected == StatusCode::OK {
            assert_eq!(
                response.headers()[header::CONTENT_SECURITY_POLICY],
                INBOX_CSP
            );
            assert!(INBOX_CSP.contains("frame-ancestors https://treymouledoux.me; sandbox allow-popups allow-popups-to-escape-sandbox;"));
        } else {
            assert_eq!(response.headers()[header::X_FRAME_OPTIONS], "DENY");
        }
    }
}

#[tokio::test]
async fn destructive_actions_require_roles_private_origin_and_same_origin_form() {
    const ORIGIN: &str = "https://pi.example.ts.net:8443";
    let (_, status) = watch::channel(Status {
        website: Website::Healthy,
        jetkvm: JetKvm::Reachable,
    });
    let router = app(App {
        viewers: Arc::new(parse_viewers("owner@example.test,reader@example.test").unwrap()),
        inbox_readers: Arc::new(parse_viewers("owner@example.test,reader@example.test").unwrap()),
        inbox_admins: Arc::new(parse_viewers(OWNER).unwrap()),
        admin_origin: Some(ORIGIN.to_owned()),
        status,
        pi_machine: None,
        pi_ssh_user: None,
    });
    // A malformed hold verifies which callers reach the form handler without contacting the broker.
    for (user, origin, site, dest, expected) in [
        (
            OWNER,
            Some(ORIGIN),
            Some("same-origin"),
            "document",
            StatusCode::BAD_REQUEST,
        ),
        (
            "reader@example.test",
            Some(ORIGIN),
            Some("same-origin"),
            "document",
            StatusCode::FORBIDDEN,
        ),
        (
            OWNER,
            Some("https://treymouledoux.me"),
            Some("same-origin"),
            "document",
            StatusCode::FORBIDDEN,
        ),
        (
            OWNER,
            None,
            Some("same-origin"),
            "document",
            StatusCode::FORBIDDEN,
        ),
        (
            OWNER,
            Some("null"),
            Some("same-origin"),
            "document",
            StatusCode::FORBIDDEN,
        ),
        (OWNER, Some(ORIGIN), None, "document", StatusCode::FORBIDDEN),
        (
            OWNER,
            Some(ORIGIN),
            Some("cross-site"),
            "document",
            StatusCode::FORBIDDEN,
        ),
        (
            OWNER,
            Some(ORIGIN),
            Some("same-origin"),
            "iframe",
            StatusCode::FORBIDDEN,
        ),
    ] {
        let mut req = Request::builder()
            .method("POST")
            .uri(format!("/messages/action/delete/{}", "a".repeat(64)))
            .header("tailscale-user-login", user)
            .header("content-type", "application/x-www-form-urlencoded")
            .header("sec-fetch-dest", dest);
        if let Some(ori) = origin {
            req = req.header("origin", ori);
        }
        if let Some(site) = site {
            req = req.header("sec-fetch-site", site);
        }
        let response = router
            .clone()
            .oneshot(req.body(Body::from("hold=invalid")).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        assert_eq!(response.headers()[header::X_FRAME_OPTIONS], "DENY");
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "private, no-store"
        );
        if expected == StatusCode::BAD_REQUEST {
            assert_eq!(
                response.headers()[header::CONTENT_SECURITY_POLICY],
                ACTION_CSP
            );
            assert_eq!(response.headers()[header::REFERRER_POLICY], "same-origin");
        }
    }
    for origin in [
        "https://pi.example.ts.net",
        "https://pi.example.ts.net:8443/",
        "https://p.ts.net:8443",
        "https://treymouledoux.me:8443",
        "http://pi.example.ts.net:8443",
        "https://pi.example.ts.net:8443@other:8443",
    ] {
        assert!(inbox_actions::origin(origin.to_owned()).is_err());
    }
    assert_eq!(inbox_actions::origin(ORIGIN.to_owned()).unwrap(), ORIGIN);
}

#[test]
fn inbox_admin_forms_do_not_elevate_contact_markup_or_accept_non_hex_ids() {
    for value in ["ABCDEF", &"A".repeat(64), &"1".repeat(63), &"x".repeat(64)] {
        assert!(!inbox_actions::valid_hex(value, 64));
    }
    assert!(inbox_actions::valid_hex(&"a".repeat(64), 64));
    assert!(!INBOX_CSP.contains("allow-forms") && !INBOX_CSP.contains("allow-scripts"));
    assert!(
        ACTION_CSP.contains("frame-ancestors 'none'") && ACTION_CSP.contains("form-action 'self'")
    );
}

#[tokio::test]
async fn authentication_api_rejects_cross_origin_and_non_admin_requests_before_parsing() {
    const ORIGIN: &str = "https://pi.example.ts.net:8443";
    let (_, status) = watch::channel(Status {
        website: Website::Healthy,
        jetkvm: JetKvm::Reachable,
    });
    let router = app(App {
        viewers: Arc::new(parse_viewers("owner@example.test,reader@example.test").unwrap()),
        inbox_readers: Arc::new(parse_viewers("owner@example.test,reader@example.test").unwrap()),
        inbox_admins: Arc::new(parse_viewers(OWNER).unwrap()),
        admin_origin: Some(ORIGIN.into()),
        status,
        pi_machine: None,
        pi_ssh_user: None,
    });
    // The invalid ceremony payload keeps this independent of any host broker or credentials.
    for (login, origin, site, expected) in [
        (OWNER, ORIGIN, "same-origin", StatusCode::BAD_REQUEST),
        (
            OWNER,
            "https://treymouledoux.me",
            "cross-site",
            StatusCode::FORBIDDEN,
        ),
        (OWNER, "null", "same-origin", StatusCode::FORBIDDEN),
        (
            "reader@example.test",
            ORIGIN,
            "same-origin",
            StatusCode::FORBIDDEN,
        ),
    ] {
        let req = Request::builder()
            .method("POST")
            .uri("/messages/auth/login-finish")
            .header("tailscale-user-login", login)
            .header("content-type", "application/json")
            .header("sec-fetch-dest", "empty")
            .header("sec-fetch-site", site)
            .header("origin", origin)
            .body(Body::from(r#"{"flow":"bad","credential":{}}"#))
            .unwrap();
        let response = router.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), expected);
        assert_eq!(response.headers()[header::X_FRAME_OPTIONS], "DENY");
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "private, no-store"
        );
    }
    for (user, status) in [
        (OWNER, StatusCode::OK),
        ("reader@example.test", StatusCode::FORBIDDEN),
    ] {
        let req = Request::builder()
            .uri("/messages/passkey")
            .header("tailscale-user-login", user)
            .body(Body::empty())
            .unwrap();
        let response = router.clone().oneshot(req).await.unwrap();
        assert_eq!(response.status(), status);
        assert_eq!(response.headers()[header::X_FRAME_OPTIONS], "DENY");
    }
}
