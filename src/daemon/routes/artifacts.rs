//! Offline artifact status, async prefetch download, and OCI/Harbor registry sync routes (Step 05).

use crate::daemon::auth::RequireAuth;
use crate::daemon::jobs::JobConcurrencyClass;
use crate::daemon::mask::restore_masked_secrets;
use crate::daemon::state::{ApiError, DaemonState};
use crate::playbook::{ExecutionEngine, compile_playbook};
use crate::registry::{
    PullImagesOptions, PushImagesBatchOptions, RegistryConfig, RegistryPingReport,
    execute_pull_images_filtered, execute_push_images_filtered, inspect_local_image_tar,
    load_rendered_image_specs_from_doc, ping_registry_endpoint,
};
use crate::workspace::Workspace;
use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

/// Aggregated offline readiness counters for `GET /api/v1/artifacts/status`.
#[derive(Debug, Default, Serialize)]
pub struct ArtifactsSummary {
    /// Ready binary count in `downloads/bin`.
    pub binaries_ready: usize,
    /// Total expected binary count.
    pub binaries_total: usize,
    /// Ready archive count in `downloads/tar` (excluding optional debs).
    pub archives_ready: usize,
    /// Total expected core archive count.
    pub archives_total: usize,
    /// Whether `downloads/tar/debian13-deps.tar.gz` is present.
    pub debs_bundle_ready: bool,
    /// Ready verified `.tar` count in `downloads/images`.
    pub images_ready: usize,
    /// Expected image count from `configs/images.yaml`.
    pub images_total: usize,
    /// Total byte size across all ready artifacts.
    pub total_bytes: u64,
}

/// Single binary or archive checklist item.
#[derive(Debug, Serialize)]
pub struct FileArtifactItem {
    /// Logical component name.
    pub name: String,
    /// Configured version from `inventory.yaml`.
    pub version: String,
    /// Workspace-relative file path.
    pub rel_path: String,
    /// Whether the file exists on disk.
    pub exists: bool,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Last modification UTC timestamp in milliseconds.
    pub modified_ms: Option<u64>,
    /// Whether this item is optional (e.g. offline `.deb` bundle).
    pub optional: bool,
}

/// Single container image `.tar` checklist item.
#[derive(Debug, Serialize)]
pub struct ImageArtifactItem {
    /// Logical image stem (`<name>.tar`).
    pub name: String,
    /// Upstream source image reference.
    pub source: String,
    /// Target private registry reference.
    pub target: String,
    /// Workspace-relative `.tar` path.
    pub rel_path: String,
    /// Whether the `.tar` file exists on disk.
    pub exists: bool,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Whether `manifest.json` `RepoTags` matches `target`.
    pub repo_tags_matched: bool,
    /// Whether `rtkk-media.json` is intact and matches `source`.
    pub has_media_manifest: bool,
}

/// Response returned by `GET /api/v1/artifacts/status`.
#[derive(Debug, Serialize)]
pub struct ArtifactsStatusResponse {
    /// High-level readiness counters.
    pub summary: ArtifactsSummary,
    /// Expected binaries under `downloads/bin`.
    pub binaries: Vec<FileArtifactItem>,
    /// Expected archives under `downloads/tar`.
    pub archives: Vec<FileArtifactItem>,
    /// Expected container image tarballs under `downloads/images`.
    pub images: Vec<ImageArtifactItem>,
}

/// Async job launch response shared by download/pull/push endpoints.
#[derive(Debug, Serialize)]
pub struct SpawnedJobResponse {
    /// Spawned job identifier.
    pub job_id: String,
    /// Initial job status (`"running"`).
    pub status: &'static str,
}

/// Request body for `POST /api/v1/artifacts/download`.
#[derive(Debug, Default, Deserialize)]
pub struct ArtifactDownloadRequest {
    /// Optional tag scopes (`"binaries"`, `"archives"`, `"charts"`, `"debs"`, `"images"`).
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Overwrite existing artifacts when `true`.
    #[serde(default)]
    pub force: bool,
}

/// Request body for `POST /api/v1/registry/ping`.
#[derive(Debug, Default, Deserialize)]
pub struct RegistryPingRequest {
    /// Optional candidate registry object overriding `inventory.yaml` `vars.registry`.
    pub registry_override: Option<Value>,
}

/// Request body for `POST /api/v1/registry/pull`.
#[derive(Debug, Default, Deserialize)]
pub struct RegistryPullRequest {
    /// Optional subset of `ImageSpec.name` entries (empty pulls all).
    #[serde(default)]
    pub image_names: Vec<String>,
    /// Force re-pulling even when local `.tar` matches.
    #[serde(default)]
    pub force: bool,
    /// Skip TLS verification on upstream source registries.
    #[serde(default)]
    pub insecure: bool,
}

