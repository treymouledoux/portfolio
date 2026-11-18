//! Only the Unix broker can redeem a receipt or change stored submissions.
use super::*;
use axum::extract::{Form, Path as RoutePath, RawQuery};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const SOCKET: &str = "/run/portfolio-inbox-actions/actions.sock";

pub(super) fn valid_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(super) fn origin(value: String) -> Result<String, &'static str> {
    let host = value
        .strip_prefix("https://")
        .and_then(|v| v.strip_suffix(":8443"))
        .ok_or("invalid DASHBOARD_ADMIN_ORIGIN")?;
    if !host.ends_with(".ts.net") || pi_machine(host.to_owned()).is_err() {
        return Err("DASHBOARD_ADMIN_ORIGIN must be the private Serve origin on port 8443");
    }
    Ok(value)
}

#[derive(Serialize)]
#[serde(tag = "action")]
enum Command<'a> {
    #[serde(rename = "snapshot")]
    Snapshot,
    #[serde(rename = "delete")]
    Delete { id: &'a str, receipt: &'a str },
    #[serde(rename = "delete-all")]
    DeleteAll { id: &'a str, receipt: &'a str },
}

#[derive(Deserialize)]
struct Reply {
    ok: bool,
    snapshot: Option<String>,
    count: Option<u64>,
    error: Option<String>,
}

async fn broker(cmd: Command<'_>) -> Result<Reply, ()> {
    time::timeout(Duration::from_secs(30), async move {
        let mut request = serde_json::to_vec(&cmd).map_err(|_| ())?;
        request.push(b'\n');
        let mut socket = tokio::net::UnixStream::connect(SOCKET)
            .await
            .map_err(|_| ())?;
        socket.write_all(&request).await.map_err(|_| ())?;
        socket.shutdown().await.map_err(|_| ())?;
        let mut body = Vec::new();
        socket
            .take(1025)
            .read_to_end(&mut body)
            .await
            .map_err(|_| ())?;
        if body.len() > 1024 || !body.ends_with(b"\n") {
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

fn confirmation(action: &str, id: &str, preview: &str, count: Option<u64>) -> Html<String> {
    // Only fixed action names, validated hexadecimal IDs and an already escaped preview enter this template.
    let (title, warning, button) = if action == "delete" {
        (
            "Remove message?",
            "This permanently removes this message from the Pi’s local inbox.".to_owned(),
            "Permanently remove message",
        )
    } else {
        (
            "Remove all messages?",
            format!(
                "This permanently removes all {} local inbox messages in this snapshot. If the inbox changes, you’ll need a fresh confirmation.",
                count.unwrap_or(0)
            ),
            "Permanently remove all messages",
        )
    };
    document(format!(
        r#"<h1>{title}</h1><p>{warning} Previously delivered email copies are unaffected.</p>
{preview}
<h2>Authorize on the Pi</h2><p>Connect to the Pi through your authorized SSH account and run this command. Paste the one-time code below.</p>
<pre><code>sudo /usr/local/libexec/portfolio-inbox-actions issue {action} {id}</code></pre>
<form method="post" action="/messages/action/{action}/{id}" autocomplete="off">
<label for="receipt">One-time code</label>
<input id="receipt" name="receipt" type="password" autocomplete="off" autocapitalize="none" spellcheck="false" minlength="32" maxlength="32" pattern="[0-9a-f]{{32}}" required>
<button class="danger" type="submit">{button}</button></form>"#
    ))
}

pub(super) async fn preview_one(
    RoutePath((action, id)): RoutePath<(String, String)>,
    RawQuery(q): RawQuery,
) -> Response {
    if action != "delete" || !valid_hex(&id, 64) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(end) = q
        .as_deref()
        .and_then(|q| q.strip_prefix("end="))
        .and_then(|n| n.parse::<u64>().ok())
    else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let result = tokio::task::spawn_blocking(move || inbox::read_snapshot(Some(end))).await;
    match result {
        Ok(Ok(page)) => match page.entries.first() {
            Some(entry) if entry.id == id && entry.end == end => {
                let mut html = String::new();
                inbox::render_entry(entry, &mut html, false);
                confirmation("delete", &id, &html, None).into_response()
            }
            _ => (
                StatusCode::CONFLICT,
                "This message changed. Reload the inbox.",
            )
                .into_response(),
        },
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            "Message unavailable. Reload the inbox.",
        )
            .into_response(),
    }
}

pub(super) async fn preview_all() -> Response {
    if let Ok(Reply {
        ok: true,
        snapshot: Some(id),
        count: Some(count),
        ..
    }) = broker(Command::Snapshot).await
        && valid_hex(&id, 64)
    {
        if count == 0 {
            return document("<h1>Inbox is empty</h1>".into()).into_response();
        }
        return confirmation("delete-all", &id, "", Some(count)).into_response();
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
    receipt: String,
}

pub(super) async fn commit(
    RoutePath((action, id)): RoutePath<(String, String)>,
    Form(approval): Form<Approval>,
) -> Response {
    if !valid_hex(&id, 64) || !valid_hex(&approval.receipt, 32) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let receipt = approval.receipt.as_str();
    let command = match action.as_str() {
        "delete" => Command::Delete { id: &id, receipt },
        "delete-all" => Command::DeleteAll { id: &id, receipt },
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    match broker(command).await {
        Ok(reply) if reply.ok => document(
            "<h1>Removal complete</h1><p>Refresh the inbox to see the updated list.</p>".into(),
        )
        .into_response(),
        Ok(reply) if reply.error.as_deref() == Some("stale") => (
            StatusCode::CONFLICT,
            "The inbox changed. Reload and review before confirming again.",
        )
            .into_response(),
        Ok(reply) if reply.error.as_deref() == Some("denied") => (
            StatusCode::FORBIDDEN,
            "Code invalid, expired or already used.",
        )
            .into_response(),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            "Outcome unavailable. Check the inbox before retrying.",
        )
            .into_response(),
    }
}
