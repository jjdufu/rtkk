//! Pure-Rust container registry pull, push, and synchronization operations (`Docker` v2 & `OCI`).

use serde_json::Value;
use std::path::Path;
use std::time::Duration;

mod client;
mod harbor_api;
mod image_list;
mod legacy_write;
mod manifest;
mod media;
mod mirror;
mod pull;
mod tar;

pub use harbor_api::{
    HarborArtifactInfo, HarborProjectInfo, HarborProjectsOverview, HarborRepositoryInfo,
    HarborTagInfo, check_manifest_exists, delete_harbor_artifact, encode_harbor_repo_segment,
    list_harbor_artifacts, list_harbor_projects_reconciled, list_harbor_repositories,
};
pub use image_list::{ImageSpec, ImageSyncConfig, SourceAuth};
pub use mirror::{
    DEFAULT_FALLBACK_MIRROR_PROJECT, HarborMirrorSyncOutcome, PublicRegistryMirror,
    TransparentMirrorTarget, collect_mirror_project_names, default_public_registry_mirrors,
    ensure_harbor_mirror_projects, extract_image_refs_from_text, parse_mirror_settings_from_vars,
    resolve_transparent_target, validate_harbor_project_name,
};

const fn default_true() -> bool {
    true
}

/// Configuration for the target private container registry (such as `Harbor` or `Docker` Registry v2).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RegistryConfig {
    /// Registry host (and optional port or subpath), e.g. `"harbor.local:8443"`.
    pub url: String,
    /// Optional username for `Basic` or `Bearer`-realm authentication.
    #[serde(default)]
    pub username: Option<String>,
    /// Optional password for `Basic` or `Bearer`-realm authentication.
    #[serde(default)]
    pub password: Option<String>,
    /// Whether to accept invalid or self-signed `TLS` certificates when pushing to this registry.
    #[serde(default)]
    pub insecure: bool,
    /// Policy when no image tags in a tarball match `url` (`"warn"` or `"fail"`).
    #[serde(default)]
    pub push_empty_policy: Option<String>,
    /// Whether auto-created `Harbor` projects should be marked public (`true` by default).
    #[serde(default = "default_true")]
    pub harbor_project_public: bool,
    /// Explicit transport scheme (`None` / `"auto"`, `"https"`, or `"http"`).
    #[serde(default)]
    pub scheme: Option<String>,
    /// Optional path to a custom `PEM` root `CA` certificate bundle for `TLS` verification.
    #[serde(default)]
    pub ca_cert_file: Option<String>,
}

/// CLI/API options for pulling or synchronizing a list of container images.
#[derive(Debug, Clone)]
pub struct PullImagesOptions {
    /// Path to the cluster `inventory.yaml` file.
    pub inventory_path: String,
    /// Path to the templated `images.yaml` manifest list.
    pub images_file: String,
    /// Optional override for the local `.tar` output directory.
    pub output_dir: Option<String>,
    /// Optional override for the target `OCI` platform (`os/arch`).
    pub platform: Option<String>,
    /// Force re-pulling even when a matching `.tar` archive already exists locally.
    pub force: bool,
    /// Optional override for skipping `TLS` verification on upstream source registries.
    pub insecure: Option<bool>,
}

/// CLI/API options for batch-pushing pre-pulled `.tar` archives from an output directory.
#[derive(Debug, Clone, Default)]
pub struct PushImagesBatchOptions {
    /// Path to the cluster `inventory.yaml` file.
    pub inventory_path: String,
    /// Path to the templated `images.yaml` manifest list.
    pub images_file: String,
    /// Optional override for the directory containing `.tar` archives.
    pub output_dir: Option<String>,
    /// Force retagging single-image `.tar` archives to match rendered `images.yaml` targets.
    pub retag: bool,
}

#[derive(Debug, Clone, Copy)]
enum EmptyMatchPolicy {
    WarnAndOk,
    Fail,
}

fn split_pem_certificates(pem_bytes: &[u8]) -> Vec<Vec<u8>> {
    const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
    const END: &str = "-----END CERTIFICATE-----";
    let Ok(text) = std::str::from_utf8(pem_bytes) else {
        return vec![pem_bytes.to_vec()];
    };
    let mut certs = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(BEGIN) {
        let from_begin = rest.get(start..).unwrap_or("");
        if let Some(end_rel) = from_begin.find(END) {
            let block_end = end_rel + END.len();
            if let Some(block) = from_begin.get(..block_end) {
                certs.push(block.as_bytes().to_vec());
            }
            rest = from_begin.get(block_end..).unwrap_or("");
        } else {
            break;
        }
    }
    if certs.is_empty() {
        certs.push(pem_bytes.to_vec());
    }
    certs
}

fn build_http_client(
    insecure: bool,
    timeout_secs: u64,
    ca_cert_file: Option<&str>,
) -> Result<reqwest::Client, String> {
    let effective_timeout = timeout_secs.max(1);
    let connect_secs = effective_timeout.min(15);
    let mut builder = reqwest::Client::builder()
        .danger_accept_invalid_certs(insecure)
        .connect_timeout(Duration::from_secs(connect_secs))
        .timeout(Duration::from_secs(effective_timeout));
    if let Some(ca_path) = ca_cert_file.map(str::trim).filter(|s| !s.is_empty()) {
        let resolved_ca = crate::utils::expand_tilde_path(ca_path);
        let pem_bytes = std::fs::read(&resolved_ca).map_err(|e| {
            format!(
                "failed to read CA certificate file {} (from '{ca_path}'): {e}",
                resolved_ca.display()
            )
        })?;
        for block in split_pem_certificates(&pem_bytes) {
            let cert = reqwest::Certificate::from_pem(&block).map_err(|e| {
                format!(
                    "failed to parse PEM certificate from {}: {e}",
                    resolved_ca.display()
                )
            })?;
            builder = builder.add_root_certificate(cert);
        }
    }
    builder
        .build()
        .map_err(|e| format!("failed to build reqwest HTTP client: {e}"))
}

