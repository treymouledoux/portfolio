#[path = "inbox_reader.rs"]
mod reader;
pub(super) use reader::{badge, export, mark};
pub(super) const BADGE_CSP: &str = "default-src 'none'; style-src 'sha256-zhIDt/Sy1teYyEPTgKh9PFPMEKmd0gN16w5acUKlZeA='; frame-ancestors https://treymouledoux.me; sandbox; base-uri 'none'; form-action 'none'";

use super::*;
use axum::extract::RawQuery;
use ring::digest::{SHA256, digest};
use serde::Deserialize;
use std::{
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom},
    os::unix::fs::OpenOptionsExt,
};

const FILE: &str = "/run/portfolio-inbox/submissions.jsonl";
const PAGE_SIZE: usize = 12;
// A valid submission of 5,000 UTF-16 units fits even after JSON escaping.
const MAX_RECORD: usize = 64 * 1024;
const WINDOW: usize = 2 * MAX_RECORD + 2;

#[derive(Deserialize, Serialize)]
struct Submission {
    received_at: String,
    name: String,
    email: String,
    message: String,
}

pub(super) struct Entry {
    pub(super) id: String,
    pub(super) end: u64,
    data: Submission,
}

pub(super) struct Page {
    pub(super) entries: Vec<Entry>,
    older: Option<u64>,
}

