mod identity;
mod inbox;
mod inbox_actions;
mod metrics;

use axum::{
    Json, Router,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use serde::Serialize;
use std::{
    collections::HashSet,
    env, io,
    net::{IpAddr, SocketAddr},
    os::{fd::FromRawFd, unix::net::UnixListener},
    path::Path,
    sync::Arc,
    time::Duration,
};
use tokio::{net::TcpStream, sync::watch, time};

const WEBSITE_HEALTH: &str = "https://treymouledoux.me/healthz";
const CHECK_TIMEOUT: Duration = Duration::from_secs(4);
const PROBE_INTERVAL: Duration = Duration::from_secs(30);
const APP_SOCKET: &str = "/run/portfolio-dashboard/app.sock";
const PEER_SOCKET: &str = "/run/portfolio-dashboard-peer/dashboard.sock";
const WHOIS_SOCKET: &str = "/run/portfolio-whois/whois.sock";
// The public apex, whose handoff page may probe this origin.
const APEX_ORIGIN: &str = "https://treymouledoux.me";
const VIEW_OVERVIEW: &str = "overview";
const VIEW_MESSAGES: &str = "messages";
const CSP: &str = "default-src 'none'; style-src 'self'; script-src 'self'; connect-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";
const ACTION_CSP: &str = "default-src 'none'; style-src 'self'; script-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";
const DENIAL_CSP: &str = "default-src 'none'; style-src 'sha256-Rzss4JSMcSpfczKxWigH8o/Y9+XPZzhGFiCwY+zW/pA='; frame-ancestors 'none'; base-uri 'none'; form-action 'none'";
const EMBED_CSP: &str = "default-src 'none'; style-src 'sha256-SM1MGwmvKiVaLC9I2tANbbB3OgX6X5fjRiK63pRVMFQ='; frame-ancestors https://treymouledoux.me; sandbox; base-uri 'none'; form-action 'none'";

/// One document carries both views so switching them never reloads the page.
/// The server fills the requested view, so a deep link, a reload or a browser
/// without this script still shows the right thing.
pub(crate) fn shell(view: &str, messages: &str) -> Html<String> {
    let messages_view = view == VIEW_MESSAGES;
    Html(
        include_str!("app.html")
            .replace(
                "{{TITLE}}",
                if messages_view {
                    "Messages · Tailnet dashboard"
                } else {
                    "Overview · Tailnet dashboard"
                },
            )
            .replace(
                "{{OVERVIEW_HIDDEN}}",
                if messages_view { " hidden" } else { "" },
            )
            .replace(
                "{{MESSAGES_HIDDEN}}",
                if messages_view { "" } else { " hidden" },
            )
            .replace(
                "{{OVERVIEW_CURRENT}}",
                if messages_view {
                    ""
                } else {
                    r#" aria-current="page""#
                },
            )
            .replace(
                "{{MESSAGES_CURRENT}}",
                if messages_view {
                    r#" aria-current="page""#
                } else {
                    ""
                },
            )
            .replace("{{MESSAGES}}", messages),
    )
}

async fn overview() -> Html<String> {
    shell(VIEW_OVERVIEW, "")
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
enum Website {
    Unknown,
    Healthy,
    Unavailable,
}
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
enum JetKvm {
    Unknown,
    Reachable,
    Unavailable,
}
#[derive(Clone, Copy, Serialize)]
struct Status {
    website: Website,
    jetkvm: JetKvm,
    #[serde(skip_serializing_if = "Option::is_none")]
    website_probe: Option<metrics::Reading>,
    #[serde(skip_serializing_if = "Option::is_none")]
    jetkvm_probe: Option<metrics::Reading>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ping_probe: Option<metrics::Reading>,
    #[serde(skip_serializing_if = "Option::is_none")]
    host: Option<metrics::Host>,
    #[serde(skip_serializing_if = "Option::is_none")]
    muse: Option<metrics::Muse>,
}
impl Status {
    fn initial() -> Self {
        Self {
            website: Website::Unknown,
            jetkvm: JetKvm::Unknown,
            website_probe: None,
            jetkvm_probe: None,
            ping_probe: None,
            host: None,
            muse: None,
        }
    }
    fn fresh(mut self) -> Self {
        if let Some(muse) = self.muse.as_mut() {
            muse.online = muse.current(metrics::epoch());
        }
        self
    }
}
#[derive(Serialize)]
struct Snapshot {
    #[serde(flatten)]
    status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pi_machine: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pi_ssh_user: Option<String>,
}
/// How this connection reached the app. Serve supplies its own identity
/// header; the gateway supplies only the peer address on a socket only the
/// gateway group can open.
#[derive(Clone, Copy)]
enum Trust {
    Serve,
    Gateway,
}

#[derive(Clone)]
struct App {
    viewers: Arc<HashSet<String>>,
    inbox_readers: Arc<HashSet<String>>,
    inbox_admins: Arc<HashSet<String>>,
    admin_origin: Option<String>,
    whois: Option<Arc<identity::Whois>>,
    status: watch::Receiver<Status>,
    probes: Option<Arc<metrics::Probes>>,
    pi_machine: Option<String>,
    pi_ssh_user: Option<String>,
}

fn pi_machine(name: String) -> Result<String, &'static str> {
    let valid_label = |label: &str| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    let labels: Vec<_> = name.split('.').collect();
    if !matches!(labels.as_slice(), [machine] if valid_label(machine))
        && !matches!(labels.as_slice(), [machine, tailnet, "ts", "net"] if valid_label(machine) && valid_label(tailnet))
    {
        return Err("DASHBOARD_PI_MACHINE must be a short DNS label or machine.tailnet.ts.net");
    }
    Ok(name)
}

fn pi_ssh_user(name: String) -> Result<String, &'static str> {
    if name == "root"
        || name.len() > 32
        || !name.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
        || !name
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
    {
        return Err("DASHBOARD_PI_SSH_USER must name a non-root local account");
    }
    Ok(name)
}

