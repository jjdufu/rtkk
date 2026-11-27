//! `Harbor` v2 project/repository/artifact management and transparent mirror import routes.

use crate::daemon::auth::RequireAuth;
use crate::daemon::jobs::JobConcurrencyClass;
use crate::daemon::routes::artifacts::SpawnedJobResponse;
use crate::daemon::state::{ApiError, DaemonState};
use crate::registry::{
    HarborArtifactInfo, HarborMirrorSyncOutcome, HarborProjectsOverview, HarborRepositoryInfo,
    MirrorExecutionMode, MirrorImportOptions, PublicRegistryMirror, RegistryConfig,
    TransparentMirrorTarget, collect_mirror_project_names, delete_harbor_artifact,
    ensure_harbor_mirror_projects, execute_mirror_images, list_harbor_artifacts,
    list_harbor_projects_reconciled, list_harbor_repositories, parse_mirror_settings_from_vars,
    plan_transparent_mirrors,
};
use crate::workspace::Workspace;
use axum::Json;
use axum::extract::{Path as AxumPath, Query, State};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

/// Response for `GET /api/v1/harbor/projects`.
#[derive(Debug, Serialize)]
pub struct HarborProjectsResponse {
    /// Reconciled `Harbor` project overview.
    #[serde(flatten)]
    pub overview: HarborProjectsOverview,
    /// Configured explicit upstream-to-project mirror mappings.
    pub mirrors: Vec<PublicRegistryMirror>,
}

/// Request body for `POST /api/v1/harbor/projects/sync`.
#[derive(Debug, Default, Deserialize)]
pub struct HarborProjectsSyncRequest {
    /// Optional explicit project names to create; when empty, creates all configured mirror & fallback projects.
    #[serde(default)]
    pub projects: Vec<String>,
    /// Fail on `401`/`403`/`404` when `true` instead of emitting non-fatal warnings.
    #[serde(default)]
    pub strict: bool,
}

/// Query parameters for `GET /api/v1/harbor/projects/{project}/repositories`.
#[derive(Debug, Default, Deserialize)]
pub struct HarborRepoQuery {
    /// Optional substring filter matched against repository name.
    pub q: Option<String>,
}

/// Response for `GET /api/v1/harbor/projects/{project}/repositories`.
#[derive(Debug, Serialize)]
pub struct HarborRepositoriesResponse {
    /// Target `Harbor` project name.
    pub project: String,
    /// Repositories discovered in the project.
    pub repositories: Vec<HarborRepositoryInfo>,
}

/// Query parameters for `GET /api/v1/harbor/projects/{project}/artifacts`.
#[derive(Debug, Deserialize)]
pub struct HarborArtifactListQuery {
    /// Repository path relative to the project (e.g. `"library/nginx"`).
    pub repository: String,
}

/// Response for `GET /api/v1/harbor/projects/{project}/artifacts`.
#[derive(Debug, Serialize)]
pub struct HarborArtifactsResponse {
    /// Target `Harbor` project name.
    pub project: String,
    /// Repository short path.
    pub repository: String,
    /// Artifacts and tags in the repository.
    pub artifacts: Vec<HarborArtifactInfo>,
}

/// Query parameters for `DELETE /api/v1/harbor/projects/{project}/artifacts`.
#[derive(Debug, Deserialize)]
pub struct HarborArtifactDeleteQuery {
    /// Repository path relative to the project.
    pub repository: String,
    /// Tag name or `sha256:...` digest to delete.
    pub reference: String,
}

/// Response for `DELETE /api/v1/harbor/projects/{project}/artifacts`.
#[derive(Debug, Serialize)]
pub struct HarborArtifactDeleteResponse {
    /// Always `true` on successful deletion.
    pub deleted: bool,
    /// Owning `Harbor` project.
    pub project: String,
    /// Repository short path.
    pub repository: String,
    /// Deleted tag or digest reference.
    pub reference: String,
}