fn record_id(line: &[u8]) -> String {
    digest(&SHA256, line)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn unavailable() -> Response {
    (StatusCode::SERVICE_UNAVAILABLE, "Inbox unavailable.").into_response()
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid submission data")
}

// Read a bounded window ending at a previous page's first record. Appends
// between requests leave that byte offset stable. Never display a partial line.
fn read_page(mut file: impl Read + Seek, before: Option<u64>) -> io::Result<Page> {
    let len = file.seek(SeekFrom::End(0))?;
    let end = before.unwrap_or(len);
    if end > len {
        return Err(invalid());
    }
    let start = end.saturating_sub(WINDOW as u64);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = vec![0; (end - start) as usize];
    file.read_exact(&mut bytes)?;
    let mut cursor = bytes.len();
    if cursor > 0 && bytes.last() != Some(&b'\n') {
        if before.is_some() {
            return Err(invalid());
        }
        // The public writer may be partway through a new record.
        cursor = bytes.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
        if bytes.len() - cursor > MAX_RECORD {
            return Err(invalid());
        }
    }
    let mut entries = Vec::new();
    while cursor > 0 && entries.len() < PAGE_SIZE {
        let line_end = cursor - 1;
        let previous = bytes[..line_end].iter().rposition(|&b| b == b'\n');
        if previous.is_none() && start != 0 {
            // This window begins inside a record, so fetch it on the next page.
            if entries.is_empty() {
                return Err(invalid());
            }
            break;
        }
        let line_start = previous.map_or(0, |i| i + 1);
        let line = &bytes[line_start..line_end];
        if line.len() > MAX_RECORD {
            return Err(invalid());
        }
        let id = record_id(line);
        entries.push(Entry {
            id,
            end: start + cursor as u64,
            data: serde_json::from_slice(line).map_err(|_| invalid())?,
        });
        cursor = line_start;
    }
    let offset = start + cursor as u64;
    Ok(Page {
        entries,
        older: (offset != 0).then_some(offset),
    })
}

fn escape(value: &str, output: &mut String) {
    for c in value.chars() {
        match c {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            _ => output.push(c),
        }
    }
}

fn page_head(control: &str) -> String {
    format!(
        r#"<header class="page-head"><div><p class="eyebrow">Private inbox</p><h1>Contact messages</h1></div>{control}</header>"#
    )
}

/// The Messages view when no passkey session is active. It is served inside
/// the single-page shell and swapped in place after the session ends.
pub(super) fn locked_fragment() -> String {
    let panel = include_str!("passkey-panel.html")
        .replace(
            "Checking your session…",
            "This inbox is locked. Unlock it to view and manage contact submissions.",
        )
        .replace("Sign in with passkey", "Unlock with passkey");
    format!(
        r#"<div id="inbox-view">{head}<section id="inbox-locked">{panel}<noscript>Enable JavaScript to unlock with a passkey.</noscript></section></div>"#,
        head = page_head(""),
    )
}

fn remove_form(operation: &str, id: &str, label: &str) -> String {
    format!(
        r#"<form class="remove-form" data-operation="{operation}" data-id="{id}">
<button class="hold-delete" type="button" data-label="{label}" aria-describedby="help-{operation}-{id}" disabled><span>{label}</span></button>
<input type="hidden" name="hold" disabled><small id="help-{operation}-{id}" class="hold-help" role="status"></small></form>"#
    )
}

fn render_card(entry: &Entry, html: &mut String, admin: bool, read: Option<bool>, can_mark: bool) {
    html.push_str("<article>");
    if let Some(false) = read {
        html.push_str(r#"<span class="eyebrow">Unread</span>"#);
    }
    html.push_str("<h2 dir=\"auto\">");
    escape(&entry.data.name, html);
    html.push_str("</h2><p class=\"meta\" dir=\"auto\">");
    escape(&entry.data.email, html);
    html.push_str("</p><p class=\"meta\">");
    escape(&entry.data.received_at, html);
    html.push_str("</p><p class=\"message\" dir=\"auto\">");
    escape(&entry.data.message, html);
    html.push_str("</p>");
    if can_mark || admin {
        html.push_str(r#"<div class="entry-actions">"#);
    }
    if let (true, Some(read)) = (can_mark, read) {
        let (route, label) = if read {
            ("unread", "Mark as unread")
        } else {
            ("read", "Mark as read")
        };
        html.push_str(&format!(r#"<form class="mark-form" method="post" action="/messages/{route}/{}?end={}"><button type="submit" disabled>{label}</button></form>"#,entry.id,entry.end));
    }
    if admin {
        html.push_str(&remove_form("delete", &entry.id, "Remove message"));
    }
    if can_mark || admin {
        html.push_str("</div>");
    }
    html.push_str("</article>");
}

fn render(
    page: Page,
    paginated: bool,
    admin: bool,
    unread_only: bool,
    dir: &Path,
    can_mark: bool,
    (total, unread): (u64, u64),
) -> io::Result<String> {
    let mut html = String::new();
    let all_current = if unread_only {
        ""
    } else {
        r#" aria-current="page""#
    };
    let unread_current = if unread_only {
        r#" aria-current="page""#
    } else {
        ""
    };
    html.push_str(&format!(r#"<nav class="filter-bar" aria-label="Message filters"><a href="/messages"{all_current}>All messages</a><a href="/messages?filter=unread"{unread_current}>Unread</a><a href="/messages/export/download" download="contact-messages.json">Export JSON</a></nav>"#));
    if page.entries.is_empty() {
        html.push_str(if unread_only {
            r#"<p class="empty">No unread messages.</p>"#
        } else {
            r#"<p class="empty">No messages yet.</p>"#
        });
    } else if admin && !unread_only {
        html.push_str(r#"<div class="bulk">"#);
        html.push_str(&remove_form("delete-all", "", "Remove all messages"));
        html.push_str("</div>");
    }
    for entry in &page.entries {
        render_card(
            entry,
            &mut html,
            admin,
            Some(reader::was_read(dir, &entry.id)?),
            can_mark,
        );
    }
    html.push_str(r#"<nav class="page-links" aria-label="Message pages">"#);
    if paginated {
        let url = if unread_only {
            "/messages?filter=unread"
        } else {
            "/messages"
        };
        html.push_str(&format!(r#"<a href="{url}">Newest messages</a>"#));
    }
    if let Some(before) = page.older {
        let filter = if unread_only {
            "&amp;filter=unread"
        } else {
            ""
        };
        html.push_str(&format!(
            "<a href=\"/messages?before={before}{filter}\">Older messages</a>"
        ));
    }
    html.push_str("</nav>");
    // Counts travel as data so the shell can show the unread badge without a
    // second archive read, and the actions stay server-rendered.
    Ok(format!(
        r#"<div id="inbox-view" data-unread="{unread}" data-total="{total}">{head}<div id="inbox-notice" role="status"></div>{html}</div>"#,
        head = page_head(r#"<button id="inbox-lock" type="button" disabled>Lock inbox</button>"#),
    ))
}

fn open_archive() -> io::Result<std::fs::File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(FILE)?;
    if !file.metadata()?.is_file() {
        return Err(invalid());
    }
    Ok(file)
}

pub(super) fn read_snapshot(before: Option<u64>) -> io::Result<Page> {
    read_page(open_archive()?, before)
}

/// The protected Messages view. Both the standalone page and the in-page
/// swap render it here, so they can never disagree about filters or roles.
async fn fragment(
    app: App,
    headers: HeaderMap,
    query: Option<String>,
) -> Result<String, Box<Response>> {
    let mut unread_only = false;
    let mut before = None;
    if let Some(query) = query {
        for part in query.split('&') {
            match part.split_once('=') {
                Some(("before", n))
                    if before.is_none() && n.bytes().all(|b| b.is_ascii_digit()) =>
                {
                    let Ok(offset) = n.parse::<u64>() else {
                        return Err(Box::new(StatusCode::BAD_REQUEST.into_response()));
                    };
                    before = Some(offset);
                }
                Some(("filter", "unread")) if !unread_only => unread_only = true,
                _ => return Err(Box::new(StatusCode::BAD_REQUEST.into_response())),
            }
        }
    }
    let Ok(dir) = reader::owner_dir(&headers) else {
        return Err(Box::new(StatusCode::FORBIDDEN.into_response()));
    };
    let can_mark = app.admin_origin.is_some();
    let admin = can_mark
        && single_header(&headers, "tailscale-user-login")
            .is_some_and(|login| app.inbox_admins.contains(login));
    let Ok(slot) = reader::SCAN_SLOTS.try_acquire() else {
        return Err(Box::new(StatusCode::TOO_MANY_REQUESTS.into_response()));
    };
    let result = tokio::task::spawn_blocking(move || {
        let _slot = slot;
        let page = reader::filtered_page(open_archive()?, before, unread_only, &dir)?;
        let counts = reader::counts(open_archive()?, &dir)?;
        render(
            page,
            before.is_some(),
            admin,
            unread_only,
            &dir,
            can_mark,
            counts,
        )
    })
    .await;
    match result {
        Ok(Ok(html)) => Ok(html),
        _ => Err(Box::new(unavailable())),
    }
}

pub(super) async fn messages(
    State(app): State<App>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Response {
    match fragment(app, headers, query).await {
        Ok(html) => shell(VIEW_MESSAGES, &html).into_response(),
        Err(response) => *response,
    }
}

/// The view alone, for the shell's in-page navigation.
pub(super) async fn view(
    State(app): State<App>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Response {
    match fragment(app, headers, query).await {
        Ok(html) => Html(html).into_response(),
        Err(response) => *response,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn line(name: &str) -> Vec<u8> {
        let mut bytes = serde_json::to_vec(&serde_json::json!({
            "name":name, "received_at":"2026-11-18", "email":"x@y.test", "message":"<img src=x onerror=alert(1)> & done"
        })).unwrap();
        bytes.push(b'\n');
        bytes
    }

    #[test]
    fn append_stable_pages_and_inert_content() {
        let mut data = Vec::new();
        for i in 0..16 {
            data.extend(line(&i.to_string()));
        }
        let first = read_page(Cursor::new(&data), None).unwrap();
        assert_eq!(first.entries.len(), PAGE_SIZE);
        assert_eq!(first.entries[0].data.name, "15");
        let next = first.older.unwrap();
        let line = line("15");
        assert_eq!(
            first.entries[0].id,
            ring::digest::digest(&ring::digest::SHA256, &line[..line.len() - 1])
                .as_ref()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        let html = render(
            first,
            false,
            false,
            false,
            Path::new("/does-not-exist"),
            true,
            (16, 16),
        )
        .unwrap();
        assert!(html.contains("&lt;img src=x onerror=alert(1)&gt; &amp; done"));
        assert!(!html.contains("<img"));
        assert!(!html.contains("/messages/action/"));
        let html = render(
            read_page(Cursor::new(&data), None).unwrap(),
            false,
            true,
            false,
            Path::new("/does-not-exist"),
            true,
            (16, 16),
        )
        .unwrap();
        assert!(
            html.contains(r#"data-operation="delete-all""#)
                && html.contains(r#"data-operation="delete""#)
        );
        assert!(html.contains(r#"class="mark-form" method="post""#));
        assert!(!html.contains(r#"target="_blank""#));
        let locked = locked_fragment();
        assert!(locked.contains("Unlock with passkey"));
        assert!(locked.contains(r#"id="inbox-locked""#));
        assert!(!locked.contains("mark-form") && !locked.contains("remove-form"));
        data.extend(self::line("new"));
        data.extend(b"{\"name\":");
        assert_eq!(
            read_page(Cursor::new(&data), None).unwrap().entries[0]
                .data
                .name,
            "new"
        );
        let older = read_page(Cursor::new(&data), Some(next)).unwrap();
        assert_eq!(older.entries.len(), 4);
        assert_eq!(older.entries[0].data.name, "3");
        assert!(older.older.is_none());
        assert!(read_page(Cursor::new(&data), Some(next - 1)).is_err());
    }

    #[test]
    fn large_history_crosses_partial_window_without_losing_entries() {
        let mut data = Vec::new();
        let name = "x".repeat(32_000);
        for _ in 0..24 {
            data.extend(line(&name));
        }
        let mut cursor = None;
        let mut count = 0;
        loop {
            let page = read_page(Cursor::new(&data), cursor).unwrap();
            count += page.entries.len();
            cursor = page.older;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(count, 24);
        assert!(read_page(Cursor::new(vec![b'x'; WINDOW + 10]), None).is_err());
        assert!(read_page(Cursor::new(b"{malformed}\n"), None).is_err());
    }
}