fn empty_policy_from_registry(registry: &RegistryConfig) -> EmptyMatchPolicy {
    match registry
        .push_empty_policy
        .as_deref()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("fail") => EmptyMatchPolicy::Fail,
        _ => EmptyMatchPolicy::WarnAndOk,
    }
}

async fn push_extracted_images(
    tar_path: &str,
    images_to_push: &[manifest::ImageToPush],
    registry: &RegistryConfig,
    http_timeout_secs: u64,
) -> Result<(), String> {
    let http_client = build_http_client(
        registry.insecure,
        http_timeout_secs,
        registry.ca_cert_file.as_deref(),
    )?;
    let domain = &images_to_push
        .first()
        .ok_or("no images to push")?
        .registry_domain;
    let base_url = client::probe_registry_base_url(&http_client, domain, registry).await?;
    crate::log_println!("Using registry base URL: {base_url}");
    let session = client::PushSession::new(&http_client, base_url, registry);
    let (uploaded_files, _temp_guard) =
        tar::upload_all_blobs(&session, tar_path, images_to_push).await?;
    session
        .push_manifests(images_to_push, &uploaded_files)
        .await
}

async fn push_images_inner(
    tar_path: &str,
    registry: &RegistryConfig,
    empty_policy: EmptyMatchPolicy,
    http_timeout_secs: u64,
    retag: bool,
) -> Result<(), String> {
    let images_to_push = tar::extract_images_to_push(tar_path, &registry.url, retag)?;
    if images_to_push.is_empty() {
        let reg_url = &registry.url;
        return match empty_policy {
            EmptyMatchPolicy::WarnAndOk => {
                crate::log_println!(
                    "Warning: No images matching registry prefix '{reg_url}' were found in the tarball."
                );
                Ok(())
            }
            EmptyMatchPolicy::Fail => Err(format!(
                "no images matching registry prefix '{reg_url}' in tarball {tar_path}"
            )),
        };
    }
    push_extracted_images(tar_path, &images_to_push, registry, http_timeout_secs).await
}

async fn parse_inventory_doc(inventory_path: &str) -> Result<Value, String> {
    let content = tokio::fs::read_to_string(inventory_path)
        .await
        .map_err(|e| format!("failed to read inventory {inventory_path}: {e}"))?;
    serde_yml::from_str(&content).map_err(|e| format!("failed to parse inventory YAML: {e}"))
}

async fn parse_registry_config(inventory_path: &str) -> Result<(RegistryConfig, Value), String> {
    let doc = parse_inventory_doc(inventory_path).await?;
    let registry = image_list::registry_from_doc(&doc)?;
    Ok((registry, doc))
}

/// Pushes all matching images inside a single `Docker`-save `.tar` archive to the configured registry.
///
/// # Errors
///
/// Returns an error if the inventory or tarball cannot be read, authentication fails, or any
/// blob/manifest upload fails.
pub async fn execute_push_images(tar_path: &str, inventory_path: &str) -> Result<(), String> {
    execute_push_images_with_retag(tar_path, inventory_path, false).await
}

/// Pushes images inside a single `Docker`-save `.tar` archive to `inventory_path`'s registry,
/// optionally rewriting mismatched `RepoTags` prefixes when `retag` is `true`.
///
/// # Errors
///
/// Returns an error if the inventory or tarball cannot be read, authentication fails, or any
/// blob/manifest upload fails.
pub async fn execute_push_images_with_retag(
    tar_path: &str,
    inventory_path: &str,
    retag: bool,
) -> Result<(), String> {
    let (registry, doc) = parse_registry_config(inventory_path).await?;
    let (_, sync) = image_list::extract_vars_and_sync(&doc)
        .unwrap_or_else(|_| (Value::Null, ImageSyncConfig::default()));
    let policy = empty_policy_from_registry(&registry);
    crate::log_println!("Starting pure-Rust HTTP API upload for {tar_path}...");
    push_images_inner(tar_path, &registry, policy, sync.http_timeout_secs, retag).await?;
    crate::log_println!("Successfully pushed images to registry.");
    Ok(())
}

/// Local on-disk inspection result for an expected image `.tar` archive.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LocalImageTarStatus {
    /// Full path to `<output_dir>/<name>.tar`.
    pub path: std::path::PathBuf,
    /// Whether the `.tar` file exists on disk.
    pub exists: bool,
    /// File size in bytes (`0` when absent).
    pub size_bytes: u64,
    /// Whether `manifest.json` contains `spec.target`.
    pub repo_tags_matched: bool,
    /// Whether `rtkk-media.json` matches `spec.source`.
    pub has_media_manifest: bool,
}

