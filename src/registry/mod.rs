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
pub(crate) use image_list::{ImageSpec, ImageSyncConfig};
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct RegistryConfig {
    pub url: String,
    pub username: Option<String>,
    pub password: Option<String>,
    #[serde(default)]
    pub insecure: bool,
    #[serde(default)]
    pub push_empty_policy: Option<String>,
}
#[derive(Debug, Clone)]
pub(crate) struct PullImagesOptions {
    pub inventory_path: String,
    pub images_file: String,
    pub output_dir: Option<String>,
    pub platform: Option<String>,
    pub force: bool,
    pub insecure: Option<bool>,
}
#[derive(Debug, Clone)]
pub(crate) struct PushImagesBatchOptions {
    pub inventory_path: String,
    pub images_file: String,
    pub output_dir: Option<String>,
}
#[derive(Debug, Clone, Copy)]
enum EmptyMatchPolicy {
    WarnAndOk,
    Fail,
}
fn build_registry_client(insecure: bool, timeout_secs: u64) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .danger_accept_invalid_certs(insecure)
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(timeout_secs.max(1)))
        .build()
        .map_err(|e| format!("failed to build reqwest client: {e}"))
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
    let registry_url_clean = registry.url.trim_end_matches('/');
    let images_to_push = tar::extract_images_to_push(tar_path, registry_url_clean)?;
    if images_to_push.is_empty() {
        let reg_url = &registry.url;
        match empty_policy {
            EmptyMatchPolicy::WarnAndOk => {
                crate::log_println!(
                    "Warning: No images matching registry prefix '{reg_url}' were found in the tarball."
                );
                return Ok(());
            }
            EmptyMatchPolicy::Fail => {
                return Err(format!(
                    "no images matching registry prefix '{reg_url}' in tarball {tar_path}"
                ));
            }
        }
    }
    let http_client = build_registry_client(registry.insecure, http_timeout_secs)?;
    let domain = &images_to_push
        .first()
        .ok_or("no images to push")?
        .registry_domain;
    let base_url = client::probe_registry_base_url(&http_client, domain, registry).await;
    crate::log_println!("Using registry base URL: {base_url}");
    let (uploaded_files, temp_root) =
        tar::upload_all_blobs(&http_client, tar_path, &base_url, &images_to_push, registry).await?;
    let push_res = client::push_manifests(
        &http_client,
        &base_url,
        &images_to_push,
        &uploaded_files,
        registry,
    )
    .await;
    let _ = std::fs::remove_dir_all(&temp_root);
    push_res?;
    Ok(())
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
pub(crate) async fn execute_push_images(
    tar_path: &str,
    inventory_path: &str,
) -> Result<(), String> {
    let (registry, doc) = parse_registry_config(inventory_path).await?;
    let (_, sync) = image_list::extract_vars_and_sync(&doc).unwrap_or_else(|_| {
        (
            Value::Null,
            ImageSyncConfig::default(),
        )
    });
    let policy = empty_policy_from_registry(&registry);
    crate::log_println!("Starting pure-Rust HTTP API upload for {tar_path}...");
    push_images_inner(tar_path, &registry, policy, sync.http_timeout_secs).await?;
    crate::log_println!("Successfully pushed images to registry.");
    crate::logging::shutdown_logging();
    Ok(())
}
pub(crate) async fn execute_push_images_batch(opts: &PushImagesBatchOptions) -> Result<(), String> {
    let doc = parse_inventory_doc(&opts.inventory_path).await?;
    let registry = image_list::registry_from_doc(&doc)?;
    let (vars, mut sync) = image_list::extract_vars_and_sync(&doc)?;
    if let Some(ref d) = opts.output_dir {
        sync.output_dir = d.clone();
    }
    let specs = image_list::load_image_specs(&opts.images_file, &vars)?;
    push_specs_from_output_dir(&specs, &sync.output_dir, &registry, sync.http_timeout_secs)
        .await?;
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
            "Starting pure-Rust HTTP API upload for {} (target {})...",
            tar_str,
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
    match legacy_write::existing_tar_matches(tar_path, target_ref) {
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
pub(crate) async fn execute_pull_images(opts: &PullImagesOptions) -> Result<(), String> {
    let doc = parse_inventory_doc(&opts.inventory_path).await?;
    let registry = image_list::registry_from_doc(&doc)?;
    let (vars, mut sync) = image_list::extract_vars_and_sync(&doc)?;
    if let Some(ref d) = opts.output_dir {
        sync.output_dir = d.clone();
    }
    if let Some(ref p) = opts.platform {
        sync.platform = p.clone();
    }
    if let Some(ins) = opts.insecure {
        sync.insecure = ins;
    }
    let mut registry = registry;
    if sync.insecure {
        registry.insecure = true;
    }
    let specs = image_list::load_image_specs(&opts.images_file, &vars)?;
    pull_all_images(&specs, &sync, &registry, opts.force).await?;
    crate::logging::shutdown_logging();
    Ok(())
}
pub(crate) async fn execute_sync_images(opts: &PullImagesOptions) -> Result<(), String> {
    let doc = parse_inventory_doc(&opts.inventory_path).await?;
    let registry = image_list::registry_from_doc(&doc)?;
    let (vars, mut sync) = image_list::extract_vars_and_sync(&doc)?;
    if let Some(ref d) = opts.output_dir {
        sync.output_dir = d.clone();
    }
    if let Some(ref p) = opts.platform {
        sync.platform = p.clone();
    }
    if let Some(ins) = opts.insecure {
        sync.insecure = ins;
    }
    let mut registry = registry;
    if sync.insecure {
        registry.insecure = true;
    }
    let specs = image_list::load_image_specs(&opts.images_file, &vars)?;
    pull_all_images(&specs, &sync, &registry, opts.force).await?;
    push_specs_from_output_dir(&specs, &sync.output_dir, &registry, sync.http_timeout_secs)
        .await?;
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
    let client = pull::build_pull_client(sync.insecure, sync.http_timeout_secs)?;
    let mut tar_paths = Vec::with_capacity(specs.len());
    for spec in specs {
        let out = legacy_write::image_tar_path(&sync.output_dir, &spec.name);
        let out_str = out.to_string_lossy().into_owned();
        if !force {
            match legacy_write::existing_tar_matches(&out, &spec.target) {
                Ok(true) => {
                    crate::log_println!(
                        "Skip existing tar for {} -> {} (use --force to rewrite)",
                        spec.name,
                        out_str
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
            "Pulling {} -> {} (platform {})",
            spec.source,
            out_str,
            sync.platform
        );
        let pulled = pull::pull_image(&client, spec, sync, registry, &platform).await?;
        let write_res = legacy_write::write_legacy_tar(&pulled, &out);
        let _ = std::fs::remove_dir_all(&pulled.temp_dir);
        write_res?;
        crate::log_println!("Wrote {out_str}");
        tar_paths.push(out_str);
    }
    Ok(tar_paths)
}
