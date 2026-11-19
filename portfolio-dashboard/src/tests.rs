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
        ..Status::initial()
    });
    app(App {
        viewers: Arc::new(parse_viewers(OWNER).unwrap()),
        inbox_admins: Arc::new(HashSet::new()),
        admin_origin: None,
        whois: None,
        inbox_readers: Arc::new(HashSet::new()),
        status,
        probes: None,
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
    for value in [
        "",
        "100.64",
        "100.63.0.1",
        "100.128.0.1",
        "127.0.0.1",
        "0.0.0.0",
        "example.test",
        "fd7a:115c:a1e0::1",
    ] {
        assert!(muse_address(value).is_err(), "{value}");
    }
    assert_eq!(
        muse_address("100.92.14.88").unwrap().to_string(),
        "100.92.14.88"
    );
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
        "/api/history",
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
async fn history_is_read_only_and_empty_until_the_pi_records_a_check() {
    let headers = [("tailscale-user-login", OWNER)];
    let response = call("GET", "/api/history", &headers).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "private, no-store"
    );
    let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
        serde_json::json!({"samples": []})
    );
    assert_eq!(
        call("POST", "/api/history", &headers).await.status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
}

#[test]
fn a_history_row_keeps_every_series_key() {
    let row = metrics::Sample {
        at: 30,
        jetkvm: Some(4),
        muse: None,
        portfolio: Some(80),
        cpu: Some(12.5),
        ram: Some(250),
    };
    assert_eq!(
        serde_json::to_value(row).unwrap(),
        serde_json::json!({"at": 30, "jetkvm": 4, "muse": null, "portfolio": 80, "cpu": 12.5, "ram": 250})
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
async fn inbox_requires_a_reader_and_exposes_only_a_lock_screen_to_top_level_navigation() {
    let (_, status) = watch::channel(Status {
        website: Website::Healthy,
        jetkvm: JetKvm::Reachable,
        ..Status::initial()
    });
    let state = App {
        viewers: Arc::new(parse_viewers("owner@example.test,viewer@example.test").unwrap()),
        inbox_admins: Arc::new(HashSet::new()),
        admin_origin: Some("https://pi.example.ts.net:8443".into()),
        whois: None,
        inbox_readers: Arc::new(parse_viewers("owner@example.test,outsider@example.test").unwrap()),
        status,
        probes: None,
        pi_machine: None,
        pi_ssh_user: None,
    };
    // Use a fixed response to test the access boundary without opening any host message data.
    let test_app = Router::new()
        .route("/messages", get(|| async { Html("fixture") }))
        .layer(middleware::from_fn_with_state(
            (state, Trust::Serve),
            protect,
        ));
    for (login, mode, dest, expected) in [
        (OWNER, "navigate", "document", StatusCode::OK),
        (OWNER, "navigate", "iframe", StatusCode::FORBIDDEN),
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
                ACTION_CSP
            );
            assert_eq!(response.headers()[header::X_FRAME_OPTIONS], "DENY");
            let body = to_bytes(response.into_body(), 65536).await.unwrap();
            assert_eq!(
                body.as_ref(),
                shell(VIEW_MESSAGES, &inbox::locked_fragment()).0.as_bytes()
            );
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
        ..Status::initial()
    });
    let router = app(App {
        viewers: Arc::new(parse_viewers("owner@example.test,reader@example.test").unwrap()),
        inbox_readers: Arc::new(parse_viewers("owner@example.test,reader@example.test").unwrap()),
        inbox_admins: Arc::new(parse_viewers(OWNER).unwrap()),
        admin_origin: Some(ORIGIN.to_owned()),
        whois: None,
        status,
        probes: None,
        pi_machine: None,
        pi_ssh_user: None,
    });
    // Even valid same-origin admin requests are rejected before the handler without a verified session.
    for (user, origin, site, dest, expected) in [
        (
            OWNER,
            Some(ORIGIN),
            Some("same-origin"),
            "empty",
            StatusCode::UNAUTHORIZED,
        ),
        (
            "reader@example.test",
            Some(ORIGIN),
            Some("same-origin"),
            "empty",
            StatusCode::FORBIDDEN,
        ),
        (
            OWNER,
            Some("https://treymouledoux.me"),
            Some("same-origin"),
            "empty",
            StatusCode::FORBIDDEN,
        ),
        (
            OWNER,
            None,
            Some("same-origin"),
            "empty",
            StatusCode::FORBIDDEN,
        ),
        (
            OWNER,
            Some("null"),
            Some("same-origin"),
            "empty",
            StatusCode::FORBIDDEN,
        ),
        (OWNER, Some(ORIGIN), None, "empty", StatusCode::FORBIDDEN),
        (
            OWNER,
            Some(ORIGIN),
            Some("cross-site"),
            "empty",
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
        if expected == StatusCode::UNAUTHORIZED {
            assert_eq!(
                response.headers()[header::CONTENT_SECURITY_POLICY],
                ACTION_CSP
            );
            assert_eq!(response.headers()[header::REFERRER_POLICY], "same-origin");
        }
    }
    for origin in [
        "",
        "https://pi.example.ts.net:8443/",
        "https://pi.example.ts.net/path",
        "https://p:8443",
        "https://pi.example.ts.net.",
        "https://PI.example.ts.net:8443",
        "https://pi.example.ts.net:0",
        "https://pi.example.ts.net:99999",
        "http://pi.example.ts.net:8443",
        "https://pi.example.ts.net:8443@other:8443",
    ] {
        assert!(
            inbox_actions::origin(origin.to_owned()).is_err(),
            "{origin}"
        );
    }
    for origin in [
        "https://pi.example.ts.net:8443",
        "https://pi.example.ts.net",
        "https://dashboard.treymouledoux.me",
    ] {
        assert_eq!(
            inbox_actions::origin(origin.to_owned()).unwrap(),
            origin,
            "{origin}"
        );
    }
}

#[tokio::test]
async fn one_document_carries_both_views_and_messages_swap_in_as_a_fragment() {
    let overview = shell(VIEW_OVERVIEW, "").0;
    assert!(overview.contains(r#"<section id="view-overview" aria-label="Overview">"#));
    assert!(overview.contains(r#"<section id="view-messages" aria-label="Messages" hidden>"#));
    assert!(overview.contains(r#"<a href="/messages" data-view="messages">"#));
    assert!(overview.contains(r#"aria-current="page">Overview</a>"#));
    assert!(!overview.contains("{{"));
    let locked = shell(VIEW_MESSAGES, &inbox::locked_fragment()).0;
    assert!(locked.contains(r#"<section id="view-overview" aria-label="Overview" hidden>"#));
    assert!(locked.contains(r#"<section id="view-messages" aria-label="Messages">"#));
    assert!(locked.contains(r#"<a href="/messages" data-view="messages" aria-current="page">"#));
    assert!(locked.contains(r#"<title>Messages · Tailnet dashboard</title>"#));
    assert!(locked.contains(r#"id="inbox-locked""#) && !locked.contains("{{"));

    let (_, status) = watch::channel(Status::initial());
    let router = Router::new()
        .route("/messages/view", get(|| async { Html("fixture") }))
        .layer(middleware::from_fn_with_state(
            (
                App {
                    viewers: Arc::new(parse_viewers(OWNER).unwrap()),
                    inbox_readers: Arc::new(parse_viewers(OWNER).unwrap()),
                    inbox_admins: Arc::new(HashSet::new()),
                    admin_origin: Some("https://dashboard.example.test".into()),
                    whois: None,
                    status,
                    probes: None,
                    pi_machine: None,
                    pi_ssh_user: None,
                },
                Trust::Serve,
            ),
            protect,
        ));
    let mut request = Request::builder().uri("/messages/view?filter=unread");
    for (key, value) in [
        ("tailscale-user-login", OWNER),
        ("sec-fetch-site", "same-origin"),
        ("sec-fetch-mode", "cors"),
        ("sec-fetch-dest", "empty"),
    ] {
        request = request.header(key, value);
    }
    let response = router
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_SECURITY_POLICY],
        ACTION_CSP
    );
    assert_eq!(
        to_bytes(response.into_body(), 65536)
            .await
            .unwrap()
            .as_ref(),
        inbox::locked_fragment().as_bytes()
    );
    // A top-level navigation never receives a bare fragment.
    let mut request = Request::builder().uri("/messages/view");
    for (key, value) in [
        ("tailscale-user-login", OWNER),
        ("sec-fetch-site", "same-origin"),
        ("sec-fetch-mode", "navigate"),
        ("sec-fetch-dest", "document"),
    ] {
        request = request.header(key, value);
    }
    assert_eq!(
        router
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn every_asset_the_shell_references_is_served() {
    let shell = shell(VIEW_OVERVIEW, "").0;
    let assets: Vec<&str> = shell
        .split('"')
        .filter(|value| {
            value.starts_with('/') && (value.ends_with(".js") || value.ends_with(".css"))
        })
        .collect();
    assert!(
        assets.contains(&"/app.js")
            && assets.contains(&"/dashboard.js")
            && assets.contains(&"/messages.js")
            && assets.contains(&"/passkey.js")
            && assets.contains(&"/hold-delete.js")
    );
    for path in assets {
        assert_eq!(
            call("GET", path, &[("tailscale-user-login", OWNER)])
                .await
                .status(),
            StatusCode::OK,
            "{path}"
        );
    }
}

#[tokio::test]
async fn gateway_identity_reaches_handlers_only_after_the_peer_lookup() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let path = std::path::PathBuf::from("/tmp")
        .join(format!("portfolio-view-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let helper = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut byte = [0u8; 1];
        while byte != *b"\n" && stream.read(&mut byte).await.unwrap() != 0 {
            request.push(byte[0]);
        }
        stream
            .write_all(b"{\"ok\":true,\"login\":\"owner@example.test\"}\n")
            .await
            .unwrap();
    });
    let (_, status) = watch::channel(Status::initial());
    let state = App {
        viewers: Arc::new(parse_viewers(OWNER).unwrap()),
        inbox_readers: Arc::new(HashSet::new()),
        inbox_admins: Arc::new(HashSet::new()),
        admin_origin: None,
        whois: Some(Arc::new(identity::Whois::new(path.clone()))),
        status,
        probes: None,
        pi_machine: None,
        pi_ssh_user: None,
    };
    let router = Router::new()
        .route(
            "/api/status",
            get(|headers: HeaderMap| async move {
                single_header(&headers, "tailscale-user-login")
                    .unwrap_or("none")
                    .to_owned()
            }),
        )
        .layer(middleware::from_fn_with_state(
            (state, Trust::Gateway),
            protect,
        ));
    // The handler sees the login the peer lookup proved, not the client's value.
    let response = router
        .oneshot(
            Request::builder()
                .uri("/api/status")
                .header("x-portfolio-peer", "100.64.9.9")
                .header("tailscale-user-login", "intruder@example.test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), 1024).await.unwrap().as_ref(),
        OWNER.as_bytes()
    );
    helper.await.unwrap();
    std::fs::remove_file(&path).ok();
}

#[tokio::test]
async fn gateway_trust_derives_identity_from_the_peer_socket_only() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let path = std::path::PathBuf::from("/tmp")
        .join(format!("portfolio-whois-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let helper = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut byte = [0u8; 1];
        while byte != *b"\n" {
            if stream.read(&mut byte).await.unwrap() == 0 {
                break;
            }
            request.push(byte[0]);
        }
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&request).unwrap()["peer"],
            "100.64.1.2"
        );
        stream
            .write_all(b"{\"ok\":true,\"login\":\"owner@example.test\"}\n")
            .await
            .unwrap();
    });
    let (_, status) = watch::channel(Status::initial());
    let state = App {
        viewers: Arc::new(parse_viewers(OWNER).unwrap()),
        inbox_readers: Arc::new(HashSet::new()),
        inbox_admins: Arc::new(HashSet::new()),
        admin_origin: None,
        whois: Some(Arc::new(identity::Whois::new(path.clone()))),
        status,
        probes: None,
        pi_machine: None,
        pi_ssh_user: None,
    };
    // The peer address decides identity; the client's own header is ignored.
    let request = Request::builder()
        .uri("/")
        .header("x-portfolio-peer", "100.64.1.2")
        .header("tailscale-user-login", "intruder@example.test")
        .body(Body::empty())
        .unwrap();
    let response = app_with(state.clone(), Trust::Gateway)
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    helper.await.unwrap();
    // Without a peer address there is no identity to look up.
    let request = Request::builder()
        .uri("/")
        .header("tailscale-user-login", OWNER)
        .body(Body::empty())
        .unwrap();
    let response = app_with(state, Trust::Gateway)
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    std::fs::remove_file(&path).ok();
}

#[test]
fn inbox_admin_forms_do_not_elevate_contact_markup_or_accept_non_hex_ids() {
    for value in ["ABCDEF", &"A".repeat(64), &"1".repeat(63), &"x".repeat(64)] {
        assert!(!inbox_actions::valid_hex(value, 64));
    }
    assert!(inbox_actions::valid_hex(&"a".repeat(64), 64));
    assert!(
        ACTION_CSP.contains("frame-ancestors 'none'") && ACTION_CSP.contains("form-action 'self'")
    );
}

#[tokio::test]
async fn authentication_api_allows_configured_readers_and_rejects_cross_origin_requests() {
    const ORIGIN: &str = "https://pi.example.ts.net:8443";
    let (_, status) = watch::channel(Status {
        website: Website::Healthy,
        jetkvm: JetKvm::Reachable,
        ..Status::initial()
    });
    let router = app(App {
        viewers: Arc::new(parse_viewers("owner@example.test,reader@example.test").unwrap()),
        inbox_readers: Arc::new(parse_viewers("owner@example.test,reader@example.test").unwrap()),
        inbox_admins: Arc::new(parse_viewers(OWNER).unwrap()),
        admin_origin: Some(ORIGIN.into()),
        whois: None,
        status,
        probes: None,
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
            StatusCode::BAD_REQUEST,
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
        ("reader@example.test", StatusCode::OK),
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

#[tokio::test]
async fn reader_resources_have_independent_roles_and_no_cross_origin_export_or_mutation() {
    // Fixture endpoints exercise middleware independently of host filesystem permissions.
    let (_, status) = watch::channel(Status::initial());
    let state = App {
        viewers: Arc::new(parse_viewers("owner@example.test,viewer@example.test").unwrap()),
        inbox_admins: Arc::new(HashSet::new()),
        admin_origin: Some("https://pi.example.ts.net:8443".into()),
        whois: None,
        inbox_readers: Arc::new(parse_viewers(OWNER).unwrap()),
        status,
        probes: None,
        pi_machine: None,
        pi_ssh_user: None,
    };
    let r = Router::new()
        .route("/messages/export/download", get(|| async { "export" }))
        .route(
            "/messages/read/{id}",
            axum::routing::post(|| async { "marked" }),
        )
        .layer(middleware::from_fn_with_state(
            (state, Trust::Serve),
            protect,
        ));
    for (user, path, method, site, dest, origin, allowed) in [
        (
            "viewer@example.test",
            "/messages/export/download",
            "GET",
            "same-origin",
            "document",
            "",
            "no",
        ),
        (
            OWNER,
            "/messages/export/download",
            "GET",
            "cross-site",
            "document",
            "",
            "no",
        ),
        (
            OWNER,
            "/messages/export/download",
            "GET",
            "same-origin",
            "document",
            "",
            "no",
        ),
        (
            OWNER,
            "/messages/read/abc",
            "POST",
            "cross-site",
            "document",
            "https://treymouledoux.me",
            "no",
        ),
        (
            OWNER,
            "/messages/read/abc",
            "POST",
            "same-origin",
            "empty",
            "https://pi.example.ts.net:8443",
            "no",
        ),
    ] {
        let req = Request::builder()
            .uri(path)
            .method(method)
            .header("tailscale-user-login", user)
            .header("sec-fetch-site", site)
            .header("sec-fetch-mode", "navigate")
            .header("sec-fetch-dest", dest)
            .header("origin", origin)
            .body(Body::empty())
            .unwrap();
        let response = r.clone().oneshot(req).await.unwrap();
        assert_eq!(
            response.status().is_success(),
            allowed == "yes",
            "{path} {user} {site}"
        );
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "private, no-store"
        );
    }
}
