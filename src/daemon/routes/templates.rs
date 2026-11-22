//! Live `.gtmpl` & `images.yaml` rendering preview endpoint (`POST /api/v1/templates/render`).

use crate::daemon::auth::RequireAuth;
use crate::daemon::state::{ApiError, DaemonState};
use crate::inventory::{Inventory, resolve_inventory};
use crate::template::{TemplateRegistry, json_to_gtmpl};
use crate::workspace::Workspace;
use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const PREVIEW_TEMPLATES: &[(&str, &str, &str)] = &[
    (
        "kubeadm-config.yaml",
        "templates/kubeadm-config.yaml.gtmpl",
        "yaml",
    ),
    (
        "containerd-config.toml",
        "templates/containerd-config.toml.gtmpl",
        "toml",
    ),
    ("etcd.service", "templates/etcd.service.gtmpl", "systemd"),
    (
        "cilium-values.yaml",
        "templates/cilium-values.yaml.gtmpl",
        "yaml",
    ),
    (
        "local-volume.yaml",
        "templates/local-volume.yaml.gtmpl",
        "yaml",
    ),
    (
        "nodelocaldns.yaml",
        "templates/nodelocaldns.yaml.gtmpl",
        "yaml",
    ),
    ("images.yaml", "images.yaml", "yaml"),
];

/// Request body for `POST /api/v1/templates/render`.
#[derive(Debug, Default, Deserialize)]
pub struct RenderTemplatesRequest {
    /// Target hostname from inventory (empty selects first control-plane or first host).
    #[serde(default)]
    pub host: String,
    /// Subset of template names or `["all"]` (empty or `"all"` renders all 7 templates).
    #[serde(default)]
    pub templates: Vec<String>,
}

/// Single rendered configuration preview file with syntax validation status.
#[derive(Debug, Serialize)]
pub struct RenderedTemplateFile {
    /// Logical output filename (e.g. `"containerd-config.toml"`).
    pub name: String,
    /// Source relative path inside `configs/`.
    pub source_template: String,
    /// Format kind (`"yaml"`, `"toml"`, or `"systemd"`).
    pub format: String,
    /// Rendered plain-text configuration content.
    pub content: String,
    /// Whether post-render syntax validation passed.
    pub syntax_valid: bool,
    /// Optional syntax validation diagnostic.
    pub validation_error: Option<String>,
}

/// Response returned by `POST /api/v1/templates/render`.
#[derive(Debug, Serialize)]
pub struct RenderTemplatesResponse {
    /// Resolved target hostname used as template variable context.
    pub host: String,
    /// Rendered files in deterministic order.
    pub files: Vec<RenderedTemplateFile>,
}

fn validate_multi_doc_yaml(rendered: &str) -> Result<(), String> {
    let mut current = String::new();
    let mut doc_count = 0usize;
    for line in rendered.lines() {
        if line.trim_end() == "---" {
            if !current.trim().is_empty() {
                let v: Value =
                    serde_yml::from_str(&current).map_err(|e| format!("YAML syntax error: {e}"))?;
                if !v.is_null() {
                    doc_count = doc_count.saturating_add(1);
                }
            }
            current.clear();
        } else {
            current.push_str(line);
            current.push('\n');
        }
    }
    if !current.trim().is_empty() {
        let v: Value =
            serde_yml::from_str(&current).map_err(|e| format!("YAML syntax error: {e}"))?;
        if !v.is_null() {
            doc_count = doc_count.saturating_add(1);
        }
    }
    if doc_count == 0 {
        return Err("Rendered YAML contained zero non-empty documents".to_string());
    }
    Ok(())
}

