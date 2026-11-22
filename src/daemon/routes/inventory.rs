//! Inventory studio API routes (`GET/PUT /api/v1/inventory`, `validate`, and SSH/`become` `probe`).

use crate::cli::build_target_configs_for_flat;
use crate::connector::create_connector;
use crate::daemon::auth::RequireAuth;
use crate::daemon::mask::{
    contains_mask_placeholder, mask_inventory_value, restore_masked_secrets,
};
use crate::daemon::state::{ApiError, DaemonState};
use crate::inventory::{FlatInventory, Inventory, resolve_inventory};
use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::time::{Duration, Instant};

/// Dynamically derived cluster variables exposed to the Inventory Studio UI.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DerivedInventoryVars {
    /// First control-plane hostname.
    pub first_control_plane: Option<String>,
    /// Comma-separated `https://ip:2379` etcd endpoints.
    pub etcd_endpoints_str: String,
    /// Comma-separated `name=https://ip:2380` etcd initial cluster string.
    pub etcd_initial_cluster: String,
}

/// Response returned by `GET /api/v1/inventory`.
#[derive(Debug, Serialize)]
pub struct GetInventoryResponse {
    /// Masked structured inventory object.
    pub inventory: Value,
    /// Resolved deterministic group -> hostnames map.
    pub groups: HashMap<String, Vec<String>>,
    /// Derived topology variables.
    pub derived: DerivedInventoryVars,
    /// Masked YAML representation for the source editor tab.
    pub masked_yaml: String,
}

/// Payload accepted by `PUT /api/v1/inventory` and `POST /api/v1/inventory/validate`.
#[derive(Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum InventoryWritePayload {
    /// Structured JSON inventory object from form tabs.
    Structured {
        /// Candidate inventory object (may contain `"********"` placeholders).
        inventory: Value,
    },
    /// Raw YAML text from the YAML source tab.
    Yaml {
        /// Candidate YAML text (may contain `"********"` placeholders).
        raw_yaml: String,
    },
}

/// Response returned by `PUT /api/v1/inventory`.
#[derive(Debug, Serialize)]
pub struct PutInventoryResponse {
    /// Always `true` when persisted.
    pub saved: bool,
    /// Workspace-relative `.bak` snapshot path.
    pub backup_path: &'static str,
    /// Recomputed derived cluster variables.
    pub derived: DerivedInventoryVars,
}

/// Response returned by `POST /api/v1/inventory/validate`.
#[derive(Debug, Serialize)]
pub struct ValidateInventoryResponse {
    /// Always `true` when valid (`400` returned on hard errors).
    pub valid: bool,
    /// Count of resolved cluster hosts excluding `localhost`.
    pub hosts_count: usize,
    /// Resolved group memberships.
    pub groups: HashMap<String, Vec<String>>,
    /// Derived cluster variables.
    pub derived: DerivedInventoryVars,
    /// Non-blocking configuration advisories.
    pub warnings: Vec<String>,
}

/// Request body for `POST /api/v1/inventory/probe`.
#[derive(Debug, Default, Deserialize)]
pub struct ProbeHostsRequest {
    /// Hostnames to probe (empty means probe all inventory hosts).
    #[serde(default)]
    pub hosts: Vec<String>,
}

/// Per-host read-only connectivity and privilege probe result.
#[derive(Debug, Serialize)]
pub struct HostProbeResult {
    /// Inventory hostname alias.
    pub host: String,
    /// Target IP or address.
    pub ip: String,
    /// Whether connection and privilege escalation succeeded.
    pub reachable: bool,
    /// Effective numeric user ID (`0` when `root` / `sudo` / `su` succeeded).
    pub uid: Option<u32>,
    /// Operating system `PRETTY_NAME`.
    pub os_pretty: String,
    /// Machine architecture (`uname -m`).
    pub arch: String,
    /// Detected network interface names.
    pub nics: Vec<String>,
    /// Round-trip probe latency in milliseconds.
    pub latency_ms: u64,
    /// Missing required Debian 13 packages if detected.
    pub missing_packages: Vec<String>,
    /// Diagnostic error if unreachable.
    pub error: Option<String>,
}

/// Response returned by `POST /api/v1/inventory/probe`.
#[derive(Debug, Serialize)]
pub struct ProbeHostsResponse {
    /// Ordered probe results matching requested hosts.
    pub results: Vec<HostProbeResult>,
}