fn parse_viewers(value: &str) -> Result<HashSet<String>, &'static str> {
    value
        .split(',')
        .map(|s| {
            let login = s.trim();
            let (local, domain) = login.split_once('@').ok_or("invalid DASHBOARD_VIEWERS")?;
            if local.is_empty()
                || domain.is_empty()
                || domain.contains('@')
                || login.contains('*')
                || !login.bytes().all(|b| b.is_ascii_graphic())
            {
                return Err("invalid DASHBOARD_VIEWERS");
            }
            Ok(login.to_owned())
        })
        .collect()
}

fn jetkvm_endpoint(address: &str, port: &str) -> Result<SocketAddr, &'static str> {
    let ip: IpAddr = address.parse().map_err(|_| "invalid DASHBOARD_JETKVM_IP")?;
    let is_tailnet = match ip {
        IpAddr::V4(ip) => ip.octets()[0] == 100 && (64..128).contains(&ip.octets()[1]),
        IpAddr::V6(ip) => ip.segments()[..3] == [0xfd7a, 0x115c, 0xa1e0],
    };
    let port: u16 = port.parse().map_err(|_| "invalid DASHBOARD_JETKVM_PORT")?;
    if !is_tailnet || port == 0 {
        return Err("configure a numeric Tailscale node address and nonzero JetKVM port");
    }
    Ok(SocketAddr::new(ip, port))
}

fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    values.next().is_none().then_some(value)
}

