use axum::{
    Json, Router,
    extract::{ConnectInfo, DefaultBodyLimit, State, rejection::JsonRejection},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    fs::{File, OpenOptions},
    io::AsyncWriteExt,
    net::TcpListener,
    sync::Mutex,
};
use tower_http::{
    compression::CompressionLayer,
    services::{ServeDir, ServeFile},
};

const BODY_LIMIT: usize = 64 * 1024;
const COOLDOWN: Duration = Duration::from_secs(3600);
const ATTEMPT_WINDOW: Duration = Duration::from_secs(60);
const MAX_CLIENTS: usize = 4096;

#[derive(Deserialize)]
struct ContactForm {
    name: String,
    email: String,
    message: String,
}
#[derive(Serialize)]
struct StoredSubmission {
    received_at: String,
    name: String,
    email: String,
    message: String,
}
#[derive(Serialize)]
struct ContactResponse {
    success: bool,
    reply_message: String,
}
struct ClientLimit {
    window: Instant,
    attempts: u32,
    last_success: Option<Instant>,
    seen: Instant,
}
struct Store {
    file: File,
    healthy: bool,
    clients: HashMap<IpAddr, ClientLimit>,
}
#[derive(Clone)]
struct AppState {
    store: Arc<Mutex<Store>>,
    trust_cloudflare: bool,
}

