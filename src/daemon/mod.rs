//! `rtkk serve` HTTP/SSE Daemon lifecycle, graceful shutdown, and `rtkk healthcheck` client.

/// Token authentication, constant-time comparison, and session cookies.
pub mod auth;
/// Embedded frontend asset serving with Gzip and SHA256 ETag support.
pub mod embed;
/// Single-flight mutating job scheduler and JSONL event persistence.
pub mod jobs;
/// Recursive credential masking (`********`) and unmodified secret preservation.
pub mod mask;
/// HTTP REST and SSE route handlers (`/api/v1/*`).
pub mod routes;
/// Server-Sent Events replay and live tailing pump.
pub mod sse;
/// Shared [`DaemonState`], `X-Request-Id` middleware, and [`state::ApiError`].
pub mod state;

use crate::daemon::auth::AuthManager;
use crate::daemon::jobs::JobManager;
use crate::daemon::state::{DaemonState, request_id_middleware};
use crate::error::RtkkError;
use crate::workspace::Workspace;
use axum::Router;
use axum::middleware;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tower_http::limit::RequestBodyLimitLayer;

/// Configuration options for starting the `rtkk serve` daemon.
#[derive(Debug, Clone)]
pub struct ServeOptions {
    /// Socket address to bind (default `127.0.0.1:19912` or `RTKK_BIND_ADDR`).
    pub listen: String,
    /// Workspace root directory (default `.` or `RTKK_WORK_DIR`).
    pub work_dir: PathBuf,
    /// Optional explicit path for the `0600` admin token file.
    pub token_file: Option<PathBuf>,
}

/// Builds the full Axum [`Router`] for a pre-initialized [`DaemonState`].
pub fn build_app_router(state: DaemonState) -> Router {
    routes::build_api_router()
        .fallback(embed::serve_embedded_or_api_404)
        .layer(RequestBodyLimitLayer::new(16 * 1024 * 1024))
        .layer(middleware::from_fn(request_id_middleware))
        .with_state(state)
}

/// Initializes a [`DaemonState`] for `opts`, bootstrapping workspace configs and the `0600` token file.
///
/// # Errors
///
/// Returns [`RtkkError`] if workspace initialization, token file creation, or job store setup fails.
pub fn initialize_daemon_state(
    opts: &ServeOptions,
) -> Result<(DaemonState, PathBuf, bool), RtkkError> {
    let workspace = Workspace::new(&opts.work_dir);
    let _ = workspace.ensure_initialized()?;

    let token_path = opts
        .token_file
        .clone()
        .or_else(|| {
            std::env::var("RTKK_TOKEN_FILE")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .map(PathBuf::from)
        })
        .unwrap_or_else(|| workspace.default_token_file());

    let explicit_token = std::env::var("RTKK_AUTH_TOKEN")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let (auth_mgr, generated_token) =
        AuthManager::initialize(explicit_token.as_deref(), &token_path)?;
    let job_mgr = JobManager::initialize(&workspace)?;

    let state = DaemonState::new(workspace, opts.listen.clone(), Arc::new(auth_mgr), job_mgr);
    Ok((state, token_path, generated_token))
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            () = async {
                if let Some(ref mut s) = sigterm {
                    let _ = s.recv().await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// Starts the `rtkk serve` HTTP/SSE server and blocks until `SIGINT` or `SIGTERM` triggers shutdown.
///
/// # Errors
///
/// Returns [`RtkkError`] if initialization or TCP binding fails.
pub async fn run_daemon(opts: ServeOptions) -> Result<(), RtkkError> {
    let (state, token_path, generated_token) = initialize_daemon_state(&opts)?;
    let listener = TcpListener::bind(&opts.listen)
        .await
        .map_err(|e| RtkkError::Daemon(format!("failed to bind {}: {e}", opts.listen)))?;
    let local_addr = listener
        .local_addr()
        .map_err(|e| RtkkError::Daemon(format!("failed to query local address: {e}")))?;

    tracing::info!(
        listen = %local_addr,
        work_dir = %state.workspace().root().display(),
        token_file = %token_path.display(),
        token_bootstrapped = generated_token,
        "rtkk daemon listening"
    );

    let jobs_for_shutdown = Arc::clone(state.jobs());
    let app = build_app_router(state);
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            wait_for_shutdown_signal().await;
            jobs_for_shutdown.cancel_all_running();
        })
        .await
        .map_err(|e| RtkkError::Daemon(format!("daemon server error: {e}")))
}

/// Executes the `rtkk healthcheck` HTTP probe against `url`.
///
/// # Errors
///
/// Returns [`RtkkError::Daemon`] if the endpoint is unreachable, returns non-`200`, or reports non-`"ok"` status.
pub async fn execute_healthcheck(url: &str, timeout_secs: u64) -> Result<(), RtkkError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs.max(1)))
        .no_proxy()
        .build()
        .map_err(|e| RtkkError::Daemon(format!("failed to build healthcheck client: {e}")))?;

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| RtkkError::Daemon(format!("healthcheck GET {url} failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(RtkkError::Daemon(format!(
            "healthcheck {url} returned status {}",
            resp.status()
        )));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| RtkkError::Daemon(format!("invalid healthcheck JSON from {url}: {e}")))?;

    if body.get("status").and_then(serde_json::Value::as_str) != Some("ok") {
        return Err(RtkkError::Daemon(format!(
            "unhealthy daemon response from {url}: {body}"
        )));
    }
    Ok(())
}