async fn protect(
    State((app, trust)): State<(App, Trust)>,
    mut req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path();
    let inbox = path == "/messages";
    // Exactly the body the single-page shell swaps into its Messages view.
    let fragment = path == "/messages/view";
    // The public handoff page asks this origin whether it is reachable.
    let probe = path == "/handoff";
    let badge = path == "/embed/inbox-count";
    let inbox_path = inbox || path.starts_with("/messages/");
    let auth_api = path.starts_with("/messages/auth/");
    let destructive = path == "/messages/delete-all"
        || path.starts_with("/messages/action/")
        || matches!(path, "/messages/auth/arm" | "/messages/auth/cancel");
    // Never mix the two sources: a gateway connection ignores any identity
    // header the client sent, and Serve's path never reads a peer address.
    let login = match trust {
        Trust::Serve => single_header(req.headers(), "tailscale-user-login").map(str::to_owned),
        Trust::Gateway => match (
            app.whois.as_ref(),
            single_header(req.headers(), "x-portfolio-peer"),
        ) {
            (Some(whois), Some(peer)) => whois.login(peer).await,
            _ => None,
        },
    };
    let allowed = probe
        || login.as_deref().is_some_and(|login| {
            app.viewers.contains(login)
                && (!(inbox_path || badge) || app.inbox_readers.contains(login))
                && (!inbox_path || app.admin_origin.is_some())
                && (!destructive || app.inbox_admins.contains(login))
        });
    let site = single_header(req.headers(), "sec-fetch-site");
    let dest = single_header(req.headers(), "sec-fetch-dest");
    let embedding = badge || path == "/embed/status";
    let navigation = (req.method() == Method::GET || req.method() == Method::HEAD)
        && single_header(req.headers(), "sec-fetch-mode") == Some("navigate")
        && (path == "/"
            || (embedding && dest == Some("iframe"))
            || (inbox && dest == Some("document")));
    let foreign_request = req.headers().contains_key("sec-fetch-site")
        && !matches!(site, Some("same-origin" | "none"))
        && !navigation
        && !probe;
    let wrong_destination = inbox_path
        && dest.is_some_and(|d| {
            if inbox || path == "/messages/passkey" {
                d != "document"
            } else if path == "/messages/export/download" {
                !matches!(d, "document" | "empty")
            } else {
                d != "empty"
            }
        });
    let wrong_origin = inbox_path
        && req.method() == Method::POST
        && (site != Some("same-origin")
            || single_header(req.headers(), "origin") != app.admin_origin.as_deref());
    let denied = !allowed || foreign_request || wrong_origin || wrong_destination;
    // Only the explicitly embeddable count and the login/enrollment ceremony are
    // available before unlock. Never open the archive to render a locked page.
    let needs_session = inbox_path && !auth_api && path != "/messages/passkey";
    let hidden_badge = badge
        && login
            .as_deref()
            .is_some_and(|user| app.viewers.contains(user) && !app.inbox_readers.contains(user))
        && !foreign_request;
    let mut response = if hidden_badge {
        (StatusCode::FORBIDDEN, Html("")).into_response()
    } else if denied {
        (StatusCode::FORBIDDEN, Html(include_str!("denied.html"))).into_response()
    } else if needs_session && !inbox_actions::unlocked(req.headers()).await {
        // A locked inbox is a server-rendered state inside the page, not a
        // reload and not an error the browser has to explain.
        if inbox {
            shell(VIEW_MESSAGES, &inbox::locked_fragment()).into_response()
        } else if fragment {
            Html(inbox::locked_fragment()).into_response()
        } else {
            StatusCode::UNAUTHORIZED.into_response()
        }
    } else {
        // Handlers read the identity this connection already proved. On the
        // gateway that value came from the peer lookup, never from the client.
        if matches!(trust, Trust::Gateway) {
            req.headers_mut().remove("tailscale-user-login");
            if let Some(login) = &login
                && let Ok(value) = HeaderValue::from_str(login)
            {
                req.headers_mut().insert("tailscale-user-login", value);
            }
        }
        next.run(req).await
    };
    let embed_document = (embedding && response.status().is_success()) || hidden_badge;
    let headers = response.headers_mut();
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    if embed_document {
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(if badge { inbox::BADGE_CSP } else { EMBED_CSP }),
        );
    } else {
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(if denied {
                DENIAL_CSP
            } else if inbox_path {
                ACTION_CSP
            } else {
                CSP
            }),
        );
        headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
        headers.insert(
            "cross-origin-resource-policy",
            // Only the body-less probe may be read by the public apex.
            HeaderValue::from_static(if probe { "same-site" } else { "same-origin" }),
        );
    }
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static(if inbox_path && !denied {
            "same-origin"
        } else {
            "no-referrer"
        }),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        "cross-origin-opener-policy",
        HeaderValue::from_static("same-origin"),
    );
    headers.insert("x-robots-tag", HeaderValue::from_static("noindex"));
    response
}

/// Chrome asks a public page before it may reach a private address.
fn preflight_headers() -> (StatusCode, HeaderMap) {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static(APEX_ORIGIN),
    );
    headers.insert(
        "access-control-allow-methods",
        HeaderValue::from_static("GET"),
    );
    headers.insert(
        "access-control-allow-private-network",
        HeaderValue::from_static("true"),
    );
    (StatusCode::NO_CONTENT, headers)
}