/// Inspects `<output_dir>/<spec.name>.tar` against `spec.target` and `spec.source`.
#[must_use]
pub fn inspect_local_image_tar(output_dir: &str, spec: &ImageSpec) -> LocalImageTarStatus {
    let path = legacy_write::image_tar_path(output_dir, &spec.name);
    let Ok(meta) = std::fs::metadata(&path) else {
        return LocalImageTarStatus {
            path,
            exists: false,
            size_bytes: 0,
            repo_tags_matched: false,
            has_media_manifest: false,
        };
    };
    if !meta.is_file() {
        return LocalImageTarStatus {
            path,
            exists: false,
            size_bytes: 0,
            repo_tags_matched: false,
            has_media_manifest: false,
        };
    }
    let repo_tags_matched =
        legacy_write::existing_tar_matches(&path, &spec.target, None).unwrap_or(false);
    let has_media_manifest =
        legacy_write::has_valid_media_source(&path, &spec.source).unwrap_or(false);
    LocalImageTarStatus {
        path,
        exists: true,
        size_bytes: meta.len(),
        repo_tags_matched,
        has_media_manifest,
    }
}

/// Read-only connectivity and authentication diagnostic report for a target registry.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RegistryPingReport {
    /// Whether `/v2/` was reachable and authenticated (if credentials were required).
    pub reachable: bool,
    /// Resolved transport scheme (`"https"` or `"http"`).
    pub resolved_scheme: String,
    /// Negotiated authentication mode (`"anonymous"`, `"basic"`, `"bearer"`, or `"failed"`).
    pub auth_mode: String,
    /// Whether Harbor v2 `/api/v2.0/projects` responded.
    pub is_harbor: bool,
    /// Parsed top-level project/namespace from `registry.url`.
    pub project: String,
    /// Whether `project` already exists on Harbor (or `true` for plain OCI v2).
    pub project_exists: bool,
    /// Total probe duration in milliseconds.
    pub latency_ms: u64,
    /// Human-readable status detail.
    pub detail: String,
}

fn split_registry_domain_and_project(raw_url: &str) -> (String, String) {
    let clean = client::normalize_registry_host_prefix(raw_url);
    match clean.split_once('/') {
        Some((dom, rest)) => (
            dom.to_string(),
            rest.split('/').next().unwrap_or("").to_string(),
        ),
        None => (clean.to_string(), String::new()),
    }
}

async fn check_harbor_project(
    http_client: &reqwest::Client,
    base_url: &str,
    project: &str,
    creds: Option<(&str, &str)>,
) -> (bool, bool) {
    if project.is_empty() {
        return (false, true);
    }
    let url = format!("{base_url}/api/v2.0/projects?name={project}");
    let mut req = http_client.get(&url);
    if let Some((u, p)) = creds {
        req = req.basic_auth(u, Some(p));
    }
    let Ok(res) = req.send().await else {
        return (false, false);
    };
    if !res.status().is_success() {
        return (false, false);
    }
    let Ok(items) = res.json::<Vec<Value>>().await else {
        return (true, false);
    };
    let exists = items
        .iter()
        .any(|item| item.get("name").and_then(Value::as_str) == Some(project));
    (true, exists)
}

