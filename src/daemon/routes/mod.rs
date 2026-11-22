//! Axum `/api/v1/*` route registration tree (`check_routes.py` verified against `A4`).

/// Offline binary, archive, and OCI/Harbor image sync endpoints (Step 05).
pub mod artifacts;
/// Authentication and session inspection endpoints (Step 02).
pub mod auth;
/// Health probe and workspace overview endpoints (Step 02).
pub mod health;
/// Inventory read/write, validation, and SSH/`become` probe endpoints (Step 04).
pub mod inventory;
/// Asynchronous job listing, cancellation, and SSE stream endpoints (Step 02).
pub mod jobs;
/// Playbook outline and async cluster orchestration endpoints (Step 06).
pub mod operations;
/// X.509 certificate lifecycle and `rtkk_bak/` archive endpoints (Step 06).
pub mod pki;
/// Live `.gtmpl` & `images.yaml` rendering preview endpoint (Step 04).
pub mod templates;

use crate::daemon::state::DaemonState;
use axum::Router;
use axum::routing::{get, post, put};

/// Builds the `/api/v1/*` [`Router`] containing all 27 mounted Daemon endpoints.
pub fn build_api_router() -> Router<DaemonState> {
    Router::new()
        .route("/api/v1/health", get(health::get_health))
        .route("/api/v1/auth/login", post(auth::post_login))
        .route("/api/v1/auth/logout", post(auth::post_logout))
        .route("/api/v1/auth/me", get(auth::get_me))
        .route("/api/v1/system/overview", get(health::get_system_overview))
        .route("/api/v1/jobs", get(jobs::list_jobs))
        .route("/api/v1/jobs/{id}", get(jobs::get_job))
        .route("/api/v1/jobs/{id}/cancel", post(jobs::cancel_job))
        .route("/api/v1/jobs/{id}/events", get(jobs::stream_job_events))
        .route("/api/v1/inventory", get(inventory::get_inventory))
        .route("/api/v1/inventory", put(inventory::put_inventory))
        .route(
            "/api/v1/inventory/validate",
            post(inventory::validate_inventory),
        )
        .route(
            "/api/v1/inventory/probe",
            post(inventory::probe_inventory_hosts),
        )
        .route(
            "/api/v1/templates/render",
            post(templates::render_templates),
        )
        .route(
            "/api/v1/artifacts/status",
            get(artifacts::get_artifacts_status),
        )
        .route(
            "/api/v1/artifacts/download",
            post(artifacts::post_artifacts_download),
        )
        .route("/api/v1/registry/ping", post(artifacts::post_registry_ping))
        .route("/api/v1/registry/pull", post(artifacts::post_registry_pull))
        .route("/api/v1/registry/push", post(artifacts::post_registry_push))
        .route(
            "/api/v1/operations/prep-hosts",
            post(operations::post_prep_hosts),
        )
        .route(
            "/api/v1/operations/run",
            post(operations::post_run_playbook),
        )
        .route(
            "/api/v1/playbooks/{name}/outline",
            get(operations::get_playbook_outline),
        )
        .route("/api/v1/pki/certificates", get(pki::list_certificates))
        .route("/api/v1/pki/generate", post(pki::post_generate_certificate))
        .route("/api/v1/pki/verify", post(pki::post_verify_certificate))
        .route("/api/v1/backups", get(pki::list_backups))
        .route(
            "/api/v1/backups/archive",
            get(pki::download_backups_archive),
        )
}