/// Reachability probe for the public handoff page: no body, no identity, no
/// state, and readable across the apex because both share a registrable site.
async fn handoff() -> (StatusCode, HeaderMap) {
    let mut headers = HeaderMap::new();
    headers.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-site"),
    );
    (StatusCode::NO_CONTENT, headers)
}

async fn embed(
    State(app): State<App>,
    headers: HeaderMap,
    axum::extract::RawQuery(query): axum::extract::RawQuery,
) -> Response {
    let notice = match query.as_deref() {
        None => "",
        Some(q) => {
            let Some(target) = q.strip_prefix("test=").and_then(metrics::Target::parse) else {
                return StatusCode::BAD_REQUEST.into_response();
            };
            if single_header(&headers, "sec-fetch-mode") != Some("navigate")
                || single_header(&headers, "sec-fetch-user") != Some("?1")
            {
                return StatusCode::FORBIDDEN.into_response();
            }
            let Some(probes) = &app.probes else {
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            };
            probes.run(target).await
        }
    };
    let status = app.status.borrow().fresh();
    let (website, web_tone) = match status.website {
        Website::Unknown => ("Unknown", "unknown"),
        Website::Healthy => ("Healthy", "good"),
        Website::Unavailable => ("Unavailable", "bad"),
    };
    let (jetkvm, kvm_tone) = match status.jetkvm {
        JetKvm::Unknown => ("Unknown", "unknown"),
        JetKvm::Reachable => ("Reachable", "good"),
        JetKvm::Unavailable => ("Unavailable", "bad"),
    };
    let latency = |reading: Option<metrics::Reading>| match reading {
        Some(r) => match r.latency_ms {
            Some(0) => "&lt;1 ms".to_owned(),
            Some(ms) => format!("{ms} ms"),
            None => "No response".to_owned(),
        },
        None => "Awaiting check".to_owned(),
    };
    let host = status.host.unwrap_or_default();
    let muse = status.muse.and_then(|m| m.online);
    let uptime = host.uptime_seconds.map_or_else(
        || "—".into(),
        |seconds| {
            format!(
                "{}d {}h {}m",
                seconds / 86400,
                seconds / 3600 % 24,
                seconds / 60 % 60
            )
        },
    );
    let checked = status.website_probe.map_or_else(
        || "Awaiting first check".into(),
        |v| {
            format!(
                "Website checked {}s ago",
                metrics::epoch().saturating_sub(v.checked_at)
            )
        },
    );
    Html(
        include_str!("embed.html")
            .replace("{{WEBSITE_STATUS}}", website)
            .replace("{{WEBSITE_TONE}}", web_tone)
            .replace("{{JETKVM_STATUS}}", jetkvm)
            .replace("{{JETKVM_TONE}}", kvm_tone)
            .replace(
                "{{MUSE_STATUS}}",
                match muse {
                    Some(true) => "Connected",
                    Some(false) => "Offline",
                    None => "Unknown",
                },
            )
            .replace(
                "{{MUSE_TONE}}",
                match muse {
                    Some(true) => "good",
                    Some(false) => "bad",
                    None => "unknown",
                },
            )
            .replace("{{WEB_MS}}", &latency(status.website_probe))
            .replace("{{TCP_MS}}", &latency(status.jetkvm_probe))
            .replace("{{PING_MS}}", &latency(status.ping_probe))
            .replace(
                "{{CPU}}",
                &host
                    .cpu_percent
                    .map_or_else(|| "—".into(), |n| format!("{n:.1}%")),
            )
            .replace(
                "{{RAM}}",
                &host
                    .memory_used_mib
                    .zip(host.memory_total_mib)
                    .map_or_else(|| "—".into(), |(a, b)| format!("{a} / {b} MiB")),
            )
            .replace("{{UPTIME}}", &uptime)
            .replace("{{CHECKED}}", &checked)
            .replace("{{NOTICE}}", notice),
    )
    .into_response()
}

#[cfg(test)]
fn app(state: App) -> Router {
    app_with(state, Trust::Serve)
}