fn validate_rendered_syntax(format_kind: &str, content: &str) -> Result<(), String> {
    if content.contains("{{") || content.contains("<no value>") {
        return Err("Unresolved template token detected in rendered output".to_string());
    }
    match format_kind {
        "yaml" => validate_multi_doc_yaml(content),
        "toml" => {
            if !content.lines().any(|l| l.contains('=')) {
                return Err("Rendered TOML contains no key-value assignments".to_string());
            }
            Ok(())
        }
        "systemd" => {
            if !content.contains("[Unit]") || !content.contains("[Service]") {
                return Err("Rendered systemd unit missing [Unit] or [Service] section".to_string());
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn load_template_registry(ws: &Workspace) -> Result<TemplateRegistry, ApiError> {
    let reg = TemplateRegistry::new();
    let tmpl_dir = ws.configs_dir().join("templates");
    if let Ok(entries) = std::fs::read_dir(&tmpl_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("gtmpl")
                && let Some(fname) = p.file_name().and_then(|n| n.to_str())
            {
                let text = std::fs::read_to_string(&p).map_err(|e| {
                    ApiError::internal(format!("Failed to read template {}: {e}", p.display()))
                })?;
                reg.register(fname, &text).map_err(ApiError::internal)?;
            }
        }
    }
    Ok(reg)
}

fn render_preview_sync(
    ws: &Workspace,
    req: &RenderTemplatesRequest,
) -> Result<RenderTemplatesResponse, ApiError> {
    let inv_path = ws.inventory_path();
    let inv_raw = std::fs::read_to_string(&inv_path)
        .map_err(|e| ApiError::not_found(format!("Failed to read inventory.yaml: {e}")))?;
    let inv: Inventory = serde_yml::from_str(&inv_raw)
        .map_err(|e| ApiError::invalid_argument(format!("Invalid inventory.yaml: {e}")))?;
    let mut flat = resolve_inventory(&inv).map_err(ApiError::invalid_argument)?;
    crate::playbook::inject_dynamic_inventory_vars(&flat.groups, &mut flat.host_vars);
    crate::playbook::inject_runtime_vars(&mut flat.host_vars, &inv_path.to_string_lossy());

    let selected_host = if req.host.trim().is_empty() {
        flat.groups
            .get("kube_control_plane")
            .and_then(|v| v.first())
            .or_else(|| flat.groups.get("all").and_then(|v| v.first()))
            .cloned()
            .ok_or_else(|| ApiError::invalid_argument("Inventory contains zero hosts to render"))?
    } else {
        req.host.trim().to_string()
    };

    let host_vars_map = flat.host_vars.get(&selected_host).ok_or_else(|| {
        ApiError::invalid_argument(format!("Host '{selected_host}' not found in inventory"))
    })?;
    let host_vars_val =
        serde_json::to_value(host_vars_map).map_err(|e| ApiError::internal(e.to_string()))?;
    let gtmpl_ctx = json_to_gtmpl(&host_vars_val);
    let registry = load_template_registry(ws)?;

    let render_all = req.templates.is_empty() || req.templates.iter().any(|t| t == "all");
    let mut files = Vec::new();
    for &(logical_name, rel_src, fmt_kind) in PREVIEW_TEMPLATES {
        if !render_all
            && !req
                .templates
                .iter()
                .any(|t| t == logical_name || t == rel_src || rel_src.ends_with(t.as_str()))
        {
            continue;
        }
        let full_path = ws.configs_dir().join(rel_src);
        let raw_tmpl = std::fs::read_to_string(&full_path).map_err(|e| {
            ApiError::not_found(format!(
                "Missing template file {}: {e}",
                full_path.display()
            ))
        })?;
        let rendered = registry
            .render(&raw_tmpl, &gtmpl_ctx)
            .map_err(ApiError::template_render)?;
        let (syntax_valid, validation_error) = match validate_rendered_syntax(fmt_kind, &rendered) {
            Ok(()) => (true, None),
            Err(err) => (false, Some(err)),
        };
        files.push(RenderedTemplateFile {
            name: logical_name.to_string(),
            source_template: rel_src.to_string(),
            format: fmt_kind.to_string(),
            content: rendered,
            syntax_valid,
            validation_error,
        });
    }

    Ok(RenderTemplatesResponse {
        host: selected_host,
        files,
    })
}

/// Renders `.gtmpl` templates and `images.yaml` for a selected target host (`POST /api/v1/templates/render`).
///
/// # Errors
///
/// Returns [`ApiError`] (`400`, `404`, `422`, or `500`) if the host is missing or rendering fails.
pub async fn render_templates(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
    Json(req): Json<RenderTemplatesRequest>,
) -> Result<Json<RenderTemplatesResponse>, ApiError> {
    let ws = state.workspace().clone();
    let res = tokio::task::spawn_blocking(move || render_preview_sync(&ws, &req))
        .await
        .map_err(|e| ApiError::internal(format!("Template render task join error: {e}")))??;
    Ok(Json(res))
}
