use super::manifest::{ImageToPush, ManifestEntry};
use super::media::{
    resolve_config_media_type, resolve_layer_media_type, MediaMeta, MEDIA_META_NAME,
};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;
pub(super) fn extract_images_to_push(
    tar_path: &str,
    registry_url_clean: &str,
) -> Result<Vec<ImageToPush>, String> {
    let file = std::fs::File::open(tar_path)
        .map_err(|e| format!("failed to open tar file {tar_path}: {e}"))?;
    let mut archive = tar::Archive::new(file);
    let mut manifest_bytes = None;
    let mut media_bytes = None;
    for entry_res in archive
        .entries()
        .map_err(|e| format!("failed to read tar entries: {e}"))?
    {
        let mut entry = entry_res.map_err(|e| format!("failed to read tar entry: {e}"))?;
        let path = entry
            .path()
            .map_err(|e| format!("failed to get entry path: {e}"))?;
        let path_str = path.to_str().unwrap_or("").to_string();
        if path_str == "manifest.json" {
            let mut buf = Vec::new();
            entry
                .read_to_end(&mut buf)
                .map_err(|e| format!("failed to read manifest.json: {e}"))?;
            manifest_bytes = Some(buf);
        } else if path_str == MEDIA_META_NAME {
            let mut buf = Vec::new();
            entry
                .read_to_end(&mut buf)
                .map_err(|e| format!("failed to read {MEDIA_META_NAME}: {e}"))?;
            media_bytes = Some(buf);
        }
    }
    let manifest_bytes =
        manifest_bytes.ok_or_else(|| "manifest.json not found in tar file".to_string())?;
    let manifest_entries: Vec<ManifestEntry> = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| format!("failed to parse manifest.json: {e}"))?;
    let media_meta: Option<MediaMeta> = media_bytes
        .as_ref()
        .and_then(|b| serde_json::from_slice(b).ok());
    let mut images_to_push = Vec::new();
    for entry in manifest_entries {
        if let Some(ref tags) = entry.repo_tags {
            for tag in tags {
                if tag.starts_with(registry_url_clean) {
                    let parts: Vec<&str> = tag.splitn(2, '/').collect();
                    if parts.len() == 2 {
                        let registry_domain = parts.first().ok_or("missing domain")?.to_string();
                        let rest = *parts.get(1).ok_or("missing rest path")?;
                        if let Some(colon_idx) = rest.rfind(':') {
                            let repo_name = rest
                                .get(..colon_idx)
                                .ok_or("invalid repo name slice")?
                                .to_string();
                            let tag_name = rest
                                .get(colon_idx + 1..)
                                .ok_or("invalid tag name slice")?
                                .to_string();
                            let layer_media_types = entry
                                .layers
                                .iter()
                                .map(|lp| {
                                    media_meta.as_ref().and_then(|m| m.layers.get(lp).cloned())
                                })
                                .collect();
                            images_to_push.push(ImageToPush {
                                config_path: entry.config.clone(),
                                layers: entry.layers.clone(),
                                registry_domain,
                                repo_name,
                                tag_name,
                                config_media_type: media_meta
                                    .as_ref()
                                    .and_then(|m| m.config.clone()),
                                layer_media_types,
                            });
                        }
                    }
                }
            }
        }
    }
    Ok(images_to_push)
}
pub(super) async fn upload_all_blobs(
    client: &reqwest::Client,
    tar_path: &str,
    base_url: &str,
    images_to_push: &[ImageToPush],
    registry: &super::RegistryConfig,
) -> Result<(HashMap<String, (String, usize, PathBuf)>, PathBuf), String> {
    let mut needed_files = HashSet::new();
    for img in images_to_push {
        needed_files.insert(img.config_path.clone());
        for layer in &img.layers {
            needed_files.insert(layer.clone());
        }
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let temp_root =
        std::env::temp_dir().join(format!("rtkk-push-blobs-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&temp_root)
        .map_err(|e| format!("failed to create push temp dir: {e}"))?;
    let extracted = extract_needed_blobs(tar_path, &needed_files, &temp_root).await?;
    let mut uploaded_files = HashMap::new();
    for (path_str, digest, size, local_path) in extracted {
        crate::log_println!("Processing: {path_str} (size: {size}, digest: {digest})");
        for img in images_to_push {
            if img.config_path == path_str || img.layers.contains(&path_str) {
                super::client::upload_blob_from_path(
                    client,
                    base_url,
                    &img.repo_name,
                    &digest,
                    &local_path,
                    size,
                    registry,
                )
                .await?;
            }
        }
        uploaded_files.insert(path_str, (digest, size, local_path));
    }
    Ok((uploaded_files, temp_root))
}
async fn extract_needed_blobs(
    tar_path: &str,
    needed_files: &HashSet<String>,
    temp_root: &Path,
) -> Result<Vec<(String, String, usize, PathBuf)>, String> {
    let (tx, mut rx) = mpsc::channel(2);
    let tar_path_owned = tar_path.to_string();
    let needed_files_owned = needed_files.clone();
    let temp_root_owned = temp_root.to_path_buf();
    let blocking_handle = tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&tar_path_owned)
            .map_err(|e| format!("failed to open tar file {tar_path_owned}: {e}"))?;
        let mut archive = tar::Archive::new(file);
        for entry_res in archive
            .entries()
            .map_err(|e| format!("failed to read tar entries: {e}"))?
        {
            let mut entry = entry_res.map_err(|e| format!("failed to read tar entry: {e}"))?;
            let path_str = {
                let path = entry
                    .path()
                    .map_err(|e| format!("failed to get entry path: {e}"))?;
                path.to_str().map(String::from)
            };
            if let Some(path_str) = path_str {
                if needed_files_owned.contains(&path_str) {
                    let item = stream_entry_to_temp(&mut entry, &path_str, &temp_root_owned)?;
                    if tx.blocking_send(item).is_err() {
                        break;
                    }
                }
            }
        }
        Ok::<(), String>(())
    });
    let mut out = Vec::new();
    while let Some(item) = rx.recv().await {
        out.push(item);
    }
    blocking_handle
        .await
        .map_err(|e| format!("spawn_blocking join error: {e}"))??;
    Ok(out)
}
fn stream_entry_to_temp<R: Read>(
    entry: &mut tar::Entry<'_, R>,
    path_str: &str,
    temp_root: &Path,
) -> Result<(String, String, usize, PathBuf), String> {
    let safe_name = path_str.replace('/', "_");
    let out_path = temp_root.join(&safe_name);
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create parent for {}: {e}", out_path.display()))?;
    }
    let mut out = std::fs::File::create(&out_path)
        .map_err(|e| format!("failed to create blob file {}: {e}", out_path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut size: usize = 0;
    loop {
        let n = entry
            .read(&mut buf)
            .map_err(|e| format!("failed to read entry {path_str}: {e}"))?;
        if n == 0 {
            break;
        }
        if let Some(slice) = buf.get(..n) {
            hasher.update(slice);
            out.write_all(slice)
                .map_err(|e| format!("failed to write blob {}: {e}", out_path.display()))?;
            size = size.saturating_add(n);
        }
    }
    out.sync_all()
        .map_err(|e| format!("failed to fsync blob: {e}"))?;
    let digest = format!("sha256:{:x}", hasher.finalize());
    Ok((path_str.to_string(), digest, size, out_path))
}
pub(super) fn layer_media_for_image(
    img: &ImageToPush,
    layer_path: &str,
    layer_idx: usize,
    blob_path: Option<&Path>,
) -> String {
    let from_img = img
        .layer_media_types
        .get(layer_idx)
        .and_then(|o| o.as_ref())
        .cloned();
    if let Some(mt) = from_img {
        if !mt.is_empty() {
            return super::media::normalize_layer_media_type(&mt);
        }
    }
    resolve_layer_media_type(None, layer_path, blob_path)
}
pub(super) fn config_media_for_image(img: &ImageToPush) -> String {
    if let Some(ref mt) = img.config_media_type {
        if !mt.is_empty() {
            return super::media::normalize_config_media_type(mt);
        }
    }
    resolve_config_media_type(None)
}