fn app_with(state: App, trust: Trust) -> Router {
    Router::new()
        .route("/", get(overview))
        .route(
            "/handoff",
            get(handoff).options(|| async { preflight_headers() }),
        )
        .route("/embed/status", get(embed))
        .route("/api/check/{target}", axum::routing::post(test_service))
        .route("/messages", get(inbox::messages))
        .route("/messages/view", get(inbox::view))
        .route("/embed/inbox-count", get(inbox::badge))
        .route("/messages/export/download", get(inbox::export))
        .route("/messages/read/{id}", axum::routing::post(inbox::mark))
        .route("/messages/unread/{id}", axum::routing::post(inbox::mark))
        .route("/messages/delete-all", get(inbox_actions::preview_all))
        .route("/messages/passkey", get(inbox_actions::passkey_page))
        .route(
            "/messages/auth/{step}",
            axum::routing::post(inbox_actions::auth)
                .layer(axum::extract::DefaultBodyLimit::max(30 * 1024)),
        )
        .route(
            "/passkey.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("passkey.js"),
                )
            }),
        )
        .route(
            "/messages.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("messages.css"),
                )
            }),
        )
        .route(
            "/messages.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("messages.js"),
                )
            }),
        )
        .route(
            "/hold-delete.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("hold-delete.js"),
                )
            }),
        )
        .route(
            "/messages/action/{action}/{id}",
            axum::routing::post(inbox_actions::commit)
                .layer(axum::extract::DefaultBodyLimit::max(128)),
        )
        .route(
            "/dashboard.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("dashboard.css"),
                )
            }),
        )
        .route(
            "/dashboard.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("dashboard.js"),
                )
            }),
        )
        .route(
            "/app.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("app.js"),
                )
            }),
        )
        .route(
            "/api/status",
            get(|State(app): State<App>| async move {
                Json(Snapshot {
                    status: app.status.borrow().fresh(),
                    pi_machine: app.pi_machine,
                    pi_ssh_user: app.pi_ssh_user,
                })
            }),
        )
        .layer(middleware::from_fn_with_state(
            (state.clone(), trust),
            protect,
        ))
        .with_state(state)
}

async fn website_healthy(client: &reqwest::Client) -> bool {
    let Ok(response) = client.get(WEBSITE_HEALTH).send().await else {
        return false;
    };
    health_response(response).await
}

async fn health_response(mut response: reqwest::Response) -> bool {
    if response.status() != StatusCode::OK || response.content_length().is_some_and(|n| n > 4) {
        return false;
    }
    let mut body = Vec::with_capacity(4);
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) if chunk.len() <= 4 - body.len() => body.extend_from_slice(&chunk),
            Ok(None) => return matches!(body.as_slice(), b"ok" | b"ok\n" | b"ok\r\n"),
            _ => return false,
        }
    }
}

