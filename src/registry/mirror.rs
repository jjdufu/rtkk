//! Public registry transparent mirror project definitions and `Harbor` project bootstrapper.

use super::RegistryConfig;
use super::build_http_client;
use super::client::{normalize_registry_host_prefix, probe_registry_base_url};
use std::collections::HashSet;

/// Mapping from an upstream public registry hostname to a target private registry project name.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PublicRegistryMirror {
    /// Upstream public registry hostname (e.g. `"docker.io"` or `"registry.k8s.io"`).
    pub host: String,
    /// Target private registry project name (e.g. `"dockerhub"` or `"k8s"`).
    pub project: String,
}

/// Summary of a `Harbor` mirror project synchronization run.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct HarborMirrorSyncOutcome {
    /// Resolved registry base `URL` probed during project creation.
    pub base_url: String,
    /// Projects newly created during this run (`HTTP` `2xx`).
    pub created: Vec<String>,
    /// Projects that already existed (`HTTP` `409 Conflict`).
    pub existing: Vec<String>,
    /// Non-fatal warnings emitted when target registry is non-`Harbor` (`404`) or lacks permission (`401`/`403`).
    pub warnings: Vec<String>,
}

const DEFAULT_MIRROR_PAIRS: &[(&str, &str)] = &[
    ("docker.io", "dockerhub"),
    ("registry-1.docker.io", "dockerhub"),
    ("index.docker.io", "dockerhub"),
    ("registry.k8s.io", "k8s"),
    ("quay.io", "quay"),
    ("gcr.io", "gcr"),
    ("k8s.gcr.io", "gcr"),
    ("ghcr.io", "ghcr"),
    ("mcr.microsoft.com", "mcr"),
    ("public.ecr.aws", "ecr-public"),
    ("nvcr.io", "nvcr"),
    ("registry.gitlab.com", "gitlab"),
];

/// Returns the default 12-entry upstream-to-project mirror mapping table.
#[must_use]
pub fn default_public_registry_mirrors() -> Vec<PublicRegistryMirror> {
    DEFAULT_MIRROR_PAIRS
        .iter()
        .map(|(host, project)| PublicRegistryMirror {
            host: (*host).to_string(),
            project: (*project).to_string(),
        })
        .collect()
}

fn validate_and_dedup_projects(raw_projects: &[String]) -> Result<Vec<String>, String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for raw in raw_projects {
        let name = raw.trim();
        if name.is_empty() {
            continue;
        }
        let valid = name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
            && name
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
        if !valid {
            return Err(format!(
                "invalid Harbor mirror project name '{name}' (allowed: lowercase [a-z0-9._-], starting with [a-z0-9])"
            ));
        }
        if seen.insert(name.to_string()) {
            out.push(name.to_string());
        }
    }
    Ok(out)
}

#[derive(Debug)]
enum ProjectCreateStatus {
    Created,
    AlreadyExists,
    NotHarbor404,
    PermissionDenied(reqwest::StatusCode),
}

