//! Pure-Rust container registry pull, push, and synchronization operations (`Docker` v2 & `OCI`).

use serde_json::Value;
use std::path::Path;
use std::time::Duration;

mod client;
mod image_list;
mod legacy_write;
mod manifest;
mod media;
mod pull;
mod tar;

pub use image_list::{ImageSpec, ImageSyncConfig, SourceAuth};

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
#[derive(Debug, Clone)]
pub struct PushImagesBatchOptions {
    /// Path to the cluster `inventory.yaml` file.
    pub inventory_path: String,
    /// Path to the templated `images.yaml` manifest list.
    pub images_file: String,
    /// Optional override for the directory containing `.tar` archives.
    pub output_dir: Option<String>,
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
    let mut builder = reqwest::Client::builder()
        .danger_accept_invalid_certs(insecure)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(timeout_secs.max(1)));
    if let Some(ca_path) = ca_cert_file.map(str::trim).filter(|s| !s.is_empty()) {
        let pem_bytes = std::fs::read(ca_path)
            .map_err(|e| format!("failed to read CA certificate file {ca_path}: {e}"))?;
        for block in split_pem_certificates(&pem_bytes) {
            let cert = reqwest::Certificate::from_pem(&block)
                .map_err(|e| format!("failed to parse PEM certificate from {ca_path}: {e}"))?;
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

async fn push_images_inner(
    tar_path: &str,
    registry: &RegistryConfig,
    empty_policy: EmptyMatchPolicy,
    http_timeout_secs: u64,
) -> Result<(), String> {
    let images_to_push = tar::extract_images_to_push(tar_path, &registry.url)?;
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
        tar::upload_all_blobs(&session, tar_path, &images_to_push).await?;
    session
        .push_manifests(&images_to_push, &uploaded_files)
        .await
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
    let (registry, doc) = parse_registry_config(inventory_path).await?;
    let (_, sync) = image_list::extract_vars_and_sync(&doc)
        .unwrap_or_else(|_| (Value::Null, ImageSyncConfig::default()));
    let policy = empty_policy_from_registry(&registry);
    crate::log_println!("Starting pure-Rust HTTP API upload for {tar_path}...");
    push_images_inner(tar_path, &registry, policy, sync.http_timeout_secs).await?;
    crate::log_println!("Successfully pushed images to registry.");
    crate::logging::shutdown_logging();
    Ok(())
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
    push_specs_from_output_dir(&specs, &sync.output_dir, &registry, sync.http_timeout_secs).await?;
    crate::log_println!(
        "Successfully pushed {} image tar(s) to registry.",
        specs.len()
    );
    crate::logging::shutdown_logging();
    Ok(())
}

async fn push_specs_from_output_dir(
    specs: &[ImageSpec],
    output_dir: &str,
    registry: &RegistryConfig,
    http_timeout_secs: u64,
) -> Result<(), String> {
    for spec in specs {
        let tar_path = legacy_write::image_tar_path(output_dir, &spec.name);
        ensure_tar_ready_for_batch(&tar_path, &spec.target)?;
        let tar_str = tar_path.to_string_lossy();
        crate::log_println!(
            "Starting pure-Rust HTTP API upload for {tar_str} (target {})...",
            spec.target
        );
        push_images_inner(
            tar_str.as_ref(),
            registry,
            EmptyMatchPolicy::Fail,
            http_timeout_secs,
        )
        .await?;
    }
    Ok(())
}

fn ensure_tar_ready_for_batch(tar_path: &Path, target_ref: &str) -> Result<(), String> {
    if !tar_path.is_file() {
        return Err(format!(
            "missing image tar for batch push: {} (expected target {target_ref})",
            tar_path.display()
        ));
    }
    match legacy_write::existing_tar_matches(tar_path, target_ref, None) {
        Ok(true) => Ok(()),
        Ok(false) => Err(format!(
            "tar {} failed contract check: RepoTags must include exact target '{target_ref}' and config/layers must be present",
            tar_path.display()
        )),
        Err(e) => Err(format!(
            "tar {} contract check error: {e}",
            tar_path.display()
        )),
    }
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
    crate::logging::shutdown_logging();
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
    push_specs_from_output_dir(&specs, &sync.output_dir, &registry, sync.http_timeout_secs).await?;
    crate::log_println!("Successfully synced {} image(s) to registry.", specs.len());
    crate::logging::shutdown_logging();
    Ok(())
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