/// Request body for `POST /api/v1/harbor/mirror/plan`.
#[derive(Debug, Default, Deserialize)]
pub struct HarborMirrorPlanRequest {
    /// Raw upstream image references (each item may also contain newline-separated lists or `image:` lines).
    #[serde(default)]
    pub images: Vec<String>,
    /// Probe `Harbor` for target project & tag existence when `true` (default: `true`).
    #[serde(default = "default_true")]
    pub check_remote: bool,
}

const fn default_true() -> bool {
    true
}

/// Response for `POST /api/v1/harbor/mirror/plan`.
#[derive(Debug, Serialize)]
pub struct HarborMirrorPlanResponse {
    /// Registry host extracted from `vars.registry.url`.
    pub registry_host: String,
    /// Whether `enable_public_registry_mirrors` is enabled in `inventory.yaml`.
    pub mirrors_enabled: bool,
    /// Whether `public_registry_mirrors_fallback` is enabled in `inventory.yaml`.
    pub allow_fallback: bool,
    /// Configured `_default` catch-all project name (if enabled).
    pub default_fallback_project: Option<String>,
    /// Per-image resolved transparent mapping plan.
    pub plans: Vec<TransparentMirrorTarget>,
}

/// Request body for `POST /api/v1/harbor/mirror/import`.
#[derive(Debug, Default, Deserialize)]
pub struct HarborMirrorImportRequest {
    /// Raw upstream image references to pull and import into transparent `Harbor` mirror paths.
    #[serde(default)]
    pub images: Vec<String>,
    /// Optional target `OCI` platform override (e.g. `"linux/amd64"`).
    pub platform: Option<String>,
    /// Persist downloaded `.tar` archives under `downloads/images/mirror/` when `Some(true)`.
    #[serde(default)]
    pub save_tar: Option<bool>,
    /// Skip `TLS` verification when pulling from upstream source registries when `Some(true)`.
    #[serde(default)]
    pub insecure: Option<bool>,
    /// Overwrite existing tags on `Harbor` when `Some(true)`.
    #[serde(default)]
    pub force: Option<bool>,
    /// Append successfully imported entries to `vars.extra_images` in `inventory.yaml` when `Some(true)`.
    #[serde(default)]
    pub append_extra_images: Option<bool>,
}

fn parse_bool_var(vars: &Value, key: &str, default_val: bool) -> bool {
    match vars.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => true,
            "false" | "0" | "no" => false,
            _ => default_val,
        },
        _ => default_val,
    }
}

async fn load_registry_and_vars(ws: &Workspace) -> Result<(RegistryConfig, Value), ApiError> {
    let inv_raw = tokio::fs::read_to_string(ws.inventory_path())
        .await
        .map_err(|e| ApiError::not_found(format!("Failed to read inventory.yaml: {e}")))?;
    let doc: Value = serde_yml::from_str(&inv_raw)
        .map_err(|e| ApiError::invalid_argument(format!("Invalid inventory.yaml: {e}")))?;
    let vars = doc
        .get("vars")
        .or_else(|| doc.get("all").and_then(|a| a.get("vars")))
        .cloned()
        .ok_or_else(|| ApiError::invalid_argument("Missing vars section in inventory.yaml"))?;
    let reg_val = vars
        .get("registry")
        .cloned()
        .ok_or_else(|| ApiError::invalid_argument("Missing vars.registry in inventory.yaml"))?;
    let mut reg_cfg: RegistryConfig = serde_json::from_value(reg_val)
        .map_err(|e| ApiError::invalid_argument(format!("Invalid registry config: {e}")))?;
    if let Some(ref ca) = reg_cfg.ca_cert_file {
        let expanded = crate::utils::expand_tilde_str(ca);
        if Path::new(&expanded).is_relative() {
            reg_cfg.ca_cert_file = Some(ws.root().join(expanded).to_string_lossy().into_owned());
        } else {
            reg_cfg.ca_cert_file = Some(expanded);
        }
    }
    Ok((reg_cfg, vars))
}

fn extract_registry_host(url: &str) -> String {
    let trimmed = url.trim();
    let no_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    no_scheme.split('/').next().unwrap_or("").to_string()
}

