//! Shared Daemon application state, `X-Request-Id` propagation, and structured `ApiError` responses.

use crate::daemon::auth::AuthManager;
use crate::daemon::jobs::JobManager;
use crate::workspace::Workspace;
use axum::Json;
use axum::body::Body;
use axum::http::{HeaderValue, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use std::sync::Arc;
use std::time::Instant;

tokio::task_local! {
    static CURRENT_REQUEST_ID: String;
}

/// Generates a fresh 16-hex-character `req_<hex>` correlation identifier.
#[must_use]
pub fn generate_request_id() -> String {
    let raw: u64 = rand::random();
    format!("req_{raw:016x}")
}

/// Returns the active `request_id` bound to the current HTTP request task, or generates one.
#[must_use]
pub fn active_request_id() -> String {
    CURRENT_REQUEST_ID
        .try_with(Clone::clone)
        .unwrap_or_else(|_| generate_request_id())
}

/// Axum middleware assigning `req_<hex>` to every incoming request and setting `X-Request-Id`.
pub async fn request_id_middleware(req: Request<Body>, next: Next) -> Response {
    let req_id = generate_request_id();
    let id_for_header = req_id.clone();
    let mut response = CURRENT_REQUEST_ID.scope(req_id, next.run(req)).await;
    if let Ok(val) = HeaderValue::from_str(&id_for_header) {
        response.headers_mut().insert("x-request-id", val);
    }
    response
}

/// Shared reference-counted state injected into all Daemon route handlers.
#[derive(Debug, Clone)]
pub struct DaemonState {
    inner: Arc<DaemonStateInner>,
}

#[derive(Debug)]
struct DaemonStateInner {
    workspace: Workspace,
    bind_addr: String,
    started_at: Instant,
    auth: Arc<AuthManager>,
    jobs: Arc<JobManager>,
}

impl DaemonState {
    /// Creates a new [`DaemonState`] from initialized subsystems.
    #[must_use]
    pub fn new(
        workspace: Workspace,
        bind_addr: String,
        auth: Arc<AuthManager>,
        jobs: Arc<JobManager>,
    ) -> Self {
        Self {
            inner: Arc::new(DaemonStateInner {
                workspace,
                bind_addr,
                started_at: Instant::now(),
                auth,
                jobs,
            }),
        }
    }

    /// Returns the active [`Workspace`].
    #[must_use]
    pub fn workspace(&self) -> &Workspace {
        &self.inner.workspace
    }

    /// Returns the configured listen address string.
    #[must_use]
    pub fn bind_addr(&self) -> &str {
        &self.inner.bind_addr
    }

    /// Returns daemon uptime in whole seconds.
    #[must_use]
    pub fn uptime_secs(&self) -> u64 {
        self.inner.started_at.elapsed().as_secs()
    }

    /// Returns the [`AuthManager`].
    #[must_use]
    pub fn auth(&self) -> &Arc<AuthManager> {
        &self.inner.auth
    }

    /// Returns the [`JobManager`].
    #[must_use]
    pub fn jobs(&self) -> &Arc<JobManager> {
        &self.inner.jobs
    }
}

/// Canonical three-field JSON error envelope for `/api/*` endpoints.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct ApiErrorBody {
    /// Machine-readable error code (`A1` `ApiErrorCode`).
    pub code: String,
    /// Human-readable error summary.
    pub message: String,
    /// Per-request trace identifier (`req_<hex>`).
    pub request_id: String,
}

/// Structured HTTP error returned by Daemon extractors and route handlers.
#[derive(Debug, Clone)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    /// Constructs a `400 INVALID_ARGUMENT` error.
    #[must_use]
    pub fn invalid_argument(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "INVALID_ARGUMENT",
            message: msg.into(),
        }
    }

    /// Constructs a `401 UNAUTHORIZED` error.
    #[must_use]
    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "UNAUTHORIZED",
            message: msg.into(),
        }
    }

    /// Constructs a `404 NOT_FOUND` error.
    #[must_use]
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "NOT_FOUND",
            message: msg.into(),
        }
    }

    /// Constructs a `409 CONFLICT_ACTIVE_JOB` error when a mutating job is already active.
    #[must_use]
    pub fn conflict_active_job(active_job_id: &str) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: "CONFLICT_ACTIVE_JOB",
            message: format!("Another mutating cluster job ({active_job_id}) is currently running"),
        }
    }

    /// Constructs a `422 TEMPLATE_RENDER_ERROR` error.
    #[must_use]
    pub fn template_render(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: "TEMPLATE_RENDER_ERROR",
            message: msg.into(),
        }
    }

    /// Constructs a `429 AUTH_RATE_LIMITED` error.
    #[must_use]
    pub fn rate_limited(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            code: "AUTH_RATE_LIMITED",
            message: msg.into(),
        }
    }

    /// Constructs a `500 INTERNAL_ERROR` error.
    #[must_use]
    pub fn internal(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "INTERNAL_ERROR",
            message: msg.into(),
        }
    }

    /// Constructs a `502 UPSTREAM_UNREACHABLE` error.
    #[must_use]
    pub fn upstream_unreachable(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            code: "UPSTREAM_UNREACHABLE",
            message: msg.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let req_id = active_request_id();
        let body = ApiErrorBody {
            code: self.code.to_string(),
            message: self.message,
            request_id: req_id.clone(),
        };
        let mut res = (self.status, Json(body)).into_response();
        if let Ok(val) = HeaderValue::from_str(&req_id) {
            res.headers_mut().insert("x-request-id", val);
        }
        res
    }
}
