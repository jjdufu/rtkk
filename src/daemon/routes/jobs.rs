//! Asynchronous job listing, detail lookup, cooperative cancellation, and SSE streaming routes.

use crate::daemon::auth::RequireAuth;
use crate::daemon::jobs::{JobDetail, JobStatus, JobSummary};
use crate::daemon::sse::build_job_sse_response;
use crate::daemon::state::{ApiError, DaemonState};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use serde::{Deserialize, Serialize};

/// Pagination query parameters for `GET /api/v1/jobs`.
#[derive(Debug, Default, Deserialize)]
pub struct ListJobsQuery {
    /// Maximum items to return (`1..=100`, default `20`).
    pub limit: Option<usize>,
    /// Optional pagination cursor (`job_id` of last item from previous page).
    pub cursor: Option<String>,
}

/// Paginated response returned by `GET /api/v1/jobs`.
#[derive(Debug, Serialize)]
pub struct ListJobsResponse {
    /// Ordered job summaries (newest first).
    pub items: Vec<JobSummary>,
    /// Next page cursor if additional older jobs exist.
    pub next_cursor: Option<String>,
}

/// Query parameters for `GET /api/v1/jobs/{id}/events`.
#[derive(Debug, Default, Deserialize)]
pub struct JobEventsQuery {
    /// Replay events strictly greater than `after_seq` (defaults to `0`).
    pub after_seq: Option<u64>,
}

/// Response returned by `POST /api/v1/jobs/{id}/cancel`.
#[derive(Debug, Serialize)]
pub struct CancelJobResponse {
    /// Cancelled job ID.
    pub id: String,
    /// Updated job status.
    pub status: JobStatus,
}

/// Lists historical and running jobs (`GET /api/v1/jobs`).
pub async fn list_jobs(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
    Query(query): Query<ListJobsQuery>,
) -> Json<ListJobsResponse> {
    let limit = query.limit.unwrap_or(20);
    let (items, next_cursor) = state.jobs().list_jobs(limit, query.cursor.as_deref());
    Json(ListJobsResponse { items, next_cursor })
}

/// Fetches detailed metadata and per-host counters for a single job (`GET /api/v1/jobs/{id}`).
///
/// # Errors
///
/// Returns [`ApiError::not_found`] if `id` does not exist.
pub async fn get_job(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
    Path(id): Path<String>,
) -> Result<Json<JobDetail>, ApiError> {
    let detail = state
        .jobs()
        .get_job(&id)
        .ok_or_else(|| ApiError::not_found(format!("Job '{id}' not found")))?;
    Ok(Json(detail))
}

/// Cancels an active job (`POST /api/v1/jobs/{id}/cancel`).
///
/// # Errors
///
/// Returns [`ApiError::not_found`] if `id` does not exist.
pub async fn cancel_job(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
    Path(id): Path<String>,
) -> Result<Json<CancelJobResponse>, ApiError> {
    let status = state.jobs().cancel_job(&id)?;
    Ok(Json(CancelJobResponse { id, status }))
}

/// Streams historical JSONL events and live SSE increments (`GET /api/v1/jobs/{id}/events`).
///
/// # Errors
///
/// Returns [`ApiError::not_found`] if `id` does not exist.
pub async fn stream_job_events(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(query): Query<JobEventsQuery>,
) -> Result<Response, ApiError> {
    let header_seq = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok());
    let after_seq = query.after_seq.or(header_seq).unwrap_or(0);
    let (detail, jsonl_path, rx) = state.jobs().subscribe_job(&id)?;
    Ok(build_job_sse_response(detail, jsonl_path, rx, after_seq))
}
