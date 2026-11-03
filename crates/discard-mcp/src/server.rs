//! HTTP transport for MCP (JSON-RPC over POST), bound to loopback and guarded by a bearer token.

use crate::identity::TokenRegistry;
use crate::tools::McpHandler;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tracing::warn;

pub const MAX_BODY_BYTES: usize = 1024 * 1024;

pub struct McpServer {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl McpServer {
    /// Start on `127.0.0.1` with a free port.
    pub async fn start(handler: Arc<McpHandler>, registry: Arc<TokenRegistry>) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let addr = listener.local_addr()?;
        let state = Arc::new(AppState { handler, registry, port: addr.port() });
        let app = Router::new()
            .route("/mcp", post(mcp))
            .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
            .with_state(state);
        let (tx, rx) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            let served = axum::serve(listener, app).with_graceful_shutdown(async {
                let _ = rx.await;
            });
            if let Err(e) = served.await {
                warn!(error = %e, "mcp server stopped with an error");
            }
        });
        Ok(Self { addr, shutdown: Some(tx), task })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn url(&self) -> String {
        format!("http://{}/mcp", self.addr)
    }

    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        let _ = (&mut self.task).await;
    }
}

struct AppState {
    handler: Arc<McpHandler>,
    registry: Arc<TokenRegistry>,
    port: u16,
}

fn json_response(status: StatusCode, body: &Value) -> Response {
    (status, [(header::CONTENT_TYPE, "application/json")], body.to_string()).into_response()
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = value.strip_prefix("Bearer ")?.trim();
    (!token.is_empty()).then_some(token)
}

/// Only requests addressed to this server on loopback are served. This blocks DNS rebinding.
fn host_ok(headers: &HeaderMap, port: u16) -> bool {
    let Some(host) = headers.get(header::HOST).and_then(|h| h.to_str().ok()) else { return false };
    host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}") || host == format!("[::1]:{port}")
}

async fn mcp(State(state): State<Arc<AppState>>, headers: HeaderMap, body: Bytes) -> Response {
    if !host_ok(&headers, state.port) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(identity) = bearer(&headers).and_then(|t| state.registry.lookup(t)) else {
        warn!("mcp request with a missing or unknown token");
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Ok(request) = serde_json::from_slice::<Value>(&body) else {
        let err = json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "parse error"}});
        return json_response(StatusCode::BAD_REQUEST, &err);
    };
    // Memory work touches the disk and SQLite, so it must not run on the async threads.
    let handler = Arc::clone(&state.handler);
    let reply = tokio::task::spawn_blocking(move || handler.handle(&identity, &request)).await;
    match reply {
        Ok(Some(reply)) => json_response(StatusCode::OK, &reply),
        Ok(None) => StatusCode::ACCEPTED.into_response(),
        Err(e) => {
            warn!(error = %e, "mcp handler panicked");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
