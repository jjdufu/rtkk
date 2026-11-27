//! Public registry transparent mirror project definitions, target resolver, and `Harbor` project bootstrapper.

use super::RegistryConfig;
use super::build_http_client;
use super::client::{normalize_registry_host_prefix, probe_registry_base_url};
use super::pull::parse_image_ref;
use serde_json::Value;
use std::collections::HashSet;

/// Default `Harbor` project name used by `/etc/containerd/certs.d/_default/hosts.toml`.
pub const DEFAULT_FALLBACK_MIRROR_PROJECT: &str = "external";

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

/// Resolved mapping from a raw upstream image reference to its transparent `Harbor` target path.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TransparentMirrorTarget {
    /// Original user-provided image string (trimmed).
    pub raw_input: String,
    /// Canonicalized upstream image reference (including `docker.io/library/` normalization).
    pub normalized_source: String,
    /// Normalized upstream registry host (e.g. `"docker.io"`).
    pub source_host: String,
    /// Normalized upstream repository path (e.g. `"library/nginx"`).
    pub source_repository: String,
    /// Effective tag to push to `Harbor` (defaults to `"latest"` when omitted).
    pub tag: String,
    /// Optional pinned `@sha256:...` digest from the input reference.
    pub digest: Option<String>,
    /// Target `Harbor` project name (e.g. `"dockerhub"` or `"external"`).
    pub target_project: String,
    /// Target repository path inside `Harbor` (`"<target_project>/<source_repository>"`).
    pub target_repository: String,
    /// Full target image reference (`"<registry_host>/<target_project>/<source_repository>:<tag>"`).
    pub target_ref: String,
    /// Human-readable description of the mirror rule that matched.
    pub matched_rule: String,
    /// Whether the target fell back to the `_default` catch-all project.
    pub used_default_fallback: bool,
    /// Whether the target project already exists on `Harbor` (populated during remote planning).
    pub project_exists: Option<bool>,
    /// Whether the target image tag already exists on `Harbor` (populated during remote planning).
    pub already_exists: Option<bool>,
    /// Optional per-item validation or probe error message.
    pub error: Option<String>,
}

const DEFAULT_MIRROR_PAIRS: &[(&str, &str)] = &[
    ("docker.io", "dockerhub"),
    ("registry-1.docker.io", "dockerhub"),
    ("index.docker.io", "dockerhub"),
    ("registry.k8s.io", "k8s"),
    ("quay.io", "quay"),
    ("gcr.io", "gcr"),
    ("k8s.gcr.io", "gcr"),
    ("mirror.gcr.io", "gcr"),
    ("ghcr.io", "ghcr"),
    ("mcr.microsoft.com", "mcr"),
    ("public.ecr.aws", "ecr-public"),
    ("nvcr.io", "nvcr"),
    ("registry.gitlab.com", "gitlab"),
    ("code.forgejo.org", "forgejo"),
    ("higress-registry.cn-hangzhou.cr.aliyuncs.com", "higress"),
    ("registry.cn-hangzhou.cr.aliyuncs.com", "aliyun-hz"),
    ("lscr.io", "lscr"),
    ("docker.elastic.co", "elastic"),
    ("icr.io", "icr"),
    ("registry.redhat.io", "redhat"),
    ("registry.access.redhat.com", "redhat"),
    ("cgr.dev", "chainguard"),
];

/// Returns the default 22-entry upstream-to-project mirror mapping table covering 17 `Harbor` projects.
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

/// Extracts `public_registry_mirrors` and `public_registry_default_mirror_project` from inventory `vars`.
///
/// When `public_registry_mirrors` is omitted or null, falls back to [`default_public_registry_mirrors`].
/// When `public_registry_default_mirror_project` is omitted or null, defaults to [`DEFAULT_FALLBACK_MIRROR_PROJECT`].
/// Explicitly setting it to an empty string disables `_default` fallback.
#[must_use]
pub fn parse_mirror_settings_from_vars(
    vars: &Value,
) -> (Vec<PublicRegistryMirror>, Option<String>) {
    let mirrors = match vars.get("public_registry_mirrors") {
        Some(Value::Array(arr)) => {
            let mut out = Vec::with_capacity(arr.len());
            for item in arr {
                if let (Some(h), Some(p)) = (
                    item.get("host").and_then(Value::as_str).map(str::trim),
                    item.get("project").and_then(Value::as_str).map(str::trim),
                ) && !h.is_empty()
                    && !p.is_empty()
                {
                    out.push(PublicRegistryMirror {
                        host: h.to_string(),
                        project: p.to_string(),
                    });
                }
            }
            out
        }
        _ => default_public_registry_mirrors(),
    };

    let default_project = match vars.get("public_registry_default_mirror_project") {
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        None | Some(Value::Null) => Some(DEFAULT_FALLBACK_MIRROR_PROJECT.to_string()),
        _ => None,
    };

    (mirrors, default_project)
}