async fn negotiate_v2_ping_auth(
    http_client: &reqwest::Client,
    v2_url: &str,
    project: &str,
    initial_res: reqwest::Response,
    creds: Option<(&str, &str)>,
) -> (bool, String, String) {
    if initial_res.status().is_success() {
        return (
            true,
            "anonymous".to_string(),
            "OCI /v2/ reachable".to_string(),
        );
    }
    if initial_res.status() != reqwest::StatusCode::UNAUTHORIZED {
        return (
            false,
            "failed".to_string(),
            format!("Unexpected /v2/ status {}", initial_res.status()),
        );
    }
    let www = initial_res
        .headers()
        .get(reqwest::header::WWW_AUTHENTICATE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let (scheme_kind, _params) = client::parse_www_authenticate_bearer(&www);
    if scheme_kind == "bearer" {
        let probe_repo = if project.is_empty() {
            "library/pause".to_string()
        } else {
            format!("{project}/pause")
        };
        match client::fetch_bearer_token(http_client, &www, &probe_repo, "pull", creds).await {
            Ok(tok) => {
                let replay = http_client.get(v2_url).bearer_auth(tok).send().await;
                let ok = replay.is_ok_and(|r| r.status().is_success());
                (
                    ok,
                    "bearer".to_string(),
                    if ok {
                        "Bearer token negotiated".to_string()
                    } else {
                        "Bearer replay rejected".to_string()
                    },
                )
            }
            Err(e) => (false, "bearer".to_string(), e),
        }
    } else if let Some((u, p)) = creds {
        let replay = http_client.get(v2_url).basic_auth(u, Some(p)).send().await;
        let ok = replay.is_ok_and(|r| r.status().is_success());
        (
            ok,
            "basic".to_string(),
            if ok {
                "Basic auth verified".to_string()
            } else {
                "Basic credentials rejected".to_string()
            },
        )
    } else {
        (
            false,
            "unauthorized".to_string(),
            "Registry requires credentials".to_string(),
        )
    }
}

/// Probes `registry` `/v2/` transport, Basic/Bearer challenge-response, and Harbor project status.
pub async fn ping_registry_endpoint(registry: &RegistryConfig) -> RegistryPingReport {
    let started = std::time::Instant::now();
    let elapsed_ms =
        |s: std::time::Instant| u64::try_from(s.elapsed().as_millis()).unwrap_or(u64::MAX);
    let (domain, project) = split_registry_domain_and_project(&registry.url);
    let http_client =
        match build_http_client(registry.insecure, 10, registry.ca_cert_file.as_deref()) {
            Ok(c) => c,
            Err(e) => {
                return RegistryPingReport {
                    reachable: false,
                    resolved_scheme: String::new(),
                    auth_mode: "failed".to_string(),
                    is_harbor: false,
                    project,
                    project_exists: false,
                    latency_ms: elapsed_ms(started),
                    detail: e,
                };
            }
        };

    let base_url = match client::probe_registry_base_url(&http_client, &domain, registry).await {
        Ok(u) => u,
        Err(e) => {
            return RegistryPingReport {
                reachable: false,
                resolved_scheme: String::new(),
                auth_mode: "failed".to_string(),
                is_harbor: false,
                project,
                project_exists: false,
                latency_ms: elapsed_ms(started),
                detail: e,
            };
        }
    };
    let resolved_scheme = if base_url.starts_with("https://") {
        "https".to_string()
    } else {
        "http".to_string()
    };
    let creds = match (&registry.username, &registry.password) {
        (Some(u), Some(p)) if !u.is_empty() => Some((u.as_str(), p.as_str())),
        _ => None,
    };

    let v2_url = format!("{base_url}/v2/");
    let Ok(initial_res) = http_client.get(&v2_url).send().await else {
        return RegistryPingReport {
            reachable: false,
            resolved_scheme,
            auth_mode: "failed".to_string(),
            is_harbor: false,
            project,
            project_exists: false,
            latency_ms: elapsed_ms(started),
            detail: format!("GET {v2_url} failed"),
        };
    };

    let (authed_ok, auth_mode, detail) =
        negotiate_v2_ping_auth(&http_client, &v2_url, &project, initial_res, creds).await;

    let (is_harbor, project_exists) =
        check_harbor_project(&http_client, &base_url, &project, creds).await;
    RegistryPingReport {
        reachable: authed_ok,
        resolved_scheme,
        auth_mode,
        is_harbor,
        project,
        project_exists: if is_harbor { project_exists } else { authed_ok },
        latency_ms: elapsed_ms(started),
        detail,
    }
}

fn filter_image_specs(
    specs: Vec<ImageSpec>,
    wanted_names: &[String],
) -> Result<Vec<ImageSpec>, String> {
    if wanted_names.is_empty() {
        return Ok(specs);
    }
    for name in wanted_names {
        if !crate::utils::is_safe_module_name(name) {
            return Err(format!("unsafe image name filter: '{name}'"));
        }
    }
    let filtered: Vec<ImageSpec> = specs
        .into_iter()
        .filter(|s| wanted_names.iter().any(|w| w == &s.name))
        .collect();
    if filtered.is_empty() {
        return Err("none of the requested image_names matched images.yaml".to_string());
    }
    Ok(filtered)
}

/// Pulls either all images or a named subset (`image_names`) into `<output_dir>/<name>.tar`.
///
/// # Errors
///
/// Returns an error if inventory/image list parsing fails, `image_names` contains unsafe names,
/// or pulling any selected image fails.
pub async fn execute_pull_images_filtered(
    opts: &PullImagesOptions,
    image_names: &[String],
) -> Result<(), String> {
    let doc = parse_inventory_doc(&opts.inventory_path).await?;
    let registry = image_list::registry_from_doc(&doc)?;
    let (vars, mut sync) = image_list::extract_vars_and_sync(&doc)?;
    if let Some(ref d) = opts.output_dir {
        sync.output_dir.clone_from(d);
    }
    if let Some(ref p) = opts.platform {
        sync.platform.clone_from(p);
    }
    if let Some(ins) = opts.insecure {
        sync.insecure = ins;
    }
    let all_specs = image_list::load_image_specs(&opts.images_file, &vars)?;
    let specs = filter_image_specs(all_specs, image_names)?;
    pull_all_images(&specs, &sync, &registry, opts.force).await?;
    Ok(())
}

/// Pushes either all images or a named subset (`image_names`) from `<output_dir>` to `registry`.
///
/// # Errors
///
/// Returns an error if any selected `.tar` archive is missing/invalid or registry upload fails.
pub async fn execute_push_images_filtered(
    opts: &PushImagesBatchOptions,
    image_names: &[String],
) -> Result<(), String> {
    let doc = parse_inventory_doc(&opts.inventory_path).await?;
    let registry = image_list::registry_from_doc(&doc)?;
    let (vars, mut sync) = image_list::extract_vars_and_sync(&doc)?;
    if let Some(ref d) = opts.output_dir {
        sync.output_dir.clone_from(d);
    }
    let all_specs = image_list::load_image_specs(&opts.images_file, &vars)?;
    let specs = filter_image_specs(all_specs, image_names)?;
    push_specs_from_output_dir(
        &specs,
        &sync.output_dir,
        &registry,
        sync.http_timeout_secs,
        opts.retag,
    )
    .await?;
    crate::log_println!(
        "Successfully pushed {} image tar(s) to registry.",
        specs.len()
    );
    Ok(())
}

/// Loads and renders [`ImageSpec`] entries from `images_path` against raw inventory YAML `doc`.
///
/// # Errors
///
/// Returns an error if `vars` is missing from `doc` or if `images_path` fails to read or render.
pub fn load_rendered_image_specs_from_doc(
    doc: &Value,
    images_path: &Path,
) -> Result<Vec<ImageSpec>, String> {
    let (vars, _) = image_list::extract_vars_and_sync(doc)?;
    image_list::load_image_specs(&images_path.to_string_lossy(), &vars)
}

/// Batch-pushes all `.tar` archives listed in an `images.yaml` manifest from the output directory.
///
/// # Errors
///
/// Returns an error if any expected `.tar` archive is missing or invalid, or if any registry
/// upload fails.
pub async fn execute_push_images_batch(opts: &PushImagesBatchOptions) -> Result<(), String> {
    let doc = parse_inventory_doc(&opts.inventory_path).await?;
    let registry = image_list::registry_from_doc(&doc)?;
    let (vars, mut sync) = image_list::extract_vars_and_sync(&doc)?;
    if let Some(ref d) = opts.output_dir {
        sync.output_dir.clone_from(d);
    }
    let specs = image_list::load_image_specs(&opts.images_file, &vars)?;
    push_specs_from_output_dir(
        &specs,
        &sync.output_dir,
        &registry,
        sync.http_timeout_secs,
        opts.retag,
    )
    .await?;
    crate::log_println!(
        "Successfully pushed {} image tar(s) to registry.",
        specs.len()
    );
    Ok(())
}

async fn push_specs_from_output_dir(
    specs: &[ImageSpec],
    output_dir: &str,
    registry: &RegistryConfig,
    http_timeout_secs: u64,
    force_retag: bool,
) -> Result<(), String> {
    for spec in specs {
        let tar_path = legacy_write::image_tar_path(output_dir, &spec.name);
        let tar_str = tar_path.to_string_lossy();
        let images_to_push =
            tar::extract_spec_image_to_push(tar_str.as_ref(), spec, &registry.url, force_retag)?;
        crate::log_println!(
            "Starting pure-Rust HTTP API upload for {tar_str} (target {})...",
            spec.target
        );
        push_extracted_images(
            tar_str.as_ref(),
            &images_to_push,
            registry,
            http_timeout_secs,
        )
        .await?;
    }
    Ok(())
}

/// Pulls all images listed in `opts.images_file` into local `Docker`-save `.tar` archives.
///
/// # Errors
///
/// Returns an error if the inventory or image list cannot be parsed, or if pulling or writing
/// any image archive fails.
pub async fn execute_pull_images(opts: &PullImagesOptions) -> Result<(), String> {
    let doc = parse_inventory_doc(&opts.inventory_path).await?;
    let registry = image_list::registry_from_doc(&doc)?;
    let (vars, mut sync) = image_list::extract_vars_and_sync(&doc)?;
    if let Some(ref d) = opts.output_dir {
        sync.output_dir.clone_from(d);
    }
    if let Some(ref p) = opts.platform {
        sync.platform.clone_from(p);
    }
    if let Some(ins) = opts.insecure {
        sync.insecure = ins;
    }
    let specs = image_list::load_image_specs(&opts.images_file, &vars)?;
    pull_all_images(&specs, &sync, &registry, opts.force).await?;
    Ok(())
}

/// Pulls all images listed in `opts.images_file` and pushes them to the target private registry.
///
/// # Errors
///
/// Returns an error if pulling any upstream image fails or pushing any archive to the target
/// registry fails.
pub async fn execute_sync_images(opts: &PullImagesOptions) -> Result<(), String> {
    let doc = parse_inventory_doc(&opts.inventory_path).await?;
    let registry = image_list::registry_from_doc(&doc)?;
    let (vars, mut sync) = image_list::extract_vars_and_sync(&doc)?;
    if let Some(ref d) = opts.output_dir {
        sync.output_dir.clone_from(d);
    }
    if let Some(ref p) = opts.platform {
        sync.platform.clone_from(p);
    }
    if let Some(ins) = opts.insecure {
        sync.insecure = ins;
    }
    let specs = image_list::load_image_specs(&opts.images_file, &vars)?;
    pull_all_images(&specs, &sync, &registry, opts.force).await?;
    push_specs_from_output_dir(
        &specs,
        &sync.output_dir,
        &registry,
        sync.http_timeout_secs,
        false,
    )
    .await?;
    crate::log_println!("Successfully synced {} image(s) to registry.", specs.len());
    Ok(())
}

/// Execution mode for [`execute_mirror_images`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirrorExecutionMode {
    /// Resolve and preview target paths without performing any network pull or push.
    DryRun,
    /// Pull upstream images and push them into `Harbor` transparent mirror paths.
    Execute {
        /// Re-pull and overwrite even when the target manifest already exists on `Harbor`.
        force: bool,
        /// Persist intermediate `.tar` archives under `<output_dir>/mirror/` when `true`.
        save_tar: bool,
        /// Append successfully imported entries into `vars.extra_images` inside `inventory.yaml`.
        append_extra_images: bool,
    },
}

