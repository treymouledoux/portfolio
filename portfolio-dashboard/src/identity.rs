//! Identity for connections the HTTPS gateway forwards to the peer socket.
//!
//! The gateway strips client-supplied identity headers and passes the direct
//! peer address instead. Only this helper lookup, over a socket the dashboard
//! group can read, turns that address into a Tailscale login.

use super::*;
use serde_json::Value;
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const MAX_REPLY: usize = 4096;

pub(super) struct Whois {
    path: PathBuf,
}

impl Whois {
    pub(super) fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// One login for one peer address; any doubt is a denial.
    pub(super) async fn login(&self, peer: &str) -> Option<String> {
        if peer.is_empty()
            || peer.len() > 45
            || !peer
                .bytes()
                .all(|b| b.is_ascii_hexdigit() || b == b'.' || b == b':')
        {
            return None;
        }
        time::timeout(Duration::from_secs(2), self.query(peer))
            .await
            .ok()?
            .ok()
    }

    async fn query(&self, peer: &str) -> Result<String, ()> {
        let mut socket = tokio::net::UnixStream::connect(&self.path)
            .await
            .map_err(|_| ())?;
        let mut request = serde_json::to_vec(&serde_json::json!({"peer": peer})).map_err(|_| ())?;
        request.push(b'\n');
        socket.write_all(&request).await.map_err(|_| ())?;
        socket.shutdown().await.map_err(|_| ())?;
        let mut reply = Vec::new();
        socket
            .take((MAX_REPLY + 1) as u64)
            .read_to_end(&mut reply)
            .await
            .map_err(|_| ())?;
        if reply.len() > MAX_REPLY || !reply.ends_with(b"\n") {
            return Err(());
        }
        let value: Value = serde_json::from_slice(&reply).map_err(|_| ())?;
        if value.get("ok") != Some(&Value::Bool(true)) {
            return Err(());
        }
        let login = value.get("login").and_then(Value::as_str).ok_or(())?;
        let valid =
            !login.is_empty() && login.len() <= 320 && login.bytes().all(|b| b.is_ascii_graphic());
        valid.then(|| login.to_owned()).ok_or(())
    }
}