/// Request body for `POST /api/v1/registry/push`.
#[derive(Debug, Default, Deserialize)]
pub struct RegistryPushRequest {
    /// Optional subset of `ImageSpec.name` entries (empty pushes all).
    #[serde(default)]
    pub image_names: Vec<String>,
}

fn inspect_file(path: &Path) -> (bool, u64, Option<u64>) {
    let Ok(meta) = std::fs::metadata(path) else {
        return (false, 0, None);
    };
    if !meta.is_file() {
        return (false, 0, None);
    }
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .and_then(|d| u64::try_from(d.as_millis()).ok());
    (true, meta.len(), mtime)
}

fn var_str(vars: Option<&Value>, key: &str, fallback: &str) -> String {
    vars.and_then(|v| v.get(key))
        .and_then(Value::as_str)
        .unwrap_or(fallback)
        .to_string()
}

fn scan_binaries_and_archives(
    ws: &Workspace,
    vars: Option<&Value>,
    summary: &mut ArtifactsSummary,
) -> (Vec<FileArtifactItem>, Vec<FileArtifactItem>) {
    let kube_v = var_str(vars, "kube_version", "v1.37.2");
    let runc_v = var_str(vars, "runc_version", "v1.5.3");
    let helm_v = var_str(vars, "helm_version", "v4.3.2");
    let containerd_v = var_str(vars, "containerd_version", "2.4.2");
    let etcd_v = var_str(vars, "etcd_version", "v3.7.4");
    let cni_v = var_str(vars, "cni_version", "v1.10.0");
    let cilium_v = var_str(vars, "cilium_version", "v1.20.5");

    let bin_defs = [
        ("kubeadm", kube_v.as_str(), "downloads/bin/kubeadm"),
        ("kubelet", kube_v.as_str(), "downloads/bin/kubelet"),
        ("kubectl", kube_v.as_str(), "downloads/bin/kubectl"),
        ("runc", runc_v.as_str(), "downloads/bin/runc"),
        ("helm", helm_v.as_str(), "downloads/bin/helm"),
    ];
    summary.binaries_total = bin_defs.len();
    summary.archives_total = 5;

    let mut binaries = Vec::with_capacity(bin_defs.len());
    for (name, ver, rel) in bin_defs {
        let (exists, size_bytes, modified_ms) = inspect_file(&ws.root().join(rel));
        if exists {
            summary.binaries_ready = summary.binaries_ready.saturating_add(1);
            summary.total_bytes = summary.total_bytes.saturating_add(size_bytes);
        }
        binaries.push(FileArtifactItem {
            name: name.to_string(),
            version: ver.to_string(),
            rel_path: rel.to_string(),
            exists,
            size_bytes,
            modified_ms,
            optional: false,
        });
    }

    let arc_defs = [
        (
            "containerd.tar.gz",
            containerd_v.as_str(),
            "downloads/tar/containerd.tar.gz",
            false,
        ),
        (
            "etcd.tar.gz",
            etcd_v.as_str(),
            "downloads/tar/etcd.tar.gz",
            false,
        ),
        (
            "cni-plugins.tgz",
            cni_v.as_str(),
            "downloads/tar/cni-plugins.tgz",
            false,
        ),
        (
            "helm.tar.gz",
            helm_v.as_str(),
            "downloads/tar/helm.tar.gz",
            false,
        ),
        (
            "cilium-chart.tgz",
            cilium_v.as_str(),
            "downloads/tar/cilium-chart.tgz",
            false,
        ),
        (
            "debian13-deps.tar.gz",
            "debian13",
            "downloads/tar/debian13-deps.tar.gz",
            true,
        ),
    ];
    let mut archives = Vec::with_capacity(arc_defs.len());
    for (name, ver, rel, optional) in arc_defs {
        let (exists, size_bytes, modified_ms) = inspect_file(&ws.root().join(rel));
        if exists {
            summary.total_bytes = summary.total_bytes.saturating_add(size_bytes);
            if optional {
                summary.debs_bundle_ready = true;
            } else {
                summary.archives_ready = summary.archives_ready.saturating_add(1);
            }
        }
        archives.push(FileArtifactItem {
            name: name.to_string(),
            version: ver.to_string(),
            rel_path: rel.to_string(),
            exists,
            size_bytes,
            modified_ms,
            optional,
        });
    }
    (binaries, archives)
}