/// Lists `Harbor` projects reconciled against configured transparent mirror mappings (`GET /api/v1/harbor/projects`).
///
/// # Errors
///
/// Returns [`ApiError`] if `inventory.yaml` is invalid or `Harbor` is unreachable.
pub async fn list_projects(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
) -> Result<Json<HarborProjectsResponse>, ApiError> {
    let (reg_cfg, vars) = load_registry_and_vars(state.workspace()).await?;
    let (mirrors, default_proj) = parse_mirror_settings_from_vars(&vars);
    let mirrors_enabled = parse_bool_var(&vars, "enable_public_registry_mirrors", false);
    let allow_fallback = parse_bool_var(&vars, "public_registry_mirrors_fallback", false);
    let overview = list_harbor_projects_reconciled(
        &reg_cfg,
        &mirrors,
        default_proj.as_deref(),
        mirrors_enabled,
        allow_fallback,
    )
    .await
    .map_err(ApiError::upstream_unreachable)?;
    Ok(Json(HarborProjectsResponse { overview, mirrors }))
}

/// Ensures missing `Harbor` mirror projects (or explicit project names) exist (`POST /api/v1/harbor/projects/sync`).
///
/// # Errors
///
/// Returns [`ApiError`] if project names are invalid or `Harbor` API calls fail.
pub async fn sync_projects(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
    Json(req): Json<HarborProjectsSyncRequest>,
) -> Result<Json<HarborMirrorSyncOutcome>, ApiError> {
    let (reg_cfg, vars) = load_registry_and_vars(state.workspace()).await?;
    let projects = if req.projects.is_empty() {
        let (mirrors, default_proj) = parse_mirror_settings_from_vars(&vars);
        collect_mirror_project_names(&mirrors, default_proj.as_deref())
    } else {
        req.projects
    };
    let outcome = ensure_harbor_mirror_projects(&reg_cfg, &projects, req.strict)
        .await
        .map_err(|e| {
            if e.contains("invalid Harbor mirror project name") {
                ApiError::invalid_argument(e)
            } else {
                ApiError::upstream_unreachable(e)
            }
        })?;
    Ok(Json(outcome))
}

/// Lists repositories under a `Harbor` project (`GET /api/v1/harbor/projects/{project}/repositories`).
///
/// # Errors
///
/// Returns [`ApiError`] if `project` is invalid or `Harbor` is unreachable.
pub async fn list_repositories(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
    AxumPath(project): AxumPath<String>,
    Query(query): Query<HarborRepoQuery>,
) -> Result<Json<HarborRepositoriesResponse>, ApiError> {
    let (reg_cfg, _) = load_registry_and_vars(state.workspace()).await?;
    let repositories = list_harbor_repositories(&reg_cfg, &project, query.q.as_deref())
        .await
        .map_err(|e| {
            if e.contains("invalid Harbor") || e.contains("cannot be empty") {
                ApiError::invalid_argument(e)
            } else {
                ApiError::upstream_unreachable(e)
            }
        })?;
    Ok(Json(HarborRepositoriesResponse {
        project,
        repositories,
    }))
}

/// Lists artifacts and tags for a `Harbor` repository (`GET /api/v1/harbor/projects/{project}/artifacts`).
///
/// # Errors
///
/// Returns [`ApiError`] if query parameters are invalid or `Harbor` is unreachable.
pub async fn list_artifacts(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
    AxumPath(project): AxumPath<String>,
    Query(query): Query<HarborArtifactListQuery>,
) -> Result<Json<HarborArtifactsResponse>, ApiError> {
    let (reg_cfg, _) = load_registry_and_vars(state.workspace()).await?;
    let artifacts = list_harbor_artifacts(&reg_cfg, &project, &query.repository)
        .await
        .map_err(|e| {
            if e.contains("invalid Harbor") || e.contains("cannot be empty") {
                ApiError::invalid_argument(e)
            } else {
                ApiError::upstream_unreachable(e)
            }
        })?;
    Ok(Json(HarborArtifactsResponse {
        project,
        repository: query.repository,
        artifacts,
    }))
}

