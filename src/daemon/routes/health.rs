//! Health probe (`GET /api/v1/health`) and workspace overview (`GET /api/v1/system/overview`).

use crate::daemon::auth::RequireAuth;
use crate::daemon::jobs::JobSummary;
use crate::daemon::state::DaemonState;
use crate::inventory::Inventory;
use axum::Json;
use axum::extract::State;
use serde::Serialize;
use std::path::Path;

/// Public health response returned by `GET /api/v1/health`.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    /// Service readiness label (`"ok"`).
    pub status: &'static str,
    /// `Cargo.toml` crate version.
    pub version: &'static str,
    /// Process uptime in whole seconds.
    pub uptime_secs: u64,
}

/// Public liveness and readiness probe handler.
pub async fn get_health(State(state): State<DaemonState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        uptime_secs: state.uptime_secs(),
    })
}

/// Readiness and topology counters returned inside [`SystemOverviewResponse`].
#[derive(Debug, Default, Serialize)]
pub struct OverviewCounts {
    /// Total inventory hosts excluding `localhost`.
    pub hosts: usize,
    /// Hosts in `kube_control_plane`.
    pub control_plane_hosts: usize,
    /// Hosts in `kube_node`.
    pub worker_hosts: usize,
    /// Hosts in `etcd`.
    pub etcd_hosts: usize,
    /// Ready binary count under `downloads/bin`.
    pub binaries_ready: usize,
    /// Expected core binary count (`5`).
    pub binaries_total: usize,
    /// Ready archive count under `downloads/tar`.
    pub archives_ready: usize,
    /// Expected core archive count (`5`).
    pub archives_total: usize,
    /// Ready `.tar` image archives under `downloads/images`.
    pub images_ready: usize,
    /// Expected image count rendered from `configs/images.yaml`.
    pub images_total: usize,
    /// Public `.crt` certificates found under `pki/`.
    pub certificates: usize,
    /// Backup files found under `rtkk_bak/`.
    pub backups: usize,
}

/// Authenticated workspace overview response returned by `GET /api/v1/system/overview`.
#[derive(Debug, Serialize)]
pub struct SystemOverviewResponse {
    /// Whether `configs/inventory.yaml` exists in the workspace.
    pub workspace_initialized: bool,
    /// Active mutating job ID if a cluster write operation is running.
    pub active_mutating_job_id: Option<String>,
    /// Up to 10 most recent jobs.
    pub recent_jobs: Vec<JobSummary>,
    /// Aggregated cluster & artifact counts.
    pub counts: OverviewCounts,
}

const CORE_BINARIES: &[&str] = &["kubeadm", "kubelet", "kubectl", "runc", "helm"];
const CORE_ARCHIVES: &[&str] = &[
    "containerd.tar.gz",
    "etcd.tar.gz",
    "cni-plugins.tgz",
    "helm.tar.gz",
    "cilium-chart.tgz",
];

fn count_files_with_ext_recursive(dir: &Path, ext: Option<&str>) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut count = 0usize;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            count = count.saturating_add(count_files_with_ext_recursive(&p, ext));
        } else if p.is_file() {
            match ext {
                None => count = count.saturating_add(1),
                Some(wanted) => {
                    if p.extension().and_then(|e| e.to_str()) == Some(wanted) {
                        count = count.saturating_add(1);
                    }
                }
            }
        }
    }
    count
}

fn compute_overview_counts(ws: &crate::workspace::Workspace) -> OverviewCounts {
    let mut counts = OverviewCounts {
        binaries_total: CORE_BINARIES.len(),
        archives_total: CORE_ARCHIVES.len(),
        ..OverviewCounts::default()
    };

    for bin in CORE_BINARIES {
        if ws.downloads_bin_dir().join(bin).is_file() {
            counts.binaries_ready = counts.binaries_ready.saturating_add(1);
        }
    }
    for arc in CORE_ARCHIVES {
        if ws.downloads_tar_dir().join(arc).is_file() {
            counts.archives_ready = counts.archives_ready.saturating_add(1);
        }
    }
    counts.images_ready = count_files_with_ext_recursive(&ws.downloads_images_dir(), Some("tar"));
    counts.certificates = count_files_with_ext_recursive(&ws.pki_dir(), Some("crt"));
    counts.backups = count_files_with_ext_recursive(&ws.backups_dir(), None);

    if let Ok(inv_text) = std::fs::read_to_string(ws.inventory_path())
        && let Ok(inv) = serde_yml::from_str::<Inventory>(&inv_text)
        && let Ok(mut flat) = crate::inventory::resolve_inventory(&inv)
    {
        counts.hosts = flat.groups.get("all").map_or(0, Vec::len);
        counts.control_plane_hosts = flat.groups.get("kube_control_plane").map_or(0, Vec::len);
        counts.worker_hosts = flat.groups.get("kube_node").map_or(0, Vec::len);
        counts.etcd_hosts = flat
            .groups
            .get("etcd")
            .map_or(counts.control_plane_hosts, Vec::len);

        crate::playbook::inject_dynamic_inventory_vars(&flat.groups, &mut flat.host_vars);
        let inv_str = ws.inventory_path().to_string_lossy().into_owned();
        crate::playbook::inject_runtime_vars(&mut flat.host_vars, &inv_str);
        if let Ok(doc) = serde_yml::from_str::<serde_json::Value>(&inv_text)
            && let Ok(specs) =
                crate::registry::load_rendered_image_specs_from_doc(&doc, &ws.images_path())
        {
            counts.images_total = specs.len();
        }
    }

    counts
}

/// Authenticated workspace overview handler (`GET /api/v1/system/overview`).
pub async fn get_system_overview(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
) -> Json<SystemOverviewResponse> {
    let ws = state.workspace().clone();
    let counts = tokio::task::spawn_blocking(move || compute_overview_counts(&ws))
        .await
        .unwrap_or_default();
    let (recent_jobs, _) = state.jobs().list_jobs(10, None);

    Json(SystemOverviewResponse {
        workspace_initialized: state.workspace().inventory_path().is_file(),
        active_mutating_job_id: state.jobs().active_mutating_job_id(),
        recent_jobs,
        counts,
    })
}
