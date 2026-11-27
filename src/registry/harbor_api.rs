//! `Harbor` v2 project, repository, and artifact management client plus `OCI` manifest existence probes.

use super::RegistryConfig;
use super::build_http_client;
use super::client::{
    fetch_bearer_token, normalize_registry_host_prefix, parse_www_authenticate_bearer,
    probe_registry_base_url,
};
use super::mirror::{PublicRegistryMirror, validate_harbor_project_name};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Summary record for a `Harbor` project reconciled against configured transparent mirror mappings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarborProjectInfo {
    /// Numeric `Harbor` project ID (`0` when configured locally but missing on `Harbor`).
    pub project_id: i64,
    /// Project name (e.g. `"dockerhub"`, `"k8s"`, `"external"`).
    pub name: String,
    /// Whether the project allows anonymous pull (`metadata.public == "true"`).
    pub public: bool,
    /// Number of repositories currently stored in this project.
    pub repo_count: u64,
    /// Optional linked upstream registry ID when configured as a `Harbor` Proxy Cache.
    pub registry_id: Option<i64>,
    /// ISO-8601 creation timestamp reported by `Harbor`.
    pub creation_time: String,
    /// Upstream public registry hostnames mapped to this project in `inventory.yaml`.
    pub mapped_hosts: Vec<String>,
    /// Whether this project is the `_default` catch-all mirror project.
    pub is_default_fallback: bool,
    /// Whether this project currently exists on `Harbor`.
    pub exists: bool,
}

/// Summary record for a repository under a `Harbor` project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarborRepositoryInfo {
    /// Numeric `Harbor` repository ID.
    pub id: i64,
    /// Full repository path (`"<project>/<short_name>"`).
    pub name: String,
    /// Repository path relative to its owning project (e.g. `"library/nginx"`).
    pub short_name: String,
    /// Number of artifacts currently stored in this repository.
    pub artifact_count: u64,
    /// Total pull counter reported by `Harbor`.
    pub pull_count: u64,
    /// Last update timestamp reported by `Harbor`.
    pub update_time: String,
}

/// Single tag attached to a `Harbor` artifact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarborTagInfo {
    /// Tag name (e.g. `"1.27"`).
    pub name: String,
    /// Tag push timestamp.
    pub push_time: String,
    /// Tag last pull timestamp.
    pub pull_time: String,
}

/// Summary record for an artifact stored in a `Harbor` repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarborArtifactInfo {
    /// Content-addressable `sha256:...` digest of the artifact manifest.
    pub digest: String,
    /// Total artifact size in bytes.
    pub size: u64,
    /// Artifact push timestamp.
    pub push_time: String,
    /// Manifest media type.
    pub media_type: String,
    /// Tags currently pointing to this artifact.
    pub tags: Vec<HarborTagInfo>,
    /// Optional target CPU architecture from `extra_attrs.architecture`.
    pub architecture: Option<String>,
    /// Optional target OS from `extra_attrs.os`.
    pub os: Option<String>,
}

/// Reconciled project listing returned by [`list_harbor_projects_reconciled`].
#[derive(Debug, Clone, Serialize)]
pub struct HarborProjectsOverview {
    /// Probed base URL of the target registry (e.g. `"https://192.168.26.3"`).
    pub base_url: String,
    /// Registry host prefix extracted from `registry.url`.
    pub registry_host: String,
    /// Whether the target registry responded to `Harbor` v2 `/api/v2.0/projects`.
    pub is_harbor: bool,
    /// Configured `enable_public_registry_mirrors` state.
    pub mirrors_enabled: bool,
    /// Configured `public_registry_mirrors_fallback` state.
    pub allow_fallback: bool,
    /// Configured `_default` catch-all project name (if enabled).
    pub default_fallback_project: Option<String>,
    /// All existing `Harbor` projects plus any configured mirror projects that are missing on `Harbor`.
    pub projects: Vec<HarborProjectInfo>,
    /// Names of configured mirror projects that do not yet exist on `Harbor`.
    pub missing_mirror_projects: Vec<String>,
}

