//! The private broker verifies passkeys and enforces expiring sessions and hold timers.
use super::*;
use axum::extract::{Form, Path as RoutePath};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const SOCKET: &str = "/run/portfolio-inbox-actions/actions.sock";

pub(super) fn valid_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The dashboard may be served by Tailscale Serve on port 8443 or by the
/// gateway on its own hostname. Either way it is one exact, lowercase HTTPS
/// origin: no path, userinfo, fragment or trailing slash.
pub(super) fn origin(value: String) -> Result<String, &'static str> {
    const INVALID: &str = "DASHBOARD_ADMIN_ORIGIN must be one exact HTTPS origin";
    let rest = value.strip_prefix("https://").ok_or(INVALID)?;
    if rest.is_empty()
        || rest.len() > 253
        || !rest.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-' | b':')
        })
    {
        return Err(INVALID);
    }
    let (host, port) = match rest.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (rest, None),
    };
    let valid_label = |label: &str| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    let labels: Vec<_> = host.split('.').collect();
    if labels.len() < 2 || !labels.iter().all(|label| valid_label(label)) {
        return Err(INVALID);
    }
    if let Some(port) = port {
        match port.parse::<u16>() {
            Ok(port) if port != 0 => {}
            _ => return Err(INVALID),
        }
    }
    Ok(value)
}

const SESSION_COOKIE: &str = "__Host-portfolio-inbox";
const MAX_WIRE: usize = 32 * 1024;

#[derive(Deserialize)]
struct Reply {
    ok: bool,
    snapshot: Option<String>,
    count: Option<u64>,
    error: Option<String>,
    flow: Option<String>,
    #[serde(rename = "publicKey")]
    public_key: Option<Value>,
    registered: Option<bool>,
    session: Option<String>,
    hold: Option<String>,
    open: Option<bool>,
}

fn session(headers: &HeaderMap) -> Option<&str> {
    let mut found = None;
    for header in headers.get_all(header::COOKIE) {
        for pair in header.to_str().ok()?.split(';') {
            let Some((name, value)) = pair.trim().split_once('=') else {
                continue;
            };
            if name == SESSION_COOKIE {
                if found.is_some() || !valid_hex(value, 64) {
                    return None;
                }
                found = Some(value);
            }
        }
    }
    found
}

async fn broker(cmd: Value) -> Result<Reply, ()> {
    time::timeout(Duration::from_secs(30), async move {
        let mut request = serde_json::to_vec(&cmd).map_err(|_| ())?;
        if request.len() >= MAX_WIRE {
            return Err(());
        }
        request.push(b'\n');
        let mut socket = tokio::net::UnixStream::connect(SOCKET)
            .await
            .map_err(|_| ())?;
        socket.write_all(&request).await.map_err(|_| ())?;
        socket.shutdown().await.map_err(|_| ())?;
        let mut body = Vec::new();
        socket
            .take((MAX_WIRE + 1) as u64)
            .read_to_end(&mut body)
            .await
            .map_err(|_| ())?;
        if body.len() > MAX_WIRE || !body.ends_with(b"\n") {
            return Err(());
        }
        serde_json::from_slice::<Reply>(&body).map_err(|_| ())
    })
    .await
    .map_err(|_| ())?
}

fn document(content: String) -> Html<String> {
    Html(include_str!("inbox-action.html").replace("{{CONTENT}}", &content))
}

pub(super) async fn unlocked(headers: &HeaderMap) -> bool {
    let (Some(user), Some(token)) = (
        single_header(headers, "tailscale-user-login"),
        session(headers),
    ) else {
        return false;
    };
    broker(json!({"action":"session", "user":user,"session":token}))
        .await
        .is_ok_and(|reply| reply.ok)
}

pub(super) async fn passkey_page() -> Html<String> {
    document(format!(
        "<h1>Dashboard passkey</h1>{}<noscript>Enable JavaScript to use a passkey.</noscript>",
        include_str!("passkey-panel.html")
    ))
}