/// Deletes an artifact or tag from a `Harbor` repository (`DELETE /api/v1/harbor/projects/{project}/artifacts`).
///
/// # Errors
///
/// Returns [`ApiError`] if parameters are invalid or `Harbor` rejects deletion.
pub async fn delete_artifact(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
    AxumPath(project): AxumPath<String>,
    Query(query): Query<HarborArtifactDeleteQuery>,
) -> Result<Json<HarborArtifactDeleteResponse>, ApiError> {
    let (reg_cfg, _) = load_registry_and_vars(state.workspace()).await?;
    delete_harbor_artifact(&reg_cfg, &project, &query.repository, &query.reference)
        .await
        .map_err(|e| {
            if e.contains("invalid Harbor") || e.contains("required") {
                ApiError::invalid_argument(e)
            } else {
                ApiError::upstream_unreachable(e)
            }
        })?;
    Ok(Json(HarborArtifactDeleteResponse {
        deleted: true,
        project,
        repository: query.repository,
        reference: query.reference,
    }))
}

/// Resolves raw upstream image names into transparent `Harbor` targets (`POST /api/v1/harbor/mirror/plan`).
///
/// # Errors
///
/// Returns [`ApiError`] if no image references are provided or inventory loading fails.
pub async fn post_mirror_plan(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
    Json(req): Json<HarborMirrorPlanRequest>,
) -> Result<Json<HarborMirrorPlanResponse>, ApiError> {
    let (reg_cfg, vars) = load_registry_and_vars(state.workspace()).await?;
    let (_, default_proj) = parse_mirror_settings_from_vars(&vars);
    let mirrors_enabled = parse_bool_var(&vars, "enable_public_registry_mirrors", false);
    let allow_fallback = parse_bool_var(&vars, "public_registry_mirrors_fallback", false);
    let plans = plan_transparent_mirrors(&reg_cfg, &vars, &req.images, req.check_remote)
        .await
        .map_err(ApiError::invalid_argument)?;
    Ok(Json(HarborMirrorPlanResponse {
        registry_host: extract_registry_host(&reg_cfg.url),
        mirrors_enabled,
        allow_fallback,
        default_fallback_project: default_proj,
        plans,
    }))
}

/// Starts an async transparent mirror pull-and-import job (`POST /api/v1/harbor/mirror/import`).
///
/// # Errors
///
/// Returns [`ApiError`] (`400` or `409`) if input images are invalid or another mutating job is running.
pub async fn post_mirror_import(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
    Json(req): Json<HarborMirrorImportRequest>,
) -> Result<Json<SpawnedJobResponse>, ApiError> {
    let (reg_cfg, vars) = load_registry_and_vars(state.workspace()).await?;
    let preview = plan_transparent_mirrors(&reg_cfg, &vars, &req.images, false)
        .await
        .map_err(ApiError::invalid_argument)?;
    for item in &preview {
        if let Some(ref err) = item.error {
            return Err(ApiError::invalid_argument(format!(
                "Invalid image '{}': {err}",
                item.raw_input
            )));
        }
    }

    let ws = state.workspace().clone();
    let count = preview.len();
    let images = req.images;
    let platform = req
        .platform
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let save_tar = req.save_tar.unwrap_or(false);
    let insecure = req.insecure.unwrap_or(false);
    let force = req.force.unwrap_or(false);
    let append_extra_images = req.append_extra_images.unwrap_or(false);

    let summary = state.jobs().spawn_job(
        "harbor_mirror_import",
        format!("Mirror import {count} image(s) into Harbor"),
        JobConcurrencyClass::Mutating,
        move |_cancel| async move {
            let opts = MirrorImportOptions {
                inventory_path: ws.inventory_path().to_string_lossy().into_owned(),
                images,
                platform,
                output_dir: Some(ws.downloads_images_dir().to_string_lossy().into_owned()),
                insecure: if insecure { Some(true) } else { None },
                mode: MirrorExecutionMode::Execute {
                    force,
                    save_tar,
                    append_extra_images,
                },
            };
            execute_mirror_images(&opts).await.map(|_| ())
        },
    )?;

    Ok(Json(SpawnedJobResponse {
        job_id: summary.id,
        status: "running",
    }))
}
