//! Fixed verbs on the root host agent socket. The dashboard decides who may
//! ask; the agent decides what it will do, by name, one request per connection.
use super::*;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const SOCKET: &str = "/run/portfolio-dashboard-agent/agent.sock";
const WIRE: usize = 64 * 1024;

/// A terminal read waits on the agent for up to eight seconds, so the client
/// timeout has to sit above that.
pub(super) async fn call(command: Value) -> Option<Value> {
    time::timeout(Duration::from_secs(30), async move {
        let mut request = serde_json::to_vec(&command).ok()?;
        if request.len() >= WIRE {
            return None;
        }
        request.push(b'\n');
        let mut socket = tokio::net::UnixStream::connect(SOCKET).await.ok()?;
        socket.write_all(&request).await.ok()?;
        socket.shutdown().await.ok()?;
        let mut body = Vec::new();
        socket
            .take((WIRE + 1) as u64)
            .read_to_end(&mut body)
            .await
            .ok()?;
        if body.len() > WIRE || !body.ends_with(b"\n") {
            return None;
        }
        let reply: Value = serde_json::from_slice(&body).ok()?;
        reply.get("ok")?.as_bool()?.then_some(reply)
    })
    .await
    .ok()?
}

/// The agent answers a named action or refuses it; nothing else is forwarded.
pub(super) async fn perform(command: Value) -> Response {
    match call(command).await {
        Some(_) => StatusCode::NO_CONTENT.into_response(),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            "The host agent did not answer.",
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn a_missing_agent_is_unavailable_rather_than_a_panic() {
        // The test host has no /run/portfolio-dashboard-agent.
        assert!(call(json!({"verb": "ping"})).await.is_none());
        assert_eq!(
            perform(json!({"verb": "ping"})).await.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
}