fn reply(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(ContactResponse {
            success: status.is_success(),
            reply_message: message.into(),
        }),
    )
        .into_response()
}
fn limited(seconds: u64) -> Response {
    let mut response = reply(
        StatusCode::TOO_MANY_REQUESTS,
        "Please wait before sending another message.",
    );
    response.headers_mut().insert(
        header::RETRY_AFTER,
        HeaderValue::from_str(&seconds.max(1).to_string()).unwrap(),
    );
    response
}
fn client_ip(peer: SocketAddr, headers: &HeaderMap, trust_cloudflare: bool) -> IpAddr {
    // Only explicitly configured, local cloudflared traffic may supply this header.
    if trust_cloudflare
        && peer.ip().is_loopback()
        && let Some(ip) = headers
            .get("cf-connecting-ip")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
    {
        return ip;
    }
    peer.ip()
}
fn validate(form: &ContactForm) -> Result<(), &'static str> {
    if form.name.trim().is_empty() {
        return Err("Please enter your name.");
    }
    // Match JavaScript/HTML length rules (UTF-16 units), rather than UTF-8 bytes.
    if form.name.encode_utf16().count() > 50 {
        return Err("Name is too long.");
    }
    let email = form.email.trim();
    if !email.contains('@')
        || !email.contains('.')
        || !(5..=254).contains(&email.encode_utf16().count())
    {
        return Err("Please enter a valid email address.");
    }
    let length = form.message.trim().encode_utf16().count();
    if length < 100 {
        return Err("Message is too short, please add a bit more information about your request.");
    }
    if length > 5000 {
        return Err("Message is too long (5000 characters max).");
    }
    Ok(())
}
async fn contact_limits(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let Some(peer) = request.extensions().get::<ConnectInfo<SocketAddr>>() else {
        return reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Client connection unavailable.",
        );
    };
    let ip = client_ip(peer.0, request.headers(), state.trust_cloudflare);
    let now = Instant::now();
    {
        let mut store = state.store.lock().await;
        store
            .clients
            .retain(|_, c| now.duration_since(c.seen) < COOLDOWN);
        if !store.clients.contains_key(&ip) && store.clients.len() >= MAX_CLIENTS {
            return reply(StatusCode::SERVICE_UNAVAILABLE, "Please try again later.");
        }
        let client = store.clients.entry(ip).or_insert(ClientLimit {
            window: now,
            attempts: 0,
            last_success: None,
            seen: now,
        });
        client.seen = now;
        if let Some(last) = client.last_success
            && let Some(remaining) = COOLDOWN.checked_sub(now.duration_since(last))
        {
            return limited(remaining.as_secs() + 1);
        }
        if now.duration_since(client.window) >= ATTEMPT_WINDOW {
            client.window = now;
            client.attempts = 0;
        }
        if client.attempts >= 5 {
            return limited((ATTEMPT_WINDOW - now.duration_since(client.window)).as_secs() + 1);
        }
        client.attempts += 1;
    }
    next.run(request).await
}
async fn contact(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    payload: Result<Json<ContactForm>, JsonRejection>,
) -> Response {
    let form = match payload {
        Ok(Json(form)) => form,
        Err(error) => {
            return reply(
                error.status(),
                if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    "Request is too large."
                } else {
                    "Please send valid JSON with name, email, and message fields."
                },
            );
        }
    };
    if let Err(reason) = validate(&form) {
        return reply(StatusCode::BAD_REQUEST, reason);
    }
    let ip = client_ip(peer, &headers, state.trust_cloudflare);
    let mut store = state.store.lock().await;
    // Recheck under the same lock as the write: concurrent requests cannot both succeed.
    if let Some(last) = store.clients.get(&ip).and_then(|c| c.last_success)
        && let Some(remaining) = COOLDOWN.checked_sub(last.elapsed())
    {
        return limited(remaining.as_secs() + 1);
    }
    if !store.healthy {
        return reply(
            StatusCode::SERVICE_UNAVAILABLE,
            "Message storage is unavailable. Please try again later.",
        );
    }
    let record = StoredSubmission {
        received_at: Utc::now().to_rfc3339(),
        name: form.name.trim().into(),
        email: form.email.trim().into(),
        message: form.message.trim().into(),
    };
    let mut line = serde_json::to_vec(&record).expect("string fields serialize");
    line.push(b'\n');
    let result = async {
        store.file.write_all(&line).await?;
        store.file.flush().await?;
        store.file.sync_data().await
    }
    .await;
    if let Err(error) = result {
        // Avoid appending to a potentially partial record after a disk failure.
        store.healthy = false;
        eprintln!("failed to persist submission: {error}");
        return reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Something went wrong on our end, please try again.",
        );
    }
    if let Some(client) = store.clients.get_mut(&ip) {
        client.last_success = Some(Instant::now());
    }
    println!("Contact submission saved");
    reply(
        StatusCode::OK,
        format!("Thanks {}, we'll get back to you soon!", form.name.trim()),
    )
}
async fn response_headers(request: axum::extract::Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let mut response = next.run(request).await;
    let cache = if path.starts_with("/api/")
        || path == "/healthz"
        || !(response.status().is_success() || response.status() == StatusCode::NOT_MODIFIED)
    {
        "no-store"
    } else if path == "/resume"
        || path == "/resume.pdf"
        || path.ends_with('/')
        || path.ends_with(".html")
        || response
            .headers()
            .get(header::CONTENT_TYPE)
            .is_some_and(|v| v.as_bytes().starts_with(b"text/html"))
    {
        "no-cache"
    } else if path.starts_with("/_next/static/") {
        "public, max-age=31536000, immutable"
    } else {
        "public, max-age=3600"
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    // Precompressed ServeDir responses need an encoding-aware cache key even
    // when the dynamic compression layer doesn't touch their body.
    response
        .headers_mut()
        .append(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    if (path == "/resume" || path == "/resume.pdf") && response.status().is_success() {
        response.headers_mut().insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_static("inline; filename=\"Trey-Mouledoux-Resume.pdf\""),
        );
    }
    response
}
fn app(assets: &Path, state: AppState) -> Router {
    Router::new()
        .route(
            "/api/contact",
            post(contact)
                .layer(DefaultBodyLimit::max(BODY_LIMIT))
                .route_layer(middleware::from_fn_with_state(
                    state.clone(),
                    contact_limits,
                )),
        )
        .route("/healthz", get(health))
        .route(
            "/github",
            get(|| async { Redirect::temporary("https://github.com/treymouledoux") }),
        )
        .route(
            "/instagram",
            get(|| async { Redirect::temporary("https://www.instagram.com/treymouledoux/") }),
        )
        .route(
            "/linkedin",
            get(|| async { Redirect::temporary("https://www.linkedin.com/in/trey-mouledoux/") }),
        )
        .route_service("/resume", ServeFile::new(assets.join("resume.pdf")))
        .fallback_service(
            ServeDir::new(assets)
                .append_index_html_on_directories(true)
                .precompressed_br()
                .precompressed_gzip(),
        )
        .layer(CompressionLayer::new())
        .layer(middleware::from_fn(response_headers))
        .with_state(state)
}
async fn health(State(state): State<AppState>) -> (StatusCode, &'static str) {
    if state.store.lock().await.healthy {
        (StatusCode::OK, "ok")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "storage unavailable")
    }
}
async fn open_state(data: &Path, trust_cloudflare: bool) -> std::io::Result<AppState> {
    tokio::fs::create_dir_all(data).await?;
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options.open(data.join("submissions.jsonl")).await?;
    file.sync_all().await?;
    #[cfg(unix)]
    File::open(data).await?.sync_all().await?;
    Ok(AppState {
        store: Arc::new(Mutex::new(Store {
            file,
            healthy: true,
            clients: HashMap::new(),
        })),
        trust_cloudflare,
    })
}
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("install Ctrl-C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = terminate => {} }
    println!("Shutting down; draining active requests");
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bind: SocketAddr = std::env::var("PORTFOLIO_BIND")
        .unwrap_or_else(|_| "127.0.0.1:6050".into())
        .parse()?;
    let assets =
        PathBuf::from(std::env::var_os("PORTFOLIO_ASSETS_DIR").unwrap_or_else(|| "assets".into()));
    let data =
        PathBuf::from(std::env::var_os("PORTFOLIO_DATA_DIR").unwrap_or_else(|| "data".into()));
    let trust_cloudflare = std::env::var("PORTFOLIO_TRUST_CLOUDFLARE").is_ok_and(|v| v == "true");
    if trust_cloudflare && !bind.ip().is_loopback() {
        return Err("Cloudflare header trust requires a loopback listener".into());
    }
    if !assets.join("index.html").is_file() || !assets.join("resume.pdf").is_file() {
        return Err("assets must contain index.html and resume.pdf".into());
    }
    let state = open_state(&data, trust_cloudflare).await?;
    let listener = TcpListener::bind(bind).await?;
    println!("Portfolio listening on {}", listener.local_addr()?);
    axum::serve(
        listener,
        app(&assets, state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