/// Double-encodes a `Harbor` repository path segment so nested `/` becomes `%252F` per `Harbor` v2 API.
#[must_use]
pub fn encode_harbor_repo_segment(repo_short_name: &str) -> String {
    let trimmed = repo_short_name.trim().trim_matches('/');
    let mut out = String::with_capacity(trimmed.len().saturating_mul(2));
    for b in trimmed.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(*b));
            }
            b'/' => out.push_str("%252F"),
            other => {
                use std::fmt::Write as _;
                let _ = write!(out, "%{other:02X}");
            }
        }
    }
    out
}

fn encode_single_path_segment(seg: &str) -> String {
    let trimmed = seg.trim();
    let mut out = String::with_capacity(trimmed.len().saturating_mul(2));
    for b in trimmed.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(*b));
            }
            other => {
                use std::fmt::Write as _;
                let _ = write!(out, "%{other:02X}");
            }
        }
    }
    out
}

fn basic_creds(registry: &RegistryConfig) -> Option<(&str, &str)> {
    match (&registry.username, &registry.password) {
        (Some(u), Some(p)) if !u.is_empty() => Some((u.as_str(), p.as_str())),
        _ => None,
    }
}

fn registry_domain(registry: &RegistryConfig) -> Result<String, String> {
    let normalized = normalize_registry_host_prefix(&registry.url);
    normalized
        .split('/')
        .next()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "registry.url host is empty".to_string())
}

fn parse_bool_meta(val: Option<&Value>) -> bool {
    match val {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s.trim().eq_ignore_ascii_case("true"),
        _ => false,
    }
}

fn parse_harbor_project_json(item: &Value) -> Option<HarborProjectInfo> {
    let name = item.get("name").and_then(Value::as_str)?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    let project_id = item.get("project_id").and_then(Value::as_i64).unwrap_or(0);
    let public = parse_bool_meta(item.get("metadata").and_then(|m| m.get("public")))
        || parse_bool_meta(item.get("public"));
    let repo_count = item.get("repo_count").and_then(Value::as_u64).unwrap_or(0);
    let registry_id = item
        .get("registry_id")
        .and_then(Value::as_i64)
        .filter(|id| *id > 0);
    let creation_time = item
        .get("creation_time")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    Some(HarborProjectInfo {
        project_id,
        name,
        public,
        repo_count,
        registry_id,
        creation_time,
        mapped_hosts: Vec::new(),
        is_default_fallback: false,
        exists: true,
    })
}

fn reconcile_projects_list(
    raw_items: &[Value],
    mirrors: &[PublicRegistryMirror],
    default_proj_owned: Option<&str>,
    harbor_project_public: bool,
) -> (Vec<HarborProjectInfo>, Vec<String>) {
    let mut mapped_by_project: HashMap<String, Vec<String>> = HashMap::new();
    for m in mirrors {
        let p = m.project.trim();
        let h = m.host.trim();
        if !p.is_empty() && !h.is_empty() {
            mapped_by_project
                .entry(p.to_string())
                .or_default()
                .push(h.to_string());
        }
    }

    let mut projects = Vec::with_capacity(raw_items.len());
    let mut existing_names = HashSet::new();
    for item in raw_items {
        if let Some(mut info) = parse_harbor_project_json(item) {
            if let Some(hosts) = mapped_by_project.get(&info.name) {
                info.mapped_hosts.clone_from(hosts);
            }
            if default_proj_owned == Some(info.name.as_str()) {
                info.is_default_fallback = true;
            }
            existing_names.insert(info.name.clone());
            projects.push(info);
        }
    }

    let mut expected_order: Vec<String> = Vec::new();
    let mut seen_expected = HashSet::new();
    for m in mirrors {
        let p = m.project.trim();
        if !p.is_empty() && seen_expected.insert(p.to_string()) {
            expected_order.push(p.to_string());
        }
    }
    if let Some(dp) = default_proj_owned
        && seen_expected.insert(dp.to_string())
    {
        expected_order.push(dp.to_string());
    }

    let mut missing_mirror_projects = Vec::new();
    for expected_proj in expected_order {
        if !existing_names.contains(&expected_proj) {
            missing_mirror_projects.push(expected_proj.clone());
            projects.push(HarborProjectInfo {
                project_id: 0,
                name: expected_proj.clone(),
                public: harbor_project_public,
                repo_count: 0,
                registry_id: None,
                creation_time: String::new(),
                mapped_hosts: mapped_by_project
                    .get(&expected_proj)
                    .cloned()
                    .unwrap_or_default(),
                is_default_fallback: default_proj_owned == Some(expected_proj.as_str()),
                exists: false,
            });
        }
    }

    projects.sort_by(|a, b| a.name.cmp(&b.name));
    (projects, missing_mirror_projects)
}

