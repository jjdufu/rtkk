use super::manifest::{Descriptor, ImageToPush, ManifestV2};
use super::tar::{config_media_for_image, layer_media_for_image};
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use tokio::io::AsyncReadExt as _;
pub(super) async fn push_manifests(
    client: &reqwest::Client,
    base_url: &str,
    images_to_push: &[ImageToPush],
    uploaded_files: &HashMap<String, (String, usize, PathBuf)>,
    registry: &super::RegistryConfig,
) -> Result<(), String> {
    for img in images_to_push {
        let (config_digest, config_size, config_local) = uploaded_files
            .get(&img.config_path)
            .ok_or_else(|| format!("missing config file details for {}", img.config_path))?;
        let mut layers_descriptors = Vec::new();
        for (idx, layer_path) in img.layers.iter().enumerate() {
            let (layer_digest, layer_size, layer_local) = uploaded_files
                .get(layer_path)
                .ok_or_else(|| format!("missing layer file details for {layer_path}"))?;
            let media_type =
                layer_media_for_image(img, layer_path, idx, Some(layer_local.as_path()));
            layers_descriptors.push(Descriptor {
                media_type,
                size: *layer_size,
                digest: layer_digest.clone(),
            });
        }
        let config_mt = config_media_for_image(img);
        let _ = config_local;
        let manifest = ManifestV2 {
            schema_version: 2,
            media_type: "application/vnd.docker.distribution.manifest.v2+json".to_string(),
            config: Descriptor {
                media_type: config_mt,
                size: *config_size,
                digest: config_digest.clone(),
            },
            layers: layers_descriptors,
        };
        let repo_name = &img.repo_name;
        let tag_name = &img.tag_name;
        let manifest_task = format!("Push Manifest {repo_name}:{tag_name}");
        crate::logging::start_task("localhost", &manifest_task);
        let manifest_bytes = serde_json::to_vec(&manifest).map_err(|e| {
            crate::logging::finish_task_failed("localhost", &manifest_task, &e.to_string());
            format!("failed to serialize manifest: {e}")
        })?;
        let manifest_url = format!("{base_url}/v2/{repo_name}/manifests/{tag_name}");
        let mut put_request = client
            .put(&manifest_url)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/vnd.docker.distribution.manifest.v2+json",
            )
            .header(reqwest::header::CONTENT_LENGTH, manifest_bytes.len())
            .body(manifest_bytes);
        if let (Some(u), Some(p)) = (&registry.username, &registry.password) {
            put_request = put_request.basic_auth(u, Some(p));
        }
        let put_response = put_request.send().await.map_err(|e| {
            crate::logging::finish_task_failed("localhost", &manifest_task, &e.to_string());
            format!("failed to upload manifest for {repo_name}: {e}")
        })?;
        if !put_response.status().is_success() {
            let status = put_response.status();
            let body = put_response.text().await.unwrap_or_default();
            crate::logging::finish_task_failed(
                "localhost",
                &manifest_task,
                &format!("status {status}"),
            );
            return Err(format!(
                "failed to upload manifest, status: {status}, body: {body}"
            ));
        }
        crate::logging::finish_task_success("localhost", &manifest_task);
    }
    Ok(())
}
pub(super) async fn probe_registry_base_url(
    client: &reqwest::Client,
    domain: &str,
    registry: &super::RegistryConfig,
) -> String {
    let https_url = format!("https://{domain}/v2/");
    let mut req = client.get(&https_url);
    if let (Some(u), Some(p)) = (&registry.username, &registry.password) {
        req = req.basic_auth(u, Some(p));
    }
    match req.send().await {
        Ok(res)
            if res.status().is_success() || res.status() == reqwest::StatusCode::UNAUTHORIZED =>
        {
            return format!("https://{domain}");
        }
        _ => {}
    }
    format!("http://{domain}")
}
async fn auto_create_harbor_project(
    client: &reqwest::Client,
    base_url: &str,
    repo_name: &str,
    registry: &super::RegistryConfig,
    body: &str,
) -> Result<bool, String> {
    if let Some(slash_idx) = repo_name.find('/') {
        let project_name = repo_name
            .get(..slash_idx)
            .ok_or("invalid project name slice")?;
        crate::log_println!(
            "Harbor project '{project_name}' not found. Attempting to create it..."
        );
        let project_payload = serde_json::json!({
            "project_name": project_name,
            "metadata": { "public": "true" }
        });
        let mut created = false;
        let create_urls = vec![
            format!("{base_url}/api/v2.0/projects"),
            format!("{base_url}/api/projects"),
        ];
        for create_url in create_urls {
            let mut create_req = client
                .post(&create_url)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .json(&project_payload);
            if let (Some(u), Some(p)) = (&registry.username, &registry.password) {
                create_req = create_req.basic_auth(u, Some(p));
            }
            if let Ok(create_res) = create_req.send().await {
                let c_status = create_res.status();
                if c_status.is_success() || c_status == reqwest::StatusCode::CONFLICT {
                    created = true;
                    break;
                }
            }
        }
        if created {
            crate::log_println!(
                "Successfully created Harbor project '{project_name}' (or it already exists). Retrying blob upload..."
            );
            Ok(true)
        } else {
            Err(format!(
                "Harbor project '{project_name}' not found and auto-creation failed (body: {body}). Please create the project manually in Harbor."
            ))
        }
    } else {
        Ok(false)
    }
}
async fn upload_blob_stream_from_path(
    client: &reqwest::Client,
    upload_url_with_digest: &str,
    path: &Path,
    size: usize,
    task_name: &str,
    registry: &super::RegistryConfig,
) -> Result<(), String> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|e| format!("failed to open blob {}: {e}", path.display()))?;
    let task_name_clone = task_name.to_string();
    let total = size;
    let stream = futures_util::stream::try_unfold(
        (file, 0usize),
        move |(mut file, mut uploaded)| {
            let task_name_clone = task_name_clone.clone();
            async move {
                let mut buf = vec![0u8; 65536];
                let n = file.read(&mut buf).await?;
                if n == 0 {
                    return Ok::<Option<(bytes::Bytes, _)>, std::io::Error>(None);
                }
                buf.truncate(n);
                uploaded = uploaded.saturating_add(n);
                if let Some(pct) = uploaded.checked_mul(100).and_then(|p| p.checked_div(total)) {
                    let uploaded_mb = crate::utils::usize_to_f64(uploaded) / (1024.0 * 1024.0);
                    let total_mb = crate::utils::usize_to_f64(total) / (1024.0 * 1024.0);
                    let pct_u32 = u32::try_from(pct).unwrap_or(0);
                    crate::logging::update_task(
                        "localhost",
                        &task_name_clone,
                        pct_u32,
                        &format!("{uploaded_mb:.1}MB/{total_mb:.1}MB"),
                    );
                }
                Ok(Some((bytes::Bytes::from(buf), (file, uploaded))))
            }
        },
    );
    let body = reqwest::Body::wrap_stream(stream);
    let mut put_request = client
        .put(upload_url_with_digest)
        .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
        .header(reqwest::header::CONTENT_LENGTH, size)
        .body(body);
    if let (Some(u), Some(p)) = (&registry.username, &registry.password) {
        put_request = put_request.basic_auth(u, Some(p));
    }
    let put_response = put_request.send().await.map_err(|e| {
        crate::logging::finish_task_failed("localhost", task_name, &e.to_string());
        format!("failed to upload blob data for {upload_url_with_digest}: {e}")
    })?;
    if !put_response.status().is_success() {
        let status = put_response.status();
        let body_text = put_response.text().await.unwrap_or_default();
        crate::logging::finish_task_failed("localhost", task_name, &format!("status {status}"));
        return Err(format!(
            "failed to upload blob data, status: {status}, body: {body_text}"
        ));
    }
    crate::logging::finish_task_success("localhost", task_name);
    Ok(())
}
fn short_digest(digest: &str) -> &str {
    if digest.starts_with("sha256:") {
        digest.get(7..19).unwrap_or("")
    } else {
        digest.get(..12.min(digest.len())).unwrap_or("")
    }
}
async fn initiate_blob_upload(
    client: &reqwest::Client,
    base_url: &str,
    repo_name: &str,
    registry: &super::RegistryConfig,
) -> Result<String, String> {
    let init_url = format!("{base_url}/v2/{repo_name}/blobs/uploads/");
    let mut init_request = client.post(&init_url);
    if let (Some(u), Some(p)) = (&registry.username, &registry.password) {
        init_request = init_request.basic_auth(u, Some(p));
    }
    init_request = init_request.header(reqwest::header::CONTENT_LENGTH, 0);
    let mut init_response = init_request
        .send()
        .await
        .map_err(|e| format!("failed to initiate blob upload for {repo_name}: {e}"))?;
    if !init_response.status().is_success()
        && init_response.status() != reqwest::StatusCode::ACCEPTED
    {
        let status = init_response.status();
        let body = init_response.text().await.unwrap_or_default();
        if (status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN)
            && body.contains("project")
            && body.contains("not found")
        {
            if auto_create_harbor_project(client, base_url, repo_name, registry, &body).await? {
                let mut retry_request = client.post(&init_url);
                if let (Some(u), Some(p)) = (&registry.username, &registry.password) {
                    retry_request = retry_request.basic_auth(u, Some(p));
                }
                retry_request = retry_request.header(reqwest::header::CONTENT_LENGTH, 0);
                init_response = retry_request.send().await.map_err(|e| {
                    format!("failed to retry blob upload initiation for {repo_name}: {e}")
                })?;
            } else {
                return Err(format!(
                    "failed to initiate blob upload, status: {status}, body: {body}"
                ));
            }
        } else {
            return Err(format!(
                "failed to initiate blob upload, status: {status}, body: {body}"
            ));
        }
    }
    let location = init_response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|val| val.to_str().ok())
        .ok_or_else(|| "missing Location header in blob upload initiation response".to_string())?;
    if location.starts_with("http://") || location.starts_with("https://") {
        Ok(location.to_string())
    } else if location.starts_with('/') {
        let url_parsed = reqwest::Url::parse(base_url)
            .map_err(|e| format!("failed to parse base_url {base_url}: {e}"))?;
        let scheme = url_parsed.scheme();
        let host = url_parsed
            .host_str()
            .ok_or_else(|| "missing host in base_url".to_string())?;
        let port = url_parsed
            .port()
            .map_or_else(String::new, |p| format!(":{p}"));
        Ok(format!("{scheme}://{host}{port}{location}"))
    } else {
        Ok(format!("{base_url}/v2/{repo_name}/blobs/uploads/{location}"))
    }
}
pub(super) async fn upload_blob_from_path(
    client: &reqwest::Client,
    base_url: &str,
    repo_name: &str,
    digest: &str,
    path: &Path,
    size: usize,
    registry: &super::RegistryConfig,
) -> Result<(), String> {
    let head_url = format!("{base_url}/v2/{repo_name}/blobs/{digest}");
    let mut head_request = client.head(&head_url);
    if let (Some(u), Some(p)) = (&registry.username, &registry.password) {
        head_request = head_request.basic_auth(u, Some(p));
    }
    let head_response = head_request
        .send()
        .await
        .map_err(|e| format!("HEAD request to check blob {digest} failed: {e}"))?;
    if head_response.status() == reqwest::StatusCode::OK {
        let task_name = format!("Push Blob {}", short_digest(digest));
        crate::logging::start_task("localhost", &task_name);
        crate::logging::finish_task_success("localhost", &task_name);
        return Ok(());
    }
    let upload_url = initiate_blob_upload(client, base_url, repo_name, registry).await?;
    let upload_url_with_digest = if upload_url.contains('?') {
        format!("{upload_url}&digest={digest}")
    } else {
        format!("{upload_url}?digest={digest}")
    };
    let task_name = format!("Push Blob {}", short_digest(digest));
    crate::logging::start_task("localhost", &task_name);
    upload_blob_stream_from_path(
        client,
        &upload_url_with_digest,
        path,
        size,
        &task_name,
        registry,
    )
    .await?;
    Ok(())
}