/// Options for pulling upstream images and importing them directly into transparent `Harbor` mirror paths.
#[derive(Debug, Clone)]
pub struct MirrorImportOptions {
    /// Path to the cluster `inventory.yaml` file.
    pub inventory_path: String,
    /// Raw upstream image references (or newline-separated lists) to import.
    pub images: Vec<String>,
    /// Optional target `OCI` platform override (e.g. `"linux/amd64"`).
    pub platform: Option<String>,
    /// Optional archive directory override when `save_tar` is enabled.
    pub output_dir: Option<String>,
    /// Optional override to skip `TLS` verification when pulling from upstream source registries.
    pub insecure: Option<bool>,
    /// Whether to preview (`DryRun`) or execute the pull/push workflow.
    pub mode: MirrorExecutionMode,
}

/// Summary outcome of a transparent mirror import execution.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct MirrorImportOutcome {
    /// Resolved transparent mirror plan entries.
    pub plans: Vec<TransparentMirrorTarget>,
    /// Target references newly pulled and pushed to `Harbor`.
    pub imported: Vec<String>,
    /// Target references skipped because they already existed on `Harbor` (`!force`).
    pub skipped: Vec<String>,
}

fn collect_unique_raw_images(raw_images: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for entry in raw_images {
        for item in extract_image_refs_from_text(entry) {
            if seen.insert(item.clone()) {
                out.push(item);
            }
        }
    }
    out
}