/// Fetches all projects from `Harbor` and reconciles them against configured transparent mirror mappings.
///
/// # Errors
///
/// Returns an error if `registry.url` is empty, connecting to the registry fails, or `Harbor`
/// returns an unexpected non-`2xx`/non-`404` status.
pub async fn list_harbor_projects_reconciled(
    registry: &RegistryConfig,
    mirrors: &[PublicRegistryMirror],
    default_fallback_project: Option<&str>,
    mirrors_enabled: bool,
    allow_fallback: bool,
) -> Result<HarborProjectsOverview, String> {
    let domain = registry_domain(registry)?;
    let client = build_http_client(registry.insecure, 4, registry.ca_cert_file.as_deref())?;
    let base_url = probe_registry_base_url(&client, &domain, registry).await?;
    let default_proj_owned = default_fallback_project
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let url = format!("{base_url}/api/v2.0/projects?page=1&page_size=100");
    let mut req = client.get(&url);
    if let Some((u, p)) = basic_creds(registry) {
        req = req.basic_auth(u, Some(p));
    }
    let res = req
        .send()
        .await
        .map_err(|e| format!("Harbor projects request to {url} failed: {e}"))?;
    let status = res.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(HarborProjectsOverview {
            base_url,
            registry_host: domain,
            is_harbor: false,
            mirrors_enabled,
            allow_fallback,
            default_fallback_project: default_proj_owned,
            projects: Vec::new(),
            missing_mirror_projects: Vec::new(),
        });
    }
    if !status.is_success() {
        let body = res.text().await.unwrap_or_default();
        let snippet = body.get(..200.min(body.len())).unwrap_or(&body);
        return Err(format!(
            "Harbor projects API {url} returned HTTP {status}: {snippet}"
        ));
    }

    let raw_items: Vec<Value> = res
        .json()
        .await
        .map_err(|e| format!("failed to parse Harbor projects JSON: {e}"))?;
    let (projects, missing_mirror_projects) = reconcile_projects_list(
        &raw_items,
        mirrors,
        default_proj_owned.as_deref(),
        registry.harbor_project_public,
    );

    Ok(HarborProjectsOverview {
        base_url,
        registry_host: domain,
        is_harbor: true,
        mirrors_enabled,
        allow_fallback,
        default_fallback_project: default_proj_owned,
        projects,
        missing_mirror_projects,
    })
}

/// Lists repositories under a given `Harbor` `project` (optionally filtered by substring `query`).
///
/// # Errors
///
/// Returns an error if `project` is invalid or the `Harbor` API request fails.
pub async fn list_harbor_repositories(
    registry: &RegistryConfig,
    project: &str,
    query: Option<&str>,
) -> Result<Vec<HarborRepositoryInfo>, String> {
    let valid_proj = validate_harbor_project_name(project)?;
    let domain = registry_domain(registry)?;
    let client = build_http_client(registry.insecure, 4, registry.ca_cert_file.as_deref())?;
    let base_url = probe_registry_base_url(&client, &domain, registry).await?;
    let url =
        format!("{base_url}/api/v2.0/projects/{valid_proj}/repositories?page=1&page_size=100");
    let mut req = client.get(&url);
    if let Some((u, p)) = basic_creds(registry) {
        req = req.basic_auth(u, Some(p));
    }
    let res = req
        .send()
        .await
        .map_err(|e| format!("Harbor repositories request to {url} failed: {e}"))?;
    let status = res.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(Vec::new());
    }
    if !status.is_success() {
        let body = res.text().await.unwrap_or_default();
        let snippet = body.get(..200.min(body.len())).unwrap_or(&body);
        return Err(format!(
            "Harbor repositories API {url} returned HTTP {status}: {snippet}"
        ));
    }
    let raw_items: Vec<Value> = res
        .json()
        .await
        .map_err(|e| format!("failed to parse Harbor repositories JSON: {e}"))?;
    let prefix = format!("{valid_proj}/");
    let q_lower = query
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_ascii_lowercase);
    let mut out = Vec::new();
    for item in &raw_items {
        let Some(name) = item.get("name").and_then(Value::as_str) else {
            continue;
        };
        let short_name = name.strip_prefix(&prefix).unwrap_or(name).to_string();
        if let Some(ref q) = q_lower
            && !name.to_ascii_lowercase().contains(q)
            && !short_name.to_ascii_lowercase().contains(q)
        {
            continue;
        }
        out.push(HarborRepositoryInfo {
            id: item.get("id").and_then(Value::as_i64).unwrap_or(0),
            name: name.to_string(),
            short_name,
            artifact_count: item
                .get("artifact_count")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            pull_count: item.get("pull_count").and_then(Value::as_u64).unwrap_or(0),
            update_time: item
                .get("update_time")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        });
    }
    Ok(out)
}

