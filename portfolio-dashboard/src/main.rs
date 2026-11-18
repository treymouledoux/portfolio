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
const CSP: &str = "default-src 'none'; style-src 'self'; script-src 'self'; connect-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";
const DENIAL_CSP: &str = "default-src 'none'; style-src 'sha256-Rzss4JSMcSpfczKxWigH8o/Y9+XPZzhGFiCwY+zW/pA='; frame-ancestors 'none'; base-uri 'none'; form-action 'none'";
const EMBED_CSP: &str = "default-src 'none'; style-src 'sha256-wPhQPM94WdFhh0fDNsbhhmFNAtX0MOYyOjT0N0aX/3s='; frame-ancestors https://treymouledoux.me; sandbox; base-uri 'none'; form-action 'none'";

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
}
#[derive(Serialize)]
struct Snapshot {
    #[serde(flatten)]
    status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pi_machine: Option<String>,
}
#[derive(Clone)]
struct App {
    viewers: Arc<HashSet<String>>,
    status: watch::Receiver<Status>,
    pi_machine: Option<String>,
}

fn pi_machine(name: String) -> Result<String, &'static str> {
    if name.is_empty()
        || name.len() > 63
        || name.starts_with('-')
        || name.ends_with('-')
        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err("DASHBOARD_PI_MACHINE must be a short DNS label");
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

async fn protect(State(app): State<App>, req: Request, next: Next) -> Response {
    let allowed = single_header(req.headers(), "tailscale-user-login")
        .is_some_and(|login| app.viewers.contains(login));
    // A link from the portfolio may navigate to the home page. Once loaded,
    // its subresources and status calls must come from the private origin.
    let site = single_header(req.headers(), "sec-fetch-site");
    let embedding = req.uri().path() == "/embed/status";
    let navigation = (req.method() == Method::GET || req.method() == Method::HEAD)
        && single_header(req.headers(), "sec-fetch-mode") == Some("navigate")
        && (req.uri().path() == "/"
            || (embedding && single_header(req.headers(), "sec-fetch-dest") == Some("iframe")));
    let foreign_request = req.headers().contains_key("sec-fetch-site")
        && !matches!(site, Some("same-origin" | "none"))
        && !navigation;
    let mut response = if allowed && !foreign_request {
        next.run(req).await
    } else {
        (StatusCode::FORBIDDEN, Html(include_str!("denied.html"))).into_response()
    };
    let embed_document = embedding && response.status().is_success();
    let headers = response.headers_mut();
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    if embed_document {
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(EMBED_CSP),
        );
    } else {
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(if !allowed || foreign_request {
                DENIAL_CSP
            } else {
                CSP
            }),
        );
        headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
        headers.insert(
            "cross-origin-resource-policy",
            HeaderValue::from_static("same-origin"),
        );
    }
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
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

async fn embed(State(app): State<App>) -> impl IntoResponse {
    let status = *app.status.borrow();
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
    Html(
        include_str!("embed.html")
            .replace("{{WEBSITE_STATUS}}", website)
            .replace("{{WEBSITE_TONE}}", web_tone)
            .replace("{{JETKVM_STATUS}}", jetkvm)
            .replace("{{JETKVM_TONE}}", kvm_tone),
    )
}

fn app(state: App) -> Router {
    Router::new()
        .route("/", get(|| async { Html(include_str!("dashboard.html")) }))
        .route("/embed/status", get(embed))
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
            "/api/status",
            get(|State(app): State<App>| async move {
                Json(Snapshot {
                    status: *app.status.borrow(),
                    pi_machine: app.pi_machine,
                })
            }),
        )
        .layer(middleware::from_fn_with_state(state.clone(), protect))
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

async fn monitor(client: reqwest::Client, jetkvm: SocketAddr, state: watch::Sender<Status>) {
    let mut interval = time::interval(PROBE_INTERVAL);
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        let (healthy, tcp) = tokio::join!(
            website_healthy(&client),
            time::timeout(CHECK_TIMEOUT, TcpStream::connect(jetkvm)),
        );
        state.send_replace(Status {
            website: if healthy {
                Website::Healthy
            } else {
                Website::Unavailable
            },
            jetkvm: if matches!(tcp, Ok(Ok(_))) {
                JetKvm::Reachable
            } else {
                JetKvm::Unavailable
            },
        });
    }
}

fn inherited_listener() -> io::Result<UnixListener> {
    if env::var("LISTEN_PID").ok().and_then(|s| s.parse().ok()) != Some(std::process::id())
        || env::var("LISTEN_FDS").as_deref() != Ok("1")
        || env::var("LISTEN_FDNAMES").as_deref() != Ok("dashboard")
    {
        return Err(io::Error::other(
            "expected one systemd socket named dashboard",
        ));
    }
    // systemd supplies the first activation fd at 3. Claim it before Tokio
    // opens its own fds. fcntl also rejects a missing activation descriptor.
    if unsafe { libc::fcntl(3, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: this valid fd was passed at exec and has no other Rust owner.
    let socket = unsafe { socket2::Socket::from_raw_fd(3) };
    let local = socket.local_addr()?;
    if !local.is_unix()
        || local.as_pathname() != Some(Path::new(APP_SOCKET))
        || socket.r#type()? != socket2::Type::STREAM
    {
        return Err(io::Error::other(
            "expected private dashboard Unix stream socket",
        ));
    }
    #[cfg(target_os = "linux")]
    if !socket.is_listener()? {
        return Err(io::Error::other("expected a listening Unix socket"));
    }
    socket.set_nonblocking(true)?;
    let owned: std::os::fd::OwnedFd = socket.into();
    Ok(UnixListener::from(owned))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = inherited_listener()?;
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
    serve(listener, viewers, jetkvm, pi_machine)
}

#[tokio::main]
async fn serve(
    listener: UnixListener,
    viewers: HashSet<String>,
    jetkvm: SocketAddr,
    pi_machine: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let listener = tokio::net::UnixListener::from_std(listener)?;
    let client = reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(CHECK_TIMEOUT)
        .build()?;
    let (tx, status) = watch::channel(Status {
        website: Website::Unknown,
        jetkvm: JetKvm::Unknown,
    });
    tokio::spawn(monitor(client, jetkvm, tx));
    axum::serve(
        listener,
        app(App {
            viewers: Arc::new(viewers),
            status,
            pi_machine,
        }),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