fn sanitize_mirror_tar_stem(target_repo: &str, tag: &str) -> String {
    let mut stem = String::with_capacity(target_repo.len() + tag.len() + 1);
    for ch in target_repo
        .chars()
        .chain(std::iter::once('-'))
        .chain(tag.chars())
    {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            stem.push(ch);
        } else {
            stem.push('_');
        }
    }
    if stem.is_empty() {
        "image".to_string()
    } else {
        stem
    }
}

/// Plans transparent `Harbor` mirror targets for `raw_images`, optionally probing remote existence.
///
/// # Errors
///
/// Returns an error if `raw_images` yields no image entries or if probing `registry` fails when
/// `check_remote` is `true`.
pub async fn plan_transparent_mirrors(
    registry: &RegistryConfig,
    vars: &Value,
    raw_images: &[String],
    check_remote: bool,
) -> Result<Vec<TransparentMirrorTarget>, String> {
    let parsed_inputs = collect_unique_raw_images(raw_images);
    if parsed_inputs.is_empty() {
        return Err("no image references provided".to_string());
    }
    let (mirrors, default_proj) = parse_mirror_settings_from_vars(vars);
    let mut plans = Vec::with_capacity(parsed_inputs.len());
    for raw in &parsed_inputs {
        match resolve_transparent_target(raw, &registry.url, &mirrors, default_proj.as_deref()) {
            Ok(plan) => plans.push(plan),
            Err(err) => plans.push(TransparentMirrorTarget {
                raw_input: raw.clone(),
                normalized_source: String::new(),
                source_host: String::new(),
                source_repository: String::new(),
                tag: String::new(),
                digest: None,
                target_project: String::new(),
                target_repository: String::new(),
                target_ref: String::new(),
                matched_rule: String::new(),
                used_default_fallback: false,
                project_exists: None,
                already_exists: None,
                error: Some(err),
            }),
        }
    }

    if !check_remote {
        return Ok(plans);
    }

    let normalized = client::normalize_registry_host_prefix(&registry.url);
    let Some(domain) = normalized.split('/').next().filter(|s| !s.is_empty()) else {
        return Ok(plans);
    };
    let Ok(http_client) =
        build_http_client(registry.insecure, 10, registry.ca_cert_file.as_deref())
    else {
        return Ok(plans);
    };
    let Ok(base_url) = client::probe_registry_base_url(&http_client, domain, registry).await else {
        return Ok(plans);
    };

    let existing_projects: Option<std::collections::HashSet<String>> =
        match list_harbor_projects_reconciled(
            registry,
            &mirrors,
            default_proj.as_deref(),
            true,
            false,
        )
        .await
        {
            Ok(ov) if ov.is_harbor => Some(
                ov.projects
                    .into_iter()
                    .filter(|p| p.exists)
                    .map(|p| p.name)
                    .collect(),
            ),
            _ => None,
        };

    for plan in &mut plans {
        if plan.error.is_some() {
            continue;
        }
        if let Some(ref proj_set) = existing_projects {
            plan.project_exists = Some(proj_set.contains(&plan.target_project));
        }
        if let Ok(exists) = harbor_api::check_manifest_exists_with_client(
            &http_client,
            &base_url,
            registry,
            &plan.target_repository,
            &plan.tag,
        )
        .await
        {
            plan.already_exists = Some(exists);
        }
    }
    Ok(plans)
}