fn collect_artifacts_status(ws: &Workspace) -> Result<ArtifactsStatusResponse, ApiError> {
    let inv_raw = std::fs::read_to_string(ws.inventory_path())
        .map_err(|e| ApiError::not_found(format!("Failed to read inventory.yaml: {e}")))?;
    let mut doc: Value = serde_yml::from_str(&inv_raw)
        .map_err(|e| ApiError::invalid_argument(format!("Invalid inventory.yaml: {e}")))?;

    // Inject dynamic vars so images.yaml templates using groups/addons render accurately
    if let Ok(inv) = serde_yml::from_str::<crate::inventory::Inventory>(&inv_raw)
        && let Ok(mut flat) = crate::inventory::resolve_inventory(&inv)
    {
        crate::playbook::inject_dynamic_inventory_vars(&flat.groups, &mut flat.host_vars);
        crate::playbook::inject_runtime_vars(
            &mut flat.host_vars,
            &ws.inventory_path().to_string_lossy(),
        );
        if let Some(first_host) = flat.groups.get("all").and_then(|g| g.first())
            && let Some(hv) = flat.host_vars.get(first_host)
            && let Ok(hv_val) = serde_json::to_value(hv)
            && let Some(obj) = doc.as_object_mut()
        {
            obj.insert("vars".to_string(), hv_val);
        }
    }

    let mut summary = ArtifactsSummary::default();
    let (binaries, archives) = scan_binaries_and_archives(ws, doc.get("vars"), &mut summary);

    let specs = load_rendered_image_specs_from_doc(&doc, &ws.images_path())
        .map_err(ApiError::template_render)?;
    summary.images_total = specs.len();
    let img_dir_str = ws.downloads_images_dir().to_string_lossy().into_owned();
    let mut images = Vec::with_capacity(specs.len());
    for spec in specs {
        let st = inspect_local_image_tar(&img_dir_str, &spec);
        if st.exists {
            summary.total_bytes = summary.total_bytes.saturating_add(st.size_bytes);
            if st.repo_tags_matched {
                summary.images_ready = summary.images_ready.saturating_add(1);
            }
        }
        images.push(ImageArtifactItem {
            rel_path: format!("downloads/images/{}.tar", spec.name),
            name: spec.name,
            source: spec.source,
            target: spec.target,
            exists: st.exists,
            size_bytes: st.size_bytes,
            repo_tags_matched: st.repo_tags_matched,
            has_media_manifest: st.has_media_manifest,
        });
    }

    Ok(ArtifactsStatusResponse {
        summary,
        binaries,
        archives,
        images,
    })
}

/// Scans `downloads/{bin,tar,images}` against `inventory.yaml` and `images.yaml` (`GET /api/v1/artifacts/status`).
///
/// # Errors
///
/// Returns [`ApiError`] if reading `inventory.yaml` or rendering `images.yaml` fails.
pub async fn get_artifacts_status(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
) -> Result<Json<ArtifactsStatusResponse>, ApiError> {
    let ws = state.workspace().clone();
    let res = tokio::task::spawn_blocking(move || collect_artifacts_status(&ws))
        .await
        .map_err(|e| ApiError::internal(format!("Artifacts status join error: {e}")))??;
    Ok(Json(res))
}

/// Starts an async `configs/download.yaml` prefetch job (`POST /api/v1/artifacts/download`).
///
/// # Errors
///
/// Returns [`ApiError`] (`400` or `409`) if scopes are invalid or another mutating job is running.
pub async fn post_artifacts_download(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
    Json(req): Json<ArtifactDownloadRequest>,
) -> Result<Json<SpawnedJobResponse>, ApiError> {
    for scope in &req.scopes {
        if !crate::utils::is_safe_module_name(scope) {
            return Err(ApiError::invalid_argument(format!(
                "Invalid download scope tag: '{scope}'"
            )));
        }
    }
    let ws = state.workspace().clone();
    let tags = req.scopes.clone();
    let force = req.force;
    let summary = state.jobs().spawn_job(
        "artifacts_download",
        "Prefetch offline binaries & archives",
        JobConcurrencyClass::Mutating,
        move |_cancel| async move {
            let inv_str = ws.inventory_path().to_string_lossy().into_owned();
            let overrides = if force {
                vec![("force_download".to_string(), "true".to_string())]
            } else {
                Vec::new()
            };
            let loaded = crate::cli::load_inventory_with_overrides(&inv_str, &overrides)
                .await
                .map_err(|e| e.to_string())?;
            let dl_path = ws.configs_dir().join("download.yaml");
            let pb = tokio::task::spawn_blocking(move || compile_playbook(&dl_path))
                .await
                .map_err(|e| e.to_string())??;
            let mut engine =
                ExecutionEngine::new(loaded.flat, loaded.target_configs, 8, loaded.inventory_dir);
            engine.tag_filter = tags;
            engine.workspace = Some(ws);
            Arc::new(engine).run_playbook(&pb).await
        },
    )?;
    Ok(Json(SpawnedJobResponse {
        job_id: summary.id,
        status: "running",
    }))
}

