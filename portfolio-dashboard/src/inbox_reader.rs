//! Reader metadata contains only owner and record digests. The contact archive stays read-only.
use super::*;
use axum::{
    body::{Body, Bytes},
    extract::Path as RoutePath,
    http::Uri,
};
use std::{
    fs::{self, File},
    io::{BufRead, BufReader},
    path::PathBuf,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::sync::{Semaphore, mpsc};

const MARKERS: &str = "/var/lib/portfolio-inbox-read";
static EXPORT_SLOTS: Semaphore = Semaphore::const_new(2);
pub(super) static SCAN_SLOTS: Semaphore = Semaphore::const_new(4);

pub(super) fn owner_dir(headers: &HeaderMap) -> Result<PathBuf, StatusCode> {
    let user = single_header(headers, "tailscale-user-login").ok_or(StatusCode::FORBIDDEN)?;
    Ok(Path::new(MARKERS).join(record_id(user.as_bytes())))
}

pub(super) fn was_read(dir: &Path, id: &str) -> io::Result<bool> {
    match fs::symlink_metadata(dir.join(id)) {
        Ok(entry) if entry.is_file() => Ok(true),
        Ok(_) => Err(invalid()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn set_read(dir: &Path, id: &str, read: bool) -> io::Result<()> {
    let marker = dir.join(id);
    if read {
        match fs::create_dir(dir) {
            Ok(()) => File::open(dir.parent().ok_or_else(invalid)?)?.sync_all()?,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e),
        }
        match File::create_new(marker) {
            Ok(f) => f.sync_all()?,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e),
        }
    } else if let Err(e) = fs::remove_file(marker) {
        if e.kind() == io::ErrorKind::NotFound {
            return Ok(());
        }
        return Err(e);
    }
    File::open(dir)?.sync_all()
}

// Pin the inode and initial byte length. Writers append under their own lock or replace
// the inode on deletion. Ignore an unfinished final line from an in-progress append.
fn visit_snapshot(file: File, mut visit: impl FnMut(&[u8]) -> io::Result<()>) -> io::Result<()> {
    let length = file.metadata()?.len();
    let mut reader = BufReader::new(file.take(length));
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = (&mut reader)
            .take(MAX_RECORD as u64 + 2)
            .read_until(b'\n', &mut line)?;
        if n > MAX_RECORD + 1 {
            return Err(invalid());
        }
        if line.last() != Some(&b'\n') {
            return Ok(());
        }
        visit(&line[..n - 1])?;
    }
}

pub(super) fn counts(file: File, dir: &Path) -> io::Result<(u64, u64)> {
    let mut total = 0;
    let mut unread = 0;
    visit_snapshot(file, |row| {
        total += 1;
        if !was_read(dir, &record_id(row))? {
            unread += 1;
        }
        Ok(())
    })?;
    Ok((total, unread))
}

pub(crate) async fn badge(headers: HeaderMap) -> Response {
    let Ok(dir) = owner_dir(&headers) else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let Ok(slot) = SCAN_SLOTS.try_acquire() else {
        return unavailable();
    };
    match tokio::task::spawn_blocking(move || {
        let _slot = slot;
        counts(open_archive()?, &dir)
    })
    .await
    {
        Ok(Ok((total, unread))) => Html(
            include_str!("inbox-badge.html")
                .replace(
                    "{{COUNT}}",
                    &if unread > 99 {
                        "99+".into()
                    } else {
                        unread.to_string()
                    },
                )
                .replace("{{TONE}}", if unread == 0 { "empty" } else { "" })
                .replace("{{DETAIL}}", &format!("{unread} unread / {total} total")),
        )
        .into_response(),
        _ => unavailable(),
    }
}

pub(super) fn filtered_page(
    mut file: File,
    before: Option<u64>,
    unread_only: bool,
    dir: &Path,
) -> io::Result<Page> {
    // Keep only one display page in memory even when most of the history is read.
    let mut result = Vec::new();
    let mut offset = before;
    loop {
        let page = read_page(&mut file, offset)?;
        for entry in page.entries {
            if !unread_only || !was_read(dir, &entry.id)? {
                if result.len() == PAGE_SIZE {
                    return Ok(Page {
                        entries: result,
                        older: Some(entry.end),
                    });
                }
                result.push(entry);
            }
        }
        offset = page.older;
        if offset.is_none() || !unread_only {
            return Ok(Page {
                entries: result,
                older: offset,
            });
        }
    }
}

fn mark_request(path: &str, id: &str, query: Option<&str>) -> Option<(bool, u64)> {
    let read = path.starts_with("/messages/read/");
    let before = query?.strip_prefix("end=")?.parse::<u64>().ok()?;
    (before > 0 && inbox_actions::valid_hex(id, 64)).then_some((read, before))
}

fn find_entry(id: &str, end: u64) -> io::Result<Option<Entry>> {
    Ok(read_snapshot(Some(end))?
        .entries
        .into_iter()
        .next()
        .filter(|e| e.end == end && e.id == id))
}

pub(crate) async fn mark(
    headers: HeaderMap,
    RoutePath(id): RoutePath<String>,
    uri: Uri,
) -> Response {
    let Some((read, end)) = mark_request(uri.path(), &id, uri.query()) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let Ok(dir) = owner_dir(&headers) else {
        return StatusCode::FORBIDDEN.into_response();
    };
    match tokio::task::spawn_blocking(move || -> io::Result<bool> {
        if find_entry(&id, end)?.is_none() {
            return Ok(false);
        }
        set_read(&dir, &id, read)?;
        Ok(true)
    })
    .await
    {
        Ok(Ok(true)) => StatusCode::NO_CONTENT.into_response(),
        Ok(Ok(false)) => StatusCode::CONFLICT.into_response(),
        _ => unavailable(),
    }
}

struct ExportStream(mpsc::Receiver<io::Result<Bytes>>);
impl futures_core::Stream for ExportStream {
    type Item = io::Result<Bytes>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.poll_recv(cx)
    }
}