fn extract_derived(flat: &FlatInventory) -> DerivedInventoryVars {
    let ref_host = flat
        .groups
        .get("kube_control_plane")
        .and_then(|v| v.first())
        .or_else(|| flat.groups.get("all").and_then(|v| v.first()));
    let Some(h) = ref_host else {
        return DerivedInventoryVars::default();
    };
    let Some(vars) = flat.host_vars.get(h) else {
        return DerivedInventoryVars::default();
    };
    DerivedInventoryVars {
        first_control_plane: vars
            .get("first_control_plane")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        etcd_endpoints_str: vars
            .get("etcd_endpoints_str")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        etcd_initial_cluster: vars
            .get("etcd_initial_cluster")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

fn parse_and_resolve_candidate(
    payload: InventoryWritePayload,
    inv_path: &Path,
) -> Result<(Value, FlatInventory, Vec<String>), ApiError> {
    let mut candidate_val: Value = match payload {
        InventoryWritePayload::Structured { inventory } => inventory,
        InventoryWritePayload::Yaml { raw_yaml } => serde_yml::from_str(&raw_yaml)
            .map_err(|e| ApiError::invalid_argument(format!("Invalid YAML syntax: {e}")))?,
    };

    if inv_path.is_file() {
        let disk_raw = std::fs::read_to_string(inv_path)
            .map_err(|e| ApiError::internal(format!("Failed to read existing inventory: {e}")))?;
        if let Ok(disk_val) = serde_yml::from_str::<Value>(&disk_raw) {
            restore_masked_secrets(&mut candidate_val, &disk_val);
        }
    }

    if contains_mask_placeholder(&candidate_val) {
        return Err(ApiError::invalid_argument(
            "Cannot save masked secret placeholder '********' without an existing secret on disk",
        ));
    }

    let yaml_str = serde_yml::to_string(&candidate_val).map_err(|e| {
        ApiError::invalid_argument(format!("Failed to serialize inventory YAML: {e}"))
    })?;
    let parsed_inv: Inventory = serde_yml::from_str(&yaml_str)
        .map_err(|e| ApiError::invalid_argument(format!("Invalid inventory schema: {e}")))?;
    let mut flat = resolve_inventory(&parsed_inv).map_err(ApiError::invalid_argument)?;
    crate::playbook::inject_dynamic_inventory_vars(&flat.groups, &mut flat.host_vars);
    crate::playbook::inject_runtime_vars(&mut flat.host_vars, &inv_path.to_string_lossy());
    let _ = build_target_configs_for_flat(&flat)
        .map_err(|e| ApiError::invalid_argument(e.to_string()))?;

    let mut warnings = Vec::new();
    if flat
        .groups
        .get("kube_control_plane")
        .is_none_or(Vec::is_empty)
    {
        warnings.push("kube_control_plane group has 0 hosts".to_string());
    }
    Ok((candidate_val, flat, warnings))
}

fn write_inventory_with_backup(inv_path: &Path, val: &Value) -> Result<(), ApiError> {
    let bak_path = inv_path.with_extension("yaml.bak");
    if inv_path.is_file() {
        std::fs::copy(inv_path, &bak_path).map_err(|e| {
            ApiError::internal(format!(
                "Failed to write backup {}: {e}",
                bak_path.display()
            ))
        })?;
        #[cfg(unix)]
        {
            let _ = std::fs::set_permissions(&bak_path, std::fs::Permissions::from_mode(0o600));
        }
    }
    let yaml_out = serde_yml::to_string(val)
        .map_err(|e| ApiError::internal(format!("YAML serialization error: {e}")))?;
    let pid = std::process::id();
    let nonce: u64 = rand::random();
    let tmp_path = inv_path.with_file_name(format!(".inventory.yaml.tmp.{pid}.{nonce:016x}"));
    std::fs::write(&tmp_path, yaml_out.as_bytes())
        .map_err(|e| ApiError::internal(format!("Failed to write temp inventory: {e}")))?;
    #[cfg(unix)]
    {
        let _ = std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp_path, inv_path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        ApiError::internal(format!("Failed to atomically replace inventory.yaml: {e}"))
    })
}

/// Reads and masks `configs/inventory.yaml` (`GET /api/v1/inventory`).
///
/// # Errors
///
/// Returns [`ApiError`] if reading or parsing `configs/inventory.yaml` fails.
pub async fn get_inventory(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
) -> Result<Json<GetInventoryResponse>, ApiError> {
    let inv_path = state.workspace().inventory_path();
    let raw_text = tokio::fs::read_to_string(&inv_path)
        .await
        .map_err(|e| ApiError::not_found(format!("Failed to read inventory.yaml: {e}")))?;
    let raw_val: Value = serde_yml::from_str(&raw_text)
        .map_err(|e| ApiError::invalid_argument(format!("Malformed inventory.yaml: {e}")))?;
    let parsed_inv: Inventory = serde_yml::from_str(&raw_text)
        .map_err(|e| ApiError::invalid_argument(format!("Invalid inventory structure: {e}")))?;
    let mut flat = resolve_inventory(&parsed_inv).map_err(ApiError::invalid_argument)?;
    crate::playbook::inject_dynamic_inventory_vars(&flat.groups, &mut flat.host_vars);
    crate::playbook::inject_runtime_vars(&mut flat.host_vars, &inv_path.to_string_lossy());

    let derived = extract_derived(&flat);
    let masked_val = mask_inventory_value(&raw_val);
    let masked_yaml = serde_yml::to_string(&masked_val)
        .map_err(|e| ApiError::internal(format!("Failed to render masked YAML: {e}")))?;

    Ok(Json(GetInventoryResponse {
        inventory: masked_val,
        groups: flat.groups,
        derived,
        masked_yaml,
    }))
}

/// Validates, restores unmodified masked secrets, backs up, and saves `configs/inventory.yaml`.
///
/// # Errors
///
/// Returns [`ApiError`] (`400`, `409`, or `500`) on validation failure, active job lock, or I/O error.
pub async fn put_inventory(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
    Json(payload): Json<InventoryWritePayload>,
) -> Result<Json<PutInventoryResponse>, ApiError> {
    if let Some(active_id) = state.jobs().active_mutating_job_id() {
        return Err(ApiError::conflict_active_job(&active_id));
    }
    let inv_path = state.workspace().inventory_path();
    let (merged_val, flat, _warnings) =
        tokio::task::spawn_blocking(move || -> Result<_, ApiError> {
            let (val, flat, warnings) = parse_and_resolve_candidate(payload, &inv_path)?;
            write_inventory_with_backup(&inv_path, &val)?;
            Ok((val, flat, warnings))
        })
        .await
        .map_err(|e| ApiError::internal(format!("Inventory write join error: {e}")))??;
    let _ = merged_val;
    Ok(Json(PutInventoryResponse {
        saved: true,
        backup_path: "configs/inventory.yaml.bak",
        derived: extract_derived(&flat),
    }))
}

/// Validates a candidate inventory without modifying disk (`POST /api/v1/inventory/validate`).
///
/// # Errors
///
/// Returns [`ApiError::invalid_argument`] if schema or group DAG validation fails.
pub async fn validate_inventory(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
    Json(payload): Json<InventoryWritePayload>,
) -> Result<Json<ValidateInventoryResponse>, ApiError> {
    let inv_path = state.workspace().inventory_path();
    let (_val, flat, warnings) =
        tokio::task::spawn_blocking(move || parse_and_resolve_candidate(payload, &inv_path))
            .await
            .map_err(|e| ApiError::internal(format!("Validate join error: {e}")))??;
    let hosts_count = flat.groups.get("all").map_or(0, Vec::len);
    let derived = extract_derived(&flat);
    Ok(Json(ValidateInventoryResponse {
        valid: true,
        hosts_count,
        groups: flat.groups,
        derived,
        warnings,
    }))
}

async fn probe_single_host(host: String, tc: crate::connector::TargetConfig) -> HostProbeResult {
    let started = Instant::now();
    let ip = tc.host.clone();
    let script = concat!(
        "id -u; ",
        "if [ -f /etc/os-release ]; then . /etc/os-release; printf '%s\\n' \"${PRETTY_NAME:-Linux}\"; else uname -s; fi; ",
        "uname -m; ",
        "if command -v ip >/dev/null 2>&1; then ip -o link show | awk -F': ' '{print $2}' | cut -d'@' -f1 | grep -v '^lo$' | paste -sd, -; else printf 'lo\\n'; fi"
    );
    let probe_fut = async {
        let conn = create_connector(&tc).await.map_err(|e| e.to_string())?;
        let out = conn.execute_shell(script).await.map_err(|e| e.to_string());
        let _ = conn.close().await;
        out
    };
    let latency_ms =
        |s: Instant| -> u64 { u64::try_from(s.elapsed().as_millis()).unwrap_or(u64::MAX) };

    match tokio::time::timeout(Duration::from_secs(8), probe_fut).await {
        Ok(Ok(cmd_res)) if cmd_res.exit_code == 0 => {
            let stdout = String::from_utf8_lossy(&cmd_res.stdout);
            let mut lines = stdout.lines().map(str::trim);
            let uid = lines.next().and_then(|s| s.parse::<u32>().ok());
            let os_pretty = lines.next().unwrap_or("Unknown").to_string();
            let arch = lines.next().unwrap_or("unknown").to_string();
            let nics: Vec<String> = lines
                .next()
                .unwrap_or("")
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            HostProbeResult {
                host,
                ip,
                reachable: true,
                uid,
                os_pretty,
                arch,
                nics,
                latency_ms: latency_ms(started),
                missing_packages: Vec::new(),
                error: None,
            }
        }
        Ok(Ok(cmd_res)) => {
            let stderr = String::from_utf8_lossy(&cmd_res.stderr);
            HostProbeResult {
                host,
                ip,
                reachable: false,
                uid: None,
                os_pretty: String::new(),
                arch: String::new(),
                nics: Vec::new(),
                latency_ms: latency_ms(started),
                missing_packages: Vec::new(),
                error: Some(format!(
                    "Probe exited with {}: {}",
                    cmd_res.exit_code,
                    stderr.trim()
                )),
            }
        }
        Ok(Err(err)) => HostProbeResult {
            host,
            ip,
            reachable: false,
            uid: None,
            os_pretty: String::new(),
            arch: String::new(),
            nics: Vec::new(),
            latency_ms: latency_ms(started),
            missing_packages: Vec::new(),
            error: Some(err),
        },
        Err(_) => HostProbeResult {
            host,
            ip,
            reachable: false,
            uid: None,
            os_pretty: String::new(),
            arch: String::new(),
            nics: Vec::new(),
            latency_ms: latency_ms(started),
            missing_packages: Vec::new(),
            error: Some("Connection timed out after 8s".to_string()),
        },
    }
}

/// Probes SSH / `sudo` / `su` connectivity and OS metadata on target hosts (`POST /api/v1/inventory/probe`).
///
/// # Errors
///
/// Returns [`ApiError`] if the workspace `inventory.yaml` cannot be loaded.
pub async fn probe_inventory_hosts(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
    Json(req): Json<ProbeHostsRequest>,
) -> Result<Json<ProbeHostsResponse>, ApiError> {
    let inv_str = state
        .workspace()
        .inventory_path()
        .to_string_lossy()
        .into_owned();
    let loaded = crate::cli::load_inventory(&inv_str)
        .await
        .map_err(|e| ApiError::invalid_argument(e.to_string()))?;

    let target_hosts: Vec<String> = if req.hosts.is_empty() {
        loaded.flat.groups.get("all").cloned().unwrap_or_default()
    } else {
        req.hosts
    };

    let mut handles = Vec::with_capacity(target_hosts.len());
    for h in target_hosts {
        let tc = if h == "localhost" {
            crate::connector::TargetConfig {
                host: "127.0.0.1".to_string(),
                connector_type: Some(crate::connector::ConnectorType::Local),
                ssh: None,
                privilege: None,
            }
        } else if let Some(cfg) = loaded.target_configs.get(&h) {
            cfg.clone()
        } else {
            return Err(ApiError::invalid_argument(format!(
                "Host '{h}' does not exist in inventory"
            )));
        };
        handles.push(tokio::spawn(probe_single_host(h, tc)));
    }

    let mut results = Vec::with_capacity(handles.len());
    for handle in handles {
        if let Ok(res) = handle.await {
            results.push(res);
        }
    }
    Ok(Json(ProbeHostsResponse { results }))
}