/// Tests target private registry `/v2/` connectivity, Basic/Bearer auth, and Harbor project status.
///
/// # Errors
///
/// Returns [`ApiError`] if reading or parsing registry configuration fails.
pub async fn post_registry_ping(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
    Json(req): Json<RegistryPingRequest>,
) -> Result<Json<RegistryPingReport>, ApiError> {
    let inv_raw = tokio::fs::read_to_string(state.workspace().inventory_path())
        .await
        .map_err(|e| ApiError::not_found(format!("Failed to read inventory.yaml: {e}")))?;
    let disk_doc: Value = serde_yml::from_str(&inv_raw)
        .map_err(|e| ApiError::invalid_argument(format!("Invalid inventory.yaml: {e}")))?;

    let reg_val = if let Some(override_reg) = req.registry_override {
        let mut wrapper = serde_json::json!({ "vars": { "registry": override_reg } });
        restore_masked_secrets(&mut wrapper, &disk_doc);
        wrapper
            .get("vars")
            .and_then(|v| v.get("registry"))
            .cloned()
            .unwrap_or(Value::Null)
    } else {
        disk_doc
            .get("vars")
            .and_then(|v| v.get("registry"))
            .or_else(|| {
                disk_doc
                    .get("all")
                    .and_then(|a| a.get("vars"))
                    .and_then(|v| v.get("registry"))
            })
            .cloned()
            .ok_or_else(|| ApiError::invalid_argument("Missing vars.registry in inventory.yaml"))?
    };

    let mut reg_cfg: RegistryConfig = serde_json::from_value(reg_val)
        .map_err(|e| ApiError::invalid_argument(format!("Invalid registry config: {e}")))?;
    if let Some(ref ca) = reg_cfg.ca_cert_file
        && Path::new(ca).is_relative()
    {
        reg_cfg.ca_cert_file = Some(
            state
                .workspace()
                .root()
                .join(ca)
                .to_string_lossy()
                .into_owned(),
        );
    }

    let report = ping_registry_endpoint(&reg_cfg).await;
    Ok(Json(report))
}

/// Starts an async OCI image pull job (`POST /api/v1/registry/pull`).
///
/// # Errors
///
/// Returns [`ApiError`] (`400` or `409`) on invalid filter names or active mutating job contention.
pub async fn post_registry_pull(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
    Json(req): Json<RegistryPullRequest>,
) -> Result<Json<SpawnedJobResponse>, ApiError> {
    for n in &req.image_names {
        if !crate::utils::is_safe_module_name(n) {
            return Err(ApiError::invalid_argument(format!(
                "Invalid image name: '{n}'"
            )));
        }
    }
    let ws = state.workspace().clone();
    let names = req.image_names;
    let force = req.force;
    let insecure = req.insecure;
    let summary = state.jobs().spawn_job(
        "registry_pull",
        "Pull OCI container images",
        JobConcurrencyClass::Mutating,
        move |_cancel| async move {
            let opts = PullImagesOptions {
                inventory_path: ws.inventory_path().to_string_lossy().into_owned(),
                images_file: ws.images_path().to_string_lossy().into_owned(),
                output_dir: Some(ws.downloads_images_dir().to_string_lossy().into_owned()),
                platform: None,
                force,
                insecure: if insecure { Some(true) } else { None },
            };
            execute_pull_images_filtered(&opts, &names).await
        },
    )?;
    Ok(Json(SpawnedJobResponse {
        job_id: summary.id,
        status: "running",
    }))
}

/// Starts an async private registry batch push job (`POST /api/v1/registry/push`).
///
/// # Errors
///
/// Returns [`ApiError`] (`400` or `409`) on invalid filter names or active mutating job contention.
pub async fn post_registry_push(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
    Json(req): Json<RegistryPushRequest>,
) -> Result<Json<SpawnedJobResponse>, ApiError> {
    for n in &req.image_names {
        if !crate::utils::is_safe_module_name(n) {
            return Err(ApiError::invalid_argument(format!(
                "Invalid image name: '{n}'"
            )));
        }
    }
    let ws = state.workspace().clone();
    let names = req.image_names;
    let summary = state.jobs().spawn_job(
        "registry_push",
        "Push OCI container images to private registry",
        JobConcurrencyClass::Mutating,
        move |_cancel| async move {
            let opts = PushImagesBatchOptions {
                inventory_path: ws.inventory_path().to_string_lossy().into_owned(),
                images_file: ws.images_path().to_string_lossy().into_owned(),
                output_dir: Some(ws.downloads_images_dir().to_string_lossy().into_owned()),
            };
            execute_push_images_filtered(&opts, &names).await
        },
    )?;
    Ok(Json(SpawnedJobResponse {
        job_id: summary.id,
        status: "running",
    }))
}