/// Lists artifacts and tags for `repo_short_name` under `Harbor` `project`.
///
/// # Errors
///
/// Returns an error if `project` or `repo_short_name` is invalid or the `Harbor` API request fails.
pub async fn list_harbor_artifacts(
    registry: &RegistryConfig,
    project: &str,
    repo_short_name: &str,
) -> Result<Vec<HarborArtifactInfo>, String> {
    let valid_proj = validate_harbor_project_name(project)?;
    let trimmed_repo = repo_short_name
        .trim()
        .trim_matches('/')
        .strip_prefix(&format!("{valid_proj}/"))
        .unwrap_or_else(|| repo_short_name.trim().trim_matches('/'));
    if trimmed_repo.is_empty() {
        return Err("repository name cannot be empty".to_string());
    }
    let encoded_repo = encode_harbor_repo_segment(trimmed_repo);
    let domain = registry_domain(registry)?;
    let client = build_http_client(registry.insecure, 4, registry.ca_cert_file.as_deref())?;
    let base_url = probe_registry_base_url(&client, &domain, registry).await?;
    let url = format!(
        "{base_url}/api/v2.0/projects/{valid_proj}/repositories/{encoded_repo}/artifacts?with_tag=true&page=1&page_size=100"
    );
    let mut req = client.get(&url);
    if let Some((u, p)) = basic_creds(registry) {
        req = req.basic_auth(u, Some(p));
    }
    let res = req
        .send()
        .await
        .map_err(|e| format!("Harbor artifacts request to {url} failed: {e}"))?;
    let status = res.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(Vec::new());
    }
    if !status.is_success() {
        let body = res.text().await.unwrap_or_default();
        let snippet = body.get(..200.min(body.len())).unwrap_or(&body);
        return Err(format!(
            "Harbor artifacts API {url} returned HTTP {status}: {snippet}"
        ));
    }
    let raw_items: Vec<Value> = res
        .json()
        .await
        .map_err(|e| format!("failed to parse Harbor artifacts JSON: {e}"))?;
    let mut out = Vec::with_capacity(raw_items.len());
    for item in &raw_items {
        let digest = item
            .get("digest")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let size = item.get("size").and_then(Value::as_u64).unwrap_or(0);
        let push_time = item
            .get("push_time")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let media_type = item
            .get("manifest_media_type")
            .or_else(|| item.get("media_type"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let extra = item.get("extra_attrs");
        let architecture = extra
            .and_then(|e| e.get("architecture"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let os = extra
            .and_then(|e| e.get("os"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut tags = Vec::new();
        if let Some(arr) = item.get("tags").and_then(Value::as_array) {
            for t in arr {
                if let Some(tname) = t.get("name").and_then(Value::as_str) {
                    tags.push(HarborTagInfo {
                        name: tname.to_string(),
                        push_time: t
                            .get("push_time")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        pull_time: t
                            .get("pull_time")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                    });
                }
            }
        }
        out.push(HarborArtifactInfo {
            digest,
            size,
            push_time,
            media_type,
            tags,
            architecture,
            os,
        });
    }
    Ok(out)
}

/// Deletes an artifact (or tag reference) from a `Harbor` repository.
///
/// # Errors
///
/// Returns an error if arguments are invalid or `Harbor` rejects the deletion request.
pub async fn delete_harbor_artifact(
    registry: &RegistryConfig,
    project: &str,
    repo_short_name: &str,
    reference: &str,
) -> Result<(), String> {
    let valid_proj = validate_harbor_project_name(project)?;
    let trimmed_repo = repo_short_name
        .trim()
        .trim_matches('/')
        .strip_prefix(&format!("{valid_proj}/"))
        .unwrap_or_else(|| repo_short_name.trim().trim_matches('/'));
    let trimmed_ref = reference.trim();
    if trimmed_repo.is_empty() || trimmed_ref.is_empty() {
        return Err("repository and reference are required for deletion".to_string());
    }
    let encoded_repo = encode_harbor_repo_segment(trimmed_repo);
    let encoded_ref = encode_single_path_segment(trimmed_ref);
    let domain = registry_domain(registry)?;
    let client = build_http_client(registry.insecure, 4, registry.ca_cert_file.as_deref())?;
    let base_url = probe_registry_base_url(&client, &domain, registry).await?;
    let url = format!(
        "{base_url}/api/v2.0/projects/{valid_proj}/repositories/{encoded_repo}/artifacts/{encoded_ref}"
    );
    let mut req = client.delete(&url);
    if let Some((u, p)) = basic_creds(registry) {
        req = req.basic_auth(u, Some(p));
    }
    let res = req
        .send()
        .await
        .map_err(|e| format!("Harbor delete request to {url} failed: {e}"))?;
    let status = res.status();
    if status.is_success() || status == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }
    let body = res.text().await.unwrap_or_default();
    let snippet = body.get(..200.min(body.len())).unwrap_or(&body);
    Err(format!(
        "Harbor delete artifact API {url} returned HTTP {status}: {snippet}"
    ))
}

/// Checks via `OCI` Distribution v2 `HEAD /v2/<repo>/manifests/<ref>` whether an image already exists on `registry`.
///
/// Handles both `Basic` and `401 WWW-Authenticate: Bearer` authentication automatically.
///
/// # Errors
///
/// Returns an error if connecting to `registry` fails or authentication negotiation fails.
pub async fn check_manifest_exists(
    registry: &RegistryConfig,
    target_repository: &str,
    reference: &str,
) -> Result<bool, String> {
    let domain = registry_domain(registry)?;
    let client = build_http_client(registry.insecure, 10, registry.ca_cert_file.as_deref())?;
    let base_url = probe_registry_base_url(&client, &domain, registry).await?;
    check_manifest_exists_with_client(&client, &base_url, registry, target_repository, reference)
        .await
}

pub(super) async fn check_manifest_exists_with_client(
    client: &reqwest::Client,
    base_url: &str,
    registry: &RegistryConfig,
    target_repository: &str,
    reference: &str,
) -> Result<bool, String> {
    const ACCEPT: &str = "application/vnd.docker.distribution.manifest.v2+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.manifest.v1+json, application/vnd.oci.image.index.v1+json";
    let url = format!("{base_url}/v2/{target_repository}/manifests/{reference}");
    let creds = basic_creds(registry);

    let mut req = client.head(&url).header(reqwest::header::ACCEPT, ACCEPT);
    if let Some((u, p)) = creds {
        req = req.basic_auth(u, Some(p));
    }
    let mut res = req
        .send()
        .await
        .map_err(|e| format!("HEAD {url} failed: {e}"))?;

    if res.status() == reqwest::StatusCode::UNAUTHORIZED {
        let www = res
            .headers()
            .get(reqwest::header::WWW_AUTHENTICATE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let (scheme, _) = parse_www_authenticate_bearer(&www);
        if scheme == "bearer" {
            let token = fetch_bearer_token(client, &www, target_repository, "pull", creds).await?;
            res = client
                .head(&url)
                .header(reqwest::header::ACCEPT, ACCEPT)
                .bearer_auth(token)
                .send()
                .await
                .map_err(|e| format!("HEAD {url} after bearer auth failed: {e}"))?;
        }
    }

    if res.status().is_success() {
        return Ok(true);
    }
    if res.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(false);
    }
    Ok(false)
}