fn write_export(file: File, mut send: impl FnMut(Vec<u8>) -> io::Result<()>) -> io::Result<()> {
    send(vec![b'['])?;
    let mut first = true;
    visit_snapshot(file, |line| {
        let row: Submission = serde_json::from_slice(line)?;
        let mut chunk = if first { Vec::new() } else { vec![b','] };
        first = false;
        serde_json::to_writer(&mut chunk, &row)?;
        send(chunk)
    })?;
    send(b"]\n".to_vec())
}

pub(crate) async fn export() -> Response {
    let Ok(slot) = EXPORT_SLOTS.try_acquire() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    let Ok(Ok(file)) = tokio::task::spawn_blocking(open_archive).await else {
        return unavailable();
    };
    let (tx, rx) = mpsc::channel(2);
    tokio::task::spawn_blocking(move || {
        let _slot = slot;
        let result = write_export(file, |chunk| {
            tx.blocking_send(Ok(Bytes::from(chunk)))
                .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))
        });
        if result.is_err() {
            let _ = tx.blocking_send(Err(invalid()));
        }
    });
    (
        [
            (header::CONTENT_TYPE, "application/json"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"contact-messages.json\"",
            ),
        ],
        Body::from_stream(ExportStream(rx)),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn per_reader_counts_filtered_pages_and_json_snapshot() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp =
            std::env::temp_dir().join(format!("portfolio-reader-{}-{now}", std::process::id()));
        fs::create_dir(&temp).unwrap();
        let archive = temp.join("submissions");
        let alice = temp.join("alice");
        let bob = temp.join("bob");
        let mut out = File::create(&archive).unwrap();
        for i in 0..30 {
            let value = serde_json::json!({"name":i.to_string(),"email":"contact@example.test","message":"<script>private</script>", "received_at":"now", "internal":"omit"});
            let row = serde_json::to_vec(&value).unwrap();
            if i % 2 == 0 {
                set_read(&alice, &record_id(&row), true).unwrap();
                set_read(&alice, &record_id(&row), true).unwrap();
            }
            out.write_all(&row).unwrap();
            out.write_all(b"\n").unwrap();
        }
        out.write_all(b"{").unwrap();
        drop(out);
        let open = || File::open(&archive).unwrap();
        assert_eq!(counts(open(), &alice).unwrap(), (30, 15));
        assert_eq!(counts(open(), &bob).unwrap(), (30, 30));
        let p = filtered_page(open(), None, true, &alice).unwrap();
        assert_eq!(p.entries.len(), 12);
        assert_eq!(p.entries[11].data.name, "7");
        let older = filtered_page(open(), p.older, true, &alice).unwrap();
        assert_eq!(
            older
                .entries
                .iter()
                .map(|e| e.data.name.as_str())
                .collect::<Vec<_>>(),
            vec!["5", "3", "1"]
        );
        assert_eq!(older.older, None);
        let first = &p.entries[0];
        set_read(&alice, &first.id, true).unwrap();
        assert_eq!(counts(open(), &alice).unwrap(), (30, 14));
        set_read(&alice, &first.id, false).unwrap();
        assert_eq!(counts(open(), &alice).unwrap(), (30, 15));
        let mut exported = vec![];
        write_export(open(), |chunk| {
            exported.extend(chunk);
            Ok(())
        })
        .unwrap();
        let items: Vec<serde_json::Value> = serde_json::from_slice(&exported).unwrap();
        assert_eq!(items.len(), 30);
        assert_eq!(items[0]["message"], "<script>private</script>");
        assert!(
            items
                .iter()
                .all(|item| item.as_object().unwrap().len() == 4 && item.get("internal").is_none())
        );
        fs::remove_dir_all(temp).unwrap();
    }
}