async fn create_single_harbor_project(
    client: &reqwest::Client,
    base_url: &str,
    registry: &RegistryConfig,
    project_name: &str,
) -> Result<ProjectCreateStatus, String> {
    let is_public = registry.harbor_project_public;
    let public_str = if is_public { "true" } else { "false" };
    let payload = serde_json::json!({
        "project_name": project_name,
        "public": is_public,
        "metadata": { "public": public_str }
    });
    let creds = match (&registry.username, &registry.password) {
        (Some(u), Some(p)) if !u.is_empty() => Some((u.as_str(), p.as_str())),
        _ => None,
    };
    let endpoints = [
        format!("{base_url}/api/v2.0/projects"),
        format!("{base_url}/api/projects"),
    ];
    let mut saw_404 = false;
    for url in endpoints {
        let mut req = client
            .post(&url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&payload);
        if let Some((u, p)) = creds {
            req = req.basic_auth(u, Some(p));
        }
        let res = req
            .send()
            .await
            .map_err(|e| format!("Harbor project API request to {url} failed: {e}"))?;
        let status = res.status();
        if status.is_success() {
            return Ok(ProjectCreateStatus::Created);
        }
        if status == reqwest::StatusCode::CONFLICT {
            return Ok(ProjectCreateStatus::AlreadyExists);
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            saw_404 = true;
            continue;
        }
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Ok(ProjectCreateStatus::PermissionDenied(status));
        }
        let body = res.text().await.unwrap_or_default();
        let snippet = body.get(..200.min(body.len())).unwrap_or(&body);
        return Err(format!(
            "Harbor project creation for '{project_name}' at {url} returned HTTP {status}: {snippet}"
        ));
    }
    if saw_404 {
        Ok(ProjectCreateStatus::NotHarbor404)
    } else {
        Err(format!(
            "Harbor project creation for '{project_name}' yielded no usable API endpoint"
        ))
    }
}

/// Ensures deduplicated `Harbor` mirror projects exist on `registry` via `POST /api/v2.0/projects`.
///
/// - `201 Created` (or `2xx`) is recorded in `created`.
/// - `409 Conflict` is treated as idempotent success and recorded in `existing`.
/// - `404 Not Found` (non-`Harbor` registry such as `CNCF` `Distribution`) or `401`/`403`
///   (insufficient privilege to create projects) emits a warning in `warnings` without failing,
///   unless `strict` is `true`.
///
/// # Errors
///
/// Returns an error if `projects` contains an invalid project identifier, `registry.url` is empty,
/// probing or connecting to `registry` fails, `Harbor` returns `5xx`, or `strict` is `true` on `401`/`403`/`404`.
pub async fn ensure_harbor_mirror_projects(
    registry: &RegistryConfig,
    projects: &[String],
    strict: bool,
) -> Result<HarborMirrorSyncOutcome, String> {
    let unique_projects = validate_and_dedup_projects(projects)?;
    if unique_projects.is_empty() {
        return Ok(HarborMirrorSyncOutcome::default());
    }
    let normalized = normalize_registry_host_prefix(&registry.url);
    let domain = normalized
        .split('/')
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "registry.url host is empty".to_string())?;
    let http_client = build_http_client(registry.insecure, 15, registry.ca_cert_file.as_deref())?;
    let base_url = probe_registry_base_url(&http_client, domain, registry).await?;
    let mut outcome = HarborMirrorSyncOutcome {
        base_url: base_url.clone(),
        ..HarborMirrorSyncOutcome::default()
    };
    let joined_list = unique_projects.join(", ");
    for project_name in &unique_projects {
        match create_single_harbor_project(&http_client, &base_url, registry, project_name).await? {
            ProjectCreateStatus::Created => {
                crate::log_println!("Created Harbor mirror project '{project_name}' at {base_url}");
                outcome.created.push(project_name.clone());
            }
            ProjectCreateStatus::AlreadyExists => {
                outcome.existing.push(project_name.clone());
            }
            ProjectCreateStatus::NotHarbor404 => {
                let msg = format!(
                    "Warning: Registry at {base_url} returned 404 on Harbor project API (non-Harbor registry); skipped auto-creating mirror projects [{joined_list}]"
                );
                if strict {
                    return Err(msg);
                }
                crate::log_println!("{msg}");
                outcome.warnings.push(msg);
                break;
            }
            ProjectCreateStatus::PermissionDenied(status) => {
                let msg = format!(
                    "Warning: Registry credentials received HTTP {status} creating Harbor project '{project_name}' at {base_url}; pre-create mirror projects [{joined_list}] manually if missing"
                );
                if strict {
                    return Err(msg);
                }
                crate::log_println!("{msg}");
                outcome.warnings.push(msg);
                break;
            }
        }
    }
    Ok(outcome)
}
