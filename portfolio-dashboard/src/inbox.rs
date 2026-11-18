use super::*;
use axum::extract::RawQuery;
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

#[derive(Deserialize)]
struct Submission {
    received_at: String,
    name: String,
    email: String,
    message: String,
}

struct Page {
    entries: Vec<Submission>,
    older: Option<u64>,
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
        entries.push(serde_json::from_slice(line).map_err(|_| invalid())?);
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

fn render(page: Page, paginated: bool) -> String {
    let mut html = String::new();
    if page.entries.is_empty() {
        html.push_str("<p>No messages yet.</p>");
    }
    for record in page.entries {
        html.push_str("<article><h2 dir=\"auto\">");
        escape(&record.name, &mut html);
        html.push_str("</h2><p class=\"meta\" dir=\"auto\">");
        escape(&record.email, &mut html);
        html.push_str("</p><p class=\"meta\">");
        escape(&record.received_at, &mut html);
        html.push_str("</p><p class=\"message\" dir=\"auto\">");
        escape(&record.message, &mut html);
        html.push_str("</p></article>");
    }
    html.push_str("<nav aria-label=\"Message pages\">");
    if paginated {
        html.push_str("<a href=\"/messages\">Newest messages</a>");
    }
    if let Some(before) = page.older {
        html.push_str(&format!(
            "<a href=\"/messages?before={before}\">Older messages</a>"
        ));
    }
    html.push_str("</nav>");
    include_str!("messages.html").replace("{{ENTRIES}}", &html)
}

pub(super) async fn messages(RawQuery(query): RawQuery) -> Response {
    let before = match query {
        None => None,
        Some(q) => match q
            .strip_prefix("before=")
            .and_then(|n| n.parse::<u64>().ok())
        {
            Some(offset) => Some(offset),
            None => return StatusCode::BAD_REQUEST.into_response(),
        },
    };
    let result = tokio::task::spawn_blocking(move || {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(FILE)?;
        if !file.metadata()?.is_file() {
            return Err(invalid());
        }
        read_page(file, before)
    })
    .await;
    match result {
        Ok(Ok(page)) => Html(render(page, before.is_some())).into_response(),
        _ => (StatusCode::SERVICE_UNAVAILABLE, "Inbox unavailable.").into_response(),
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
        assert_eq!(first.entries[0].name, "15");
        let next = first.older.unwrap();
        let html = render(first, false);
        assert!(html.contains("&lt;img src=x onerror=alert(1)&gt; &amp; done"));
        assert!(!html.contains("<img"));
        data.extend(line("new"));
        data.extend(b"{\"name\":");
        assert_eq!(
            read_page(Cursor::new(&data), None).unwrap().entries[0].name,
            "new"
        );
        let older = read_page(Cursor::new(&data), Some(next)).unwrap();
        assert_eq!(older.entries.len(), 4);
        assert_eq!(older.entries[0].name, "3");
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