fn append_extra_images_to_inventory(
    inventory_path: &str,
    plans: &[TransparentMirrorTarget],
) -> Result<(), String> {
    if plans.is_empty() {
        return Ok(());
    }
    let content = std::fs::read_to_string(inventory_path)
        .map_err(|e| format!("failed to read {inventory_path} for extra_images update: {e}"))?;
    let mut doc: Value = serde_yml::from_str(&content)
        .map_err(|e| format!("failed to parse {inventory_path}: {e}"))?;
    let vars_obj = doc
        .get_mut("vars")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| format!("missing vars mapping in {inventory_path}"))?;
    let entry = vars_obj
        .entry("extra_images".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let arr = entry
        .as_array_mut()
        .ok_or_else(|| "vars.extra_images must be a YAML list".to_string())?;

    let mut existing_targets: std::collections::HashSet<String> = arr
        .iter()
        .filter_map(|item| {
            item.get("target")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    let mut changed = false;
    for plan in plans {
        if existing_targets.insert(plan.target_ref.clone()) {
            let stem = sanitize_mirror_tar_stem(&plan.target_repository, &plan.tag);
            arr.push(serde_json::json!({
                "name": stem,
                "source": plan.normalized_source,
                "target": plan.target_ref,
            }));
            changed = true;
        }
    }
    if !changed {
        return Ok(());
    }
    let bak_path = format!("{inventory_path}.bak");
    let _ = std::fs::write(&bak_path, &content);
    let rendered = serde_yml::to_string(&doc)
        .map_err(|e| format!("failed to serialize updated inventory YAML: {e}"))?;
    let tmp_path = format!("{inventory_path}.tmp.{}", std::process::id());
    std::fs::write(&tmp_path, rendered)
        .map_err(|e| format!("failed to write temporary inventory {tmp_path}: {e}"))?;
    std::fs::rename(&tmp_path, inventory_path)
        .map_err(|e| format!("failed to replace {inventory_path}: {e}"))?;
    crate::log_println!(
        "Appended imported mirror image(s) to vars.extra_images in {inventory_path}"
    );
    Ok(())
}

async fn import_single_mirror_plan(
    pull_client: &reqwest::Client,
    plan: &TransparentMirrorTarget,
    sync: &ImageSyncConfig,
    registry: &RegistryConfig,
    platform: &(String, String, Option<String>),
    mirror_tar_dir: Option<&Path>,
) -> Result<(), String> {
    let stem = sanitize_mirror_tar_stem(&plan.target_repository, &plan.tag);
    let spec = ImageSpec {
        name: stem.clone(),
        source: plan.normalized_source.clone(),
        target: plan.target_ref.clone(),
    };
    crate::log_println!(
        "Mirroring {} -> {} (platform {}, rule: {})",
        spec.source,
        spec.target,
        sync.platform,
        plan.matched_rule
    );
    let pulled = pull::pull_image(pull_client, &spec, sync, registry, platform).await?;
    let temp_tar_guard = if mirror_tar_dir.is_some() {
        None
    } else {
        Some(media::TempDirGuard::new("rtkk-mirror-tar")?)
    };
    let tar_path = if let Some(dir) = mirror_tar_dir {
        dir.join(format!("{stem}.tar"))
    } else if let Some(ref guard) = temp_tar_guard {
        guard.path().join(format!("{stem}.tar"))
    } else {
        return Err("internal error: missing temporary mirror tar directory".to_string());
    };

    legacy_write::write_legacy_tar(&pulled, &tar_path)?;
    pulled.cleanup_temp();

    let tar_str = tar_path.to_string_lossy().into_owned();
    let target_domain = client::normalize_registry_host_prefix(&registry.url)
        .split('/')
        .next()
        .unwrap_or("")
        .to_string();
    let images_to_push = tar::extract_spec_image_to_push(&tar_str, &spec, &target_domain, false)?;
    push_extracted_images(&tar_str, &images_to_push, registry, sync.http_timeout_secs).await?;
    drop(temp_tar_guard);
    crate::log_println!("Imported {} -> {}", spec.source, spec.target);
    Ok(())
}

/// Pulls one or more upstream images and imports them directly into their transparent `Harbor` mirror paths.
///
/// # Errors
///
/// Returns an error if inventory loading fails, any requested image cannot be mapped, upstream pull
/// fails, or pushing to the private registry fails.
pub async fn execute_mirror_images(
    opts: &MirrorImportOptions,
) -> Result<MirrorImportOutcome, String> {
    let doc = parse_inventory_doc(&opts.inventory_path).await?;
    let registry = image_list::registry_from_doc(&doc)?;
    let (vars, mut sync) = image_list::extract_vars_and_sync(&doc)?;
    if let Some(ref d) = opts.output_dir {
        sync.output_dir.clone_from(d);
    }
    if let Some(ref p) = opts.platform {
        sync.platform.clone_from(p);
    }
    if let Some(ins) = opts.insecure {
        sync.insecure = ins;
    }

    let is_dry_run = matches!(opts.mode, MirrorExecutionMode::DryRun);
    let plans = plan_transparent_mirrors(&registry, &vars, &opts.images, !is_dry_run).await?;
    for p in &plans {
        if let Some(ref err) = p.error {
            return Err(format!("invalid mirror image '{}': {err}", p.raw_input));
        }
    }

    let MirrorExecutionMode::Execute {
        force,
        save_tar,
        append_extra_images,
    } = opts.mode
    else {
        for p in &plans {
            crate::log_println!(
                "[dry-run] {} -> {} (rule: {})",
                p.normalized_source,
                p.target_ref,
                p.matched_rule
            );
        }
        return Ok(MirrorImportOutcome {
            plans,
            imported: Vec::new(),
            skipped: Vec::new(),
        });
    };

    let target_projects: Vec<String> = plans.iter().map(|p| p.target_project.clone()).collect();
    ensure_harbor_mirror_projects(&registry, &target_projects, false).await?;

    let platform = pull::platform_tuple(&sync.platform)?;
    let pull_client = build_http_client(
        sync.insecure,
        sync.http_timeout_secs,
        registry.ca_cert_file.as_deref(),
    )?;

    let mirror_tar_dir = save_tar.then(|| Path::new(&sync.output_dir).join("mirror"));
    if let Some(ref dir) = mirror_tar_dir {
        std::fs::create_dir_all(dir).map_err(|e| {
            format!(
                "failed to create mirror tar directory {}: {e}",
                dir.display()
            )
        })?;
    }

    let mut imported = Vec::new();
    let mut skipped = Vec::new();
    let mut succeeded_plans = Vec::new();

    for plan in &plans {
        if !force && plan.already_exists == Some(true) {
            crate::log_println!(
                "Skip existing transparent mirror image {} (already present on registry)",
                plan.target_ref
            );
            skipped.push(plan.target_ref.clone());
            succeeded_plans.push(plan.clone());
            continue;
        }
        import_single_mirror_plan(
            &pull_client,
            plan,
            &sync,
            &registry,
            &platform,
            mirror_tar_dir.as_deref(),
        )
        .await?;
        imported.push(plan.target_ref.clone());
        succeeded_plans.push(plan.clone());
    }

    if append_extra_images {
        append_extra_images_to_inventory(&opts.inventory_path, &succeeded_plans)?;
    }

    Ok(MirrorImportOutcome {
        plans,
        imported,
        skipped,
    })
}

async fn pull_all_images(
    specs: &[ImageSpec],
    sync: &ImageSyncConfig,
    registry: &RegistryConfig,
    force: bool,
) -> Result<Vec<String>, String> {
    std::fs::create_dir_all(&sync.output_dir)
        .map_err(|e| format!("failed to create output directory {}: {e}", sync.output_dir))?;
    let platform = pull::platform_tuple(&sync.platform)?;
    let client = build_http_client(
        sync.insecure,
        sync.http_timeout_secs,
        registry.ca_cert_file.as_deref(),
    )?;
    let mut tar_paths = Vec::with_capacity(specs.len());
    for spec in specs {
        let out = legacy_write::image_tar_path(&sync.output_dir, &spec.name);
        let out_str = out.to_string_lossy().into_owned();
        if !force {
            match legacy_write::existing_tar_matches(&out, &spec.target, Some(&spec.source)) {
                Ok(true) => {
                    crate::log_println!(
                        "Skip existing tar for {} -> {out_str} (use --force to rewrite)",
                        spec.name
                    );
                    tar_paths.push(out_str);
                    continue;
                }
                Ok(false) => {}
                Err(e) => {
                    crate::log_println!(
                        "Existing tar check for {} failed ({e}); will re-pull",
                        spec.name
                    );
                }
            }
        }
        crate::log_println!(
            "Pulling {} -> {out_str} (platform {})",
            spec.source,
            sync.platform
        );
        let pulled = pull::pull_image(&client, spec, sync, registry, &platform).await?;
        legacy_write::write_legacy_tar(&pulled, &out)?;
        pulled.cleanup_temp();
        crate::log_println!("Wrote {out_str}");
        tar_paths.push(out_str);
    }
    Ok(tar_paths)
}

#[cfg(test)]
mod tests {
    use super::client::{normalize_registry_host_prefix, parse_www_authenticate_bearer};
    use super::media::{
        DOCKER_CONFIG, DOCKER_MANIFEST_V2, OCI_CONFIG, OCI_LAYER_GZIP, OCI_MANIFEST_V1,
        TempDirGuard, outer_manifest_media_type,
    };
    use super::pull::parse_image_ref;
    use super::tar::validate_tar_entry_path;
    use std::path::Path;

    #[test]
    fn www_authenticate_parser_preserves_quoted_commas() -> Result<(), String> {
        let header = r#"Bearer realm="https://harbor.local/service/token",service="harbor-registry",scope="repository:library/nginx:pull,push""#;
        let (scheme, params) = parse_www_authenticate_bearer(header);
        if scheme != "bearer" {
            return Err(format!("expected scheme 'bearer', got '{scheme}'"));
        }
        if params.get("scope").map(String::as_str) != Some("repository:library/nginx:pull,push") {
            return Err(format!("unexpected scope: {:?}", params.get("scope")));
        }
        if params.get("realm").map(String::as_str) != Some("https://harbor.local/service/token") {
            return Err(format!("unexpected realm: {:?}", params.get("realm")));
        }
        Ok(())
    }

    #[test]
    fn image_ref_supports_tag_plus_digest_and_default_latest() -> Result<(), String> {
        let digest = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let r1 = parse_image_ref(&format!("nginx:1.25.3@{digest}"))?;
        if r1.repository != "library/nginx"
            || r1.tag.as_deref() != Some("1.25.3")
            || r1.digest.as_deref() != Some(digest)
        {
            return Err(format!("unexpected parsed ref: {r1:?}"));
        }
        let r2 = parse_image_ref("nginx")?;
        if r2.repository != "library/nginx" || r2.tag.as_deref() != Some("latest") {
            return Err(format!("expected library/nginx:latest, got {r2:?}"));
        }
        let r3 = parse_image_ref("10.0.0.1:5000/k8s/pause")?;
        if r3.host != "10.0.0.1:5000"
            || r3.repository != "k8s/pause"
            || r3.tag.as_deref() != Some("latest")
        {
            return Err(format!("unexpected private bare ref: {r3:?}"));
        }
        Ok(())
    }

    #[test]
    fn media_type_prefix_and_tar_guards_work() -> Result<(), String> {
        if normalize_registry_host_prefix("https://harbor.local:8443/") != "harbor.local:8443" {
            return Err("failed to strip https:// and trailing slash".to_string());
        }
        if outer_manifest_media_type(OCI_CONFIG, &[]) != OCI_MANIFEST_V1 {
            return Err("expected OCI manifest v1 for OCI config".to_string());
        }
        if outer_manifest_media_type(DOCKER_CONFIG, &[OCI_LAYER_GZIP.to_string()])
            != OCI_MANIFEST_V1
        {
            return Err("expected OCI manifest v1 for OCI layers".to_string());
        }
        if outer_manifest_media_type(DOCKER_CONFIG, &[]) != DOCKER_MANIFEST_V2 {
            return Err("expected Docker manifest v2 for Docker config".to_string());
        }
        if validate_tar_entry_path(Path::new("../etc/passwd")).is_ok() {
            return Err("expected ParentDir traversal to fail".to_string());
        }
        if validate_tar_entry_path(Path::new("/etc/passwd")).is_ok() {
            return Err("expected RootDir path to fail".to_string());
        }
        let path_copy = {
            let guard = TempDirGuard::new("rtkk-unit-test")?;
            let p = guard.path().to_path_buf();
            if !p.is_dir() {
                return Err("expected temp dir to exist".to_string());
            }
            p
        };
        if path_copy.exists() {
            return Err("expected TempDirGuard drop to remove directory".to_string());
        }
        Ok(())
    }
}