/// Collects deduplicated `Harbor` project names from explicit mirrors plus optional `_default` fallback project.
#[must_use]
pub fn collect_mirror_project_names(
    mirrors: &[PublicRegistryMirror],
    default_project: Option<&str>,
) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for m in mirrors {
        let p = m.project.trim();
        if !p.is_empty() && seen.insert(p.to_string()) {
            out.push(p.to_string());
        }
    }
    if let Some(dp) = default_project.map(str::trim).filter(|s| !s.is_empty())
        && seen.insert(dp.to_string())
    {
        out.push(dp.to_string());
    }
    out
}

/// Parses one or more raw image references from newline/comma-separated text or `YAML` `image:` lines.
#[must_use]
pub fn extract_image_refs_from_text(raw: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for line in raw.lines() {
        let stripped_comment = line.split('#').next().unwrap_or("").trim();
        if stripped_comment.is_empty() {
            continue;
        }
        let cleaned = stripped_comment
            .strip_prefix('-')
            .map_or(stripped_comment, str::trim);
        let candidate = if let Some(rest) = cleaned.strip_prefix("image:") {
            rest.trim()
        } else {
            cleaned
        };
        for token in candidate.split(',') {
            let unquoted = token.trim().trim_matches('"').trim_matches('\'').trim();
            if !unquoted.is_empty() && seen.insert(unquoted.to_string()) {
                out.push(unquoted.to_string());
            }
        }
    }
    out
}

/// Validates a `Harbor` project name (`[a-z0-9][a-z0-9._-]*`).
///
/// # Errors
///
/// Returns an error if `name` is empty or contains disallowed characters.
pub fn validate_harbor_project_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Harbor project name cannot be empty".to_string());
    }
    let valid = trimmed
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
        && trimmed
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    if !valid {
        return Err(format!(
            "invalid Harbor mirror project name '{trimmed}' (allowed: lowercase [a-z0-9._-], starting with [a-z0-9])"
        ));
    }
    Ok(trimmed.to_string())
}

/// Resolves an upstream image reference into its transparent private registry target path.
///
/// # Errors
///
/// Returns an error if `raw_image` cannot be parsed, `registry_url` has no host, `raw_image` already
/// targets the private registry host, or neither an explicit mirror nor `default_project` matches.
pub fn resolve_transparent_target(
    raw_image: &str,
    registry_url: &str,
    mirrors: &[PublicRegistryMirror],
    default_project: Option<&str>,
) -> Result<TransparentMirrorTarget, String> {
    let raw_trimmed = raw_image.trim();
    let parsed = parse_image_ref(raw_trimmed)?;
    let normalized_reg = normalize_registry_host_prefix(registry_url);
    let reg_domain = normalized_reg
        .split('/')
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "registry.url host is empty".to_string())?;

    if parsed.host.eq_ignore_ascii_case(reg_domain) {
        return Err(format!(
            "image '{raw_trimmed}' already points to private registry host '{reg_domain}'"
        ));
    }

    let (target_project, matched_rule, used_default_fallback) = if let Some(m) = mirrors
        .iter()
        .find(|m| m.host.eq_ignore_ascii_case(&parsed.host))
    {
        let proj = validate_harbor_project_name(&m.project)?;
        (proj.clone(), format!("{} -> {proj}", m.host), false)
    } else if let Some(dp) = default_project.map(str::trim).filter(|s| !s.is_empty()) {
        let proj = validate_harbor_project_name(dp)?;
        (proj.clone(), format!("_default -> {proj}"), true)
    } else {
        return Err(format!(
            "no mirror rule matches host '{}' and no _default fallback project is configured",
            parsed.host
        ));
    };

    let tag = if let Some(ref t) = parsed.tag {
        t.clone()
    } else if let Some(ref d) = parsed.digest {
        let hex = d.strip_prefix("sha256:").unwrap_or(d);
        let short = hex.get(..12.min(hex.len())).unwrap_or(hex);
        format!("sha256-{short}")
    } else {
        "latest".to_string()
    };

    let normalized_source = match (&parsed.tag, &parsed.digest) {
        (Some(t), Some(d)) => format!("{}/{}:{t}@{d}", parsed.host, parsed.repository),
        (Some(t), None) => format!("{}/{}:{t}", parsed.host, parsed.repository),
        (None, Some(d)) => format!("{}/{}@{d}", parsed.host, parsed.repository),
        (None, None) => format!("{}/{}:latest", parsed.host, parsed.repository),
    };

    let target_repository = format!("{target_project}/{}", parsed.repository);
    let target_ref = format!("{reg_domain}/{target_repository}:{tag}");

    Ok(TransparentMirrorTarget {
        raw_input: raw_trimmed.to_string(),
        normalized_source,
        source_host: parsed.host,
        source_repository: parsed.repository,
        tag,
        digest: parsed.digest,
        target_project,
        target_repository,
        target_ref,
        matched_rule,
        used_default_fallback,
        project_exists: None,
        already_exists: None,
        error: None,
    })
}

fn validate_and_dedup_projects(raw_projects: &[String]) -> Result<Vec<String>, String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for raw in raw_projects {
        let name = raw.trim();
        if name.is_empty() {
            continue;
        }
        let valid_name = validate_harbor_project_name(name)?;
        if seen.insert(valid_name.clone()) {
            out.push(valid_name);
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
