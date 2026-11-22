//! Playbook outline inspection and async orchestration routes (`prep-hosts`, `run`, `outline` — Step 06).

use crate::daemon::auth::RequireAuth;
use crate::daemon::jobs::JobConcurrencyClass;
use crate::daemon::routes::artifacts::SpawnedJobResponse;
use crate::daemon::state::{ApiError, DaemonState};
use crate::playbook::{
    ExecutionEngine, PlaybookOutline, compile_playbook, extract_playbook_outline,
};
use crate::workspace::Workspace;
use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Request body for `POST /api/v1/operations/prep-hosts`.
#[derive(Debug, Default, Deserialize)]
pub struct PrepHostsRequest {
    /// Continue updating reachable nodes when some nodes fail hostname lookup.
    #[serde(default)]
    pub best_effort: bool,
}

/// Request body for `POST /api/v1/operations/run`.
#[derive(Debug, Deserialize)]
pub struct RunPlaybookRequest {
    /// Whitelisted built-in playbook stem (`"deploy"`, `"cleanup"`, `"download"`).
    pub playbook: String,
    /// Optional tag filter list.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Optional host concurrency override (`>= 1`).
    pub forks: Option<usize>,
    /// Optional temporary variable overrides (`-s` equivalent).
    #[serde(default)]
    pub extra_vars: HashMap<String, Value>,
}

fn resolve_builtin_playbook_path(ws: &Workspace, name: &str) -> Result<PathBuf, ApiError> {
    let file_name = match name.trim() {
        "deploy" => "deploy.yaml",
        "cleanup" => "cleanup.yaml",
        "download" => "download.yaml",
        other => {
            return Err(ApiError::not_found(format!(
                "Unsupported playbook '{other}' (expected deploy, cleanup, or download)"
            )));
        }
    };
    let path = ws.configs_dir().join(file_name);
    if !path.is_file() {
        return Err(ApiError::not_found(format!(
            "Playbook file {} not found in workspace",
            path.display()
        )));
    }
    Ok(path)
}

fn value_to_override_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        _ => serde_json::to_string(v).unwrap_or_default(),
    }
}

/// Parses and returns the static Play/Task/Tag outline of a built-in playbook (`GET /api/v1/playbooks/{name}/outline`).
///
/// # Errors
///
/// Returns [`ApiError`] (`404` or `400`) if `name` is unknown or compilation fails.
pub async fn get_playbook_outline(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
    Path(name): Path<String>,
) -> Result<Json<PlaybookOutline>, ApiError> {
    let pb_path = resolve_builtin_playbook_path(state.workspace(), &name)?;
    let logical = name.trim().to_string();
    let outline = tokio::task::spawn_blocking(move || extract_playbook_outline(&logical, &pb_path))
        .await
        .map_err(|e| ApiError::internal(format!("Outline task join error: {e}")))?
        .map_err(ApiError::invalid_argument)?;
    Ok(Json(outline))
}

/// Starts an async `prep hosts` cluster `/etc/hosts` synchronization job (`POST /api/v1/operations/prep-hosts`).
///
/// # Errors
///
/// Returns [`ApiError`] (`409`) if another mutating job is already running.
pub async fn post_prep_hosts(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
    Json(req): Json<PrepHostsRequest>,
) -> Result<Json<SpawnedJobResponse>, ApiError> {
    let ws = state.workspace().clone();
    let best_effort = req.best_effort;
    let summary = state.jobs().spawn_job(
        "prep_hosts",
        "Synchronize cluster /etc/hosts",
        JobConcurrencyClass::Mutating,
        move |_cancel| async move {
            let inv_str = ws.inventory_path().to_string_lossy().into_owned();
            let loaded = crate::cli::load_inventory(&inv_str)
                .await
                .map_err(|e| e.to_string())?;
            crate::prep::execute_prep_hosts(
                &inv_str,
                &loaded.flat,
                &loaded.target_configs,
                best_effort,
            )
            .await
        },
    )?;
    Ok(Json(SpawnedJobResponse {
        job_id: summary.id,
        status: "running",
    }))
}

/// Starts an async built-in playbook execution job (`POST /api/v1/operations/run`).
///
/// Pure `deploy --tags check` runs as [`JobConcurrencyClass::Readonly`]; all other runs
/// acquire the workspace single-flight [`JobConcurrencyClass::Mutating`] lock.
///
/// # Errors
///
/// Returns [`ApiError`] (`400`, `404`, or `409`) on invalid parameters, unknown playbook, or lock conflict.
pub async fn post_run_playbook(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
    Json(req): Json<RunPlaybookRequest>,
) -> Result<Json<SpawnedJobResponse>, ApiError> {
    let pb_path = resolve_builtin_playbook_path(state.workspace(), &req.playbook)?;
    if let Some(f) = req.forks
        && f == 0
    {
        return Err(ApiError::invalid_argument("forks must be >= 1"));
    }
    let clean_tags: Vec<String> = req
        .tags
        .iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();

    let is_readonly_check = req.playbook.trim() == "deploy"
        && clean_tags.len() == 1
        && clean_tags.first().map(String::as_str) == Some("check");
    let concurrency_class = if is_readonly_check {
        JobConcurrencyClass::Readonly
    } else {
        JobConcurrencyClass::Mutating
    };

    let overrides: Vec<(String, String)> = req
        .extra_vars
        .iter()
        .map(|(k, v)| (k.clone(), value_to_override_string(v)))
        .collect();
    let ws = state.workspace().clone();
    let forks_opt = req.forks;
    let title = if clean_tags.is_empty() {
        format!("Run {}.yaml", req.playbook.trim())
    } else {
        format!(
            "Run {}.yaml [{}]",
            req.playbook.trim(),
            clean_tags.join(",")
        )
    };

    let summary = state.jobs().spawn_job(
        format!("playbook_{}", req.playbook.trim()),
        title,
        concurrency_class,
        move |_cancel| async move {
            let inv_str = ws.inventory_path().to_string_lossy().into_owned();
            let loaded = crate::cli::load_inventory_with_overrides(&inv_str, &overrides)
                .await
                .map_err(|e| e.to_string())?;
            let compiled = tokio::task::spawn_blocking(move || compile_playbook(&pb_path))
                .await
                .map_err(|e| e.to_string())??;
            let forks_val = forks_opt.unwrap_or(20).max(1);
            let mut engine = ExecutionEngine::new(
                loaded.flat,
                loaded.target_configs,
                forks_val,
                loaded.inventory_dir,
            );
            engine.tag_filter = clean_tags;
            engine.workspace = Some(ws);
            Arc::new(engine).run_playbook(&compiled).await
        },
    )?;

    Ok(Json(SpawnedJobResponse {
        job_id: summary.id,
        status: "running",
    }))
}