async fn test_service(
    State(app): State<App>,
    axum::extract::Path(name): axum::extract::Path<String>,
    headers: HeaderMap,
) -> Response {
    if single_header(&headers, "sec-fetch-site") != Some("same-origin") {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(target) = metrics::Target::parse(&name) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(probes) = &app.probes else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    probes.run(target).await.into_response()
}

fn claim_listener(fd: std::os::fd::RawFd, path: &str) -> io::Result<UnixListener> {
    // systemd supplies activation fds from 3. Claim each before Tokio opens its
    // own fds. fcntl also rejects a missing activation descriptor.
    if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: this valid fd was passed at exec and has no other Rust owner.
    let socket = unsafe { socket2::Socket::from_raw_fd(fd) };
    let local = socket.local_addr()?;
    if !local.is_unix()
        || local.as_pathname() != Some(Path::new(path))
        || socket.r#type()? != socket2::Type::STREAM
    {
        return Err(io::Error::other("expected the named private Unix socket"));
    }
    #[cfg(target_os = "linux")]
    if !socket.is_listener()? {
        return Err(io::Error::other("expected a listening Unix socket"));
    }
    socket.set_nonblocking(true)?;
    let owned: std::os::fd::OwnedFd = socket.into();
    Ok(UnixListener::from(owned))
}

/// Serve activation accepts the Serve socket alone, or that socket plus the
/// gateway-only peer socket. Names, not ordering, decide which is which.
fn inherited_listeners() -> io::Result<(UnixListener, Option<UnixListener>)> {
    if env::var("LISTEN_PID").ok().and_then(|s| s.parse().ok()) != Some(std::process::id()) {
        return Err(io::Error::other("expected systemd socket activation"));
    }
    let count: usize = env::var("LISTEN_FDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| io::Error::other("expected systemd socket activation"))?;
    let names: Vec<String> = env::var("LISTEN_FDNAMES")
        .map(|value| value.split(':').map(str::to_owned).collect())
        .unwrap_or_default();
    if count != names.len() || !(1..=2).contains(&count) {
        return Err(io::Error::other(
            "expected systemd sockets named dashboard and peer",
        ));
    }
    let (mut dashboard, mut peer) = (None, None);
    for (index, name) in names.iter().enumerate() {
        let slot = match name.as_str() {
            "dashboard" if dashboard.is_none() => &mut dashboard,
            "peer" if peer.is_none() => &mut peer,
            _ => {
                return Err(io::Error::other(
                    "expected systemd sockets named dashboard and peer",
                ));
            }
        };
        *slot = Some(claim_listener(
            3 + index as std::os::fd::RawFd,
            if name == "dashboard" {
                APP_SOCKET
            } else {
                PEER_SOCKET
            },
        )?);
    }
    let dashboard = dashboard.ok_or_else(|| io::Error::other("expected the dashboard socket"))?;
    Ok((dashboard, peer))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (listener, peer) = inherited_listeners()?;
    let viewers = parse_viewers(&env::var("DASHBOARD_VIEWERS")?)?;
    let jetkvm = jetkvm_endpoint(
        &env::var("DASHBOARD_JETKVM_IP")?,
        &env::var("DASHBOARD_JETKVM_PORT")?,
    )?;
    let pi_machine = match env::var("DASHBOARD_PI_MACHINE") {
        Ok(value) => Some(pi_machine(value)?),
        Err(env::VarError::NotPresent) => None,
        Err(error) => return Err(error.into()),
    };
    let pi_ssh_user = match env::var("DASHBOARD_PI_SSH_USER") {
        Ok(value) => Some(pi_ssh_user(value)?),
        Err(env::VarError::NotPresent) => None,
        Err(error) => return Err(error.into()),
    };
    serve(listener, peer, viewers, jetkvm, pi_machine, pi_ssh_user)
}

#[tokio::main]
async fn serve(
    listener: UnixListener,
    peer: Option<UnixListener>,
    viewers: HashSet<String>,
    jetkvm: SocketAddr,
    pi_machine: Option<String>,
    pi_ssh_user: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let inbox_readers = match env::var("DASHBOARD_INBOX_READERS") {
        Ok(value) => parse_viewers(&value).map_err(|_| "invalid DASHBOARD_INBOX_READERS")?,
        Err(env::VarError::NotPresent) => HashSet::new(),
        Err(error) => return Err(error.into()),
    };
    let inbox_admins = match env::var("DASHBOARD_INBOX_ADMINS") {
        Ok(value) => parse_viewers(&value).map_err(|_| "invalid DASHBOARD_INBOX_ADMINS")?,
        Err(env::VarError::NotPresent) => HashSet::new(),
        Err(error) => return Err(error.into()),
    };
    let admin_origin = match env::var("DASHBOARD_ADMIN_ORIGIN") {
        Ok(value) => Some(inbox_actions::origin(value)?),
        Err(env::VarError::NotPresent) if inbox_admins.is_empty() => None,
        Err(error) => return Err(error.into()),
    };
    let listener = tokio::net::UnixListener::from_std(listener)?;
    let peer = match peer {
        Some(peer) => Some(tokio::net::UnixListener::from_std(peer)?),
        None => None,
    };
    let client = reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(CHECK_TIMEOUT)
        .build()?;
    let (tx, status) = watch::channel(Status::initial());
    let probes = Arc::new(metrics::Probes::new(client, jetkvm, tx));
    tokio::spawn(probes.clone().monitor());
    let state = App {
        viewers: Arc::new(viewers),
        inbox_readers: Arc::new(inbox_readers),
        inbox_admins: Arc::new(inbox_admins),
        admin_origin,
        whois: peer
            .is_some()
            .then(|| Arc::new(identity::Whois::new(WHOIS_SOCKET))),
        status,
        probes: Some(probes),
        pi_machine,
        pi_ssh_user,
    };
    match peer {
        Some(peer) => {
            tokio::try_join!(
                axum::serve(listener, app_with(state.clone(), Trust::Serve)),
                axum::serve(peer, app_with(state, Trust::Gateway)),
            )?;
        }
        None => axum::serve(listener, app_with(state, Trust::Serve)).await?,
    }
    Ok(())
}

#[cfg(test)]
mod tests;