pub(super) async fn preview_all(headers: HeaderMap) -> Response {
    let (Some(user), Some(token)) = (
        single_header(&headers, "tailscale-user-login"),
        session(&headers),
    ) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if let Ok(Reply {
        ok: true,
        snapshot: Some(id),
        count: Some(count),
        ..
    }) = broker(json!({"action":"snapshot","user":user,"session":token})).await
        && valid_hex(&id, 64)
    {
        return Json(json!({"id":id,"count":count})).into_response();
    }
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "Inbox snapshot unavailable.",
    )
        .into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Approval {
    hold: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Finish {
    flow: String,
    credential: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arm {
    operation: String,
    id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cancel {
    hold: String,
}

fn valid_operation(op: &str) -> bool {
    matches!(op, "delete" | "delete-all")
}

fn auth_error(code: &str) -> Response {
    let (status, error) = match code {
        "closed" => (StatusCode::FORBIDDEN, "closed"),
        "unregistered" => (StatusCode::NOT_FOUND, "unregistered"),
        "stale" => (StatusCode::CONFLICT, "stale"),
        "denied" => (StatusCode::FORBIDDEN, "denied"),
        _ => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
    };
    (status, Json(json!({"error":error}))).into_response()
}

pub(super) async fn auth(
    RoutePath(step): RoutePath<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let Some(user) = single_header(&headers, "tailscale-user-login") else {
        return auth_error("denied");
    };
    let mut cmd = json!({"action":step,"user":user});
    match step.as_str() {
        "enrollment" | "enroll-options" | "login-options" | "session" | "lock"
            if body == json!({}) => {}
        "enroll-finish" | "login-finish" => {
            let Ok(Finish { flow, credential }) = serde_json::from_value(body) else {
                return StatusCode::BAD_REQUEST.into_response();
            };
            if !valid_hex(&flow, 64) || !credential.is_object() {
                return StatusCode::BAD_REQUEST.into_response();
            }
            cmd["flow"] = json!(flow);
            cmd["credential"] = credential;
        }
        "arm" => {
            let Ok(Arm { operation, id }) = serde_json::from_value(body) else {
                return StatusCode::BAD_REQUEST.into_response();
            };
            if !valid_operation(&operation) || !valid_hex(&id, 64) {
                return StatusCode::BAD_REQUEST.into_response();
            }
            cmd["operation"] = json!(operation);
            cmd["id"] = json!(id);
        }
        "cancel" => {
            let Ok(Cancel { hold }) = serde_json::from_value(body) else {
                return StatusCode::BAD_REQUEST.into_response();
            };
            if !valid_hex(&hold, 64) {
                return StatusCode::BAD_REQUEST.into_response();
            }
            cmd["hold"] = json!(hold);
        }
        _ => return StatusCode::BAD_REQUEST.into_response(),
    }
    if matches!(step.as_str(), "session" | "arm" | "cancel" | "lock") {
        let Some(token) = session(&headers) else {
            return auth_error("denied");
        };
        cmd["session"] = json!(token);
    }
    let Ok(reply) = broker(cmd).await else {
        return auth_error("unavailable");
    };
    auth_reply(&step, reply)
}

fn auth_reply(step: &str, reply: Reply) -> Response {
    if !reply.ok {
        return auth_error(reply.error.as_deref().unwrap_or("unavailable"));
    }
    match step {
        "enrollment" => Json(json!({"open": reply.open.unwrap_or(false)})).into_response(),
        "enroll-options" | "login-options" => match (reply.flow, reply.public_key) {
            (Some(flow), Some(options)) if valid_hex(&flow, 64) && options.is_object() => {
                Json(json!({"flow":flow,"publicKey":options})).into_response()
            }
            _ => auth_error("unavailable"),
        },
        "enroll-finish" if reply.registered == Some(true) => {
            Json(json!({"registered":true})).into_response()
        }
        "login-finish" => {
            let Some(token) = reply.session.filter(|v| valid_hex(v, 64)) else {
                return auth_error("unavailable");
            };
            let mut result = Json(json!({"ok":true})).into_response();
            // Token stays out of JavaScript, response bodies, URLs, and logs.
            let cookie = format!(
                "{SESSION_COOKIE}={token}; Path=/; Max-Age=900; Secure; HttpOnly; SameSite=Strict"
            );
            result.headers_mut().insert(
                header::SET_COOKIE,
                HeaderValue::from_str(&cookie).expect("hex cookie"),
            );
            result
        }
        "arm" => match reply.hold {
            Some(hold) if valid_hex(&hold, 64) => Json(json!({"hold":hold})).into_response(),
            _ => auth_error("unavailable"),
        },
        "lock" => {
            let mut response = StatusCode::NO_CONTENT.into_response();
            response.headers_mut().insert(
                header::SET_COOKIE,
                HeaderValue::from_static(
                    "__Host-portfolio-inbox=; Path=/; Max-Age=0; Secure; HttpOnly; SameSite=Strict",
                ),
            );
            response
        }
        "cancel" | "session" => Json(json!({"ok":true})).into_response(),
        _ => auth_error("unavailable"),
    }
}

pub(super) async fn commit(
    RoutePath((operation, id)): RoutePath<(String, String)>,
    headers: HeaderMap,
    Form(approval): Form<Approval>,
) -> Response {
    if !valid_operation(&operation) || !valid_hex(&id, 64) || !valid_hex(&approval.hold, 64) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let (Some(user), Some(session)) = (
        single_header(&headers, "tailscale-user-login"),
        session(&headers),
    ) else {
        return auth_error("denied");
    };
    let command = json!({
        "action": "commit",
        "user": user,
        "session": session,
        "operation": operation,
        "id": id,
        "hold": approval.hold,
    });
    match broker(command).await {
        Ok(reply) if reply.ok => StatusCode::NO_CONTENT.into_response(),
        Ok(reply) if reply.error.as_deref() == Some("stale") => (
            StatusCode::CONFLICT,
            "The inbox changed. Reload and review before confirming again.",
        )
            .into_response(),
        Ok(reply) if reply.error.as_deref() == Some("early") => (
            StatusCode::CONFLICT,
            "Hold for the full five seconds before confirming.",
        )
            .into_response(),
        Ok(reply) if reply.error.as_deref() == Some("denied") => (
            StatusCode::FORBIDDEN,
            "Approval expired or already used. Reopen the confirmation page.",
        )
            .into_response(),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            "Outcome unavailable. Check the inbox before retrying.",
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn verification_sets_a_host_only_httponly_cookie_and_never_returns_session_json() {
        let token = "b".repeat(64);
        let response = auth_reply(
            "login-finish",
            serde_json::from_value(json!({
                "ok":true, "session":token
            }))
            .unwrap(),
        );
        assert_eq!(response.status(), StatusCode::OK);
        let cookie = response.headers()[header::SET_COOKIE].to_str().unwrap();
        assert!(cookie.starts_with("__Host-portfolio-inbox="));
        for attr in [
            "Path=/",
            "Max-Age=900",
            "Secure",
            "HttpOnly",
            "SameSite=Strict",
        ] {
            assert!(cookie.split("; ").any(|a| a == attr));
        }
        assert!(!cookie.contains("Domain="));
        let body = axum::body::to_bytes(response.into_body(), 2048)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            json!({"ok":true})
        );
        let enrollment = auth_reply(
            "enroll-finish",
            serde_json::from_value(json!({
                "ok":true, "registered":true,"session":token
            }))
            .unwrap(),
        );
        assert!(enrollment.headers().get(header::SET_COOKIE).is_none());
    }

    #[test]
    fn duplicate_or_invalid_cookie_never_authenticates() {
        let token = "a".repeat(64);
        let mut headers = HeaderMap::new();
        assert!(session(&headers).is_none());
        headers.append(
            header::COOKIE,
            format!("other=1; {SESSION_COOKIE}={token}")
                .parse()
                .unwrap(),
        );
        assert_eq!(session(&headers), Some(token.as_str()));
        headers.append(
            header::COOKIE,
            format!("{SESSION_COOKIE}={token}").parse().unwrap(),
        );
        assert!(session(&headers).is_none());
        headers.insert(
            header::COOKIE,
            format!("{SESSION_COOKIE}=xx").parse().unwrap(),
        );
        assert!(session(&headers).is_none());
    }
}
