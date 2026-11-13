//! Tarball inspection, path-traversal validation, and streaming blob extraction for image push.

use super::client::{PushSession, normalize_registry_host_prefix};
use super::manifest::{ImageToPush, ManifestEntry};
use super::media::{
    MEDIA_META_NAME, MediaMeta, OCI_CONFIG, TempDirGuard, resolve_config_media_type,
    resolve_layer_media_type,
};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write as _};
use std::path::{Component, Path, PathBuf};
use tokio::sync::mpsc;

/// Validates that a tar entry path is relative and contains no `..`, root `/`, or drive prefix.
pub(super) fn validate_tar_entry_path(path: &Path) -> Result<String, String> {
    for comp in path.components() {
        match comp {
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "unsafe tar entry path rejected: {}",
                    path.display()
                ));
            }
            Component::CurDir | Component::Normal(_) => {}
        }
    }
    let raw = path
        .to_str()
        .ok_or_else(|| format!("non-UTF-8 tar entry path: {}", path.display()))?;
    let normalized = raw.strip_prefix("./").unwrap_or(raw);
    if normalized.is_empty() {
        return Err("empty tar entry path".to_string());
    }
    Ok(normalized.to_string())
}

fn parse_repo_tag_for_prefix(
    tag: &str,
    normalized_prefix: &str,
) -> Option<(String, String, String)> {
    if normalized_prefix.is_empty() {
        return None;
    }
    let boundary_prefix = format!("{normalized_prefix}/");
    if !tag.starts_with(&boundary_prefix) {
        return None;
    }
    let (domain, rest) = tag.split_once('/')?;
    if domain.is_empty() || rest.is_empty() {
        return None;
    }
    let (repo_name, tag_name) = match rest.rfind(':') {
        Some(colon_idx) => {
            let repo = rest.get(..colon_idx)?;
            let tag_part = rest.get(colon_idx + 1..)?;
            if repo.is_empty() || tag_part.is_empty() {
                return None;
            }
            (repo.to_string(), tag_part.to_string())
        }
        None => (rest.to_string(), "latest".to_string()),
    };
    Some((domain.to_string(), repo_name, tag_name))
}

pub(super) fn extract_images_to_push(
    tar_path: &str,
    registry_url: &str,
) -> Result<Vec<ImageToPush>, String> {
    let normalized_prefix = normalize_registry_host_prefix(registry_url);
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
        let path_str = {
            let entry_path = entry
                .path()
                .map_err(|e| format!("failed to get entry path: {e}"))?;
            validate_tar_entry_path(&entry_path)?
        };
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
        let clean_config = validate_tar_entry_path(Path::new(&entry.config))?;
        let mut clean_layers = Vec::with_capacity(entry.layers.len());
        for layer in &entry.layers {
            clean_layers.push(validate_tar_entry_path(Path::new(layer))?);
        }
        if let Some(ref tags) = entry.repo_tags {
            for tag in tags {
                if let Some((registry_domain, repo_name, tag_name)) =
                    parse_repo_tag_for_prefix(tag, normalized_prefix)
                {
                    let layer_media_types = clean_layers
                        .iter()
                        .map(|lp| media_meta.as_ref().and_then(|m| m.layers.get(lp).cloned()))
                        .collect();
                    images_to_push.push(ImageToPush {
                        config_path: clean_config.clone(),
                        layers: clean_layers.clone(),
                        registry_domain,
                        repo_name,
                        tag_name,
                        config_media_type: media_meta.as_ref().and_then(|m| m.config.clone()),
                        layer_media_types,
                    });
                }
            }
        }
    }
    Ok(images_to_push)
}

pub(super) async fn upload_all_blobs(
    session: &PushSession<'_>,
    tar_path: &str,
    images_to_push: &[ImageToPush],
) -> Result<(HashMap<String, (String, usize, PathBuf)>, TempDirGuard), String> {
    let mut needed_files = HashSet::new();
    for img in images_to_push {
        needed_files.insert(img.config_path.clone());
        for layer in &img.layers {
            needed_files.insert(layer.clone());
        }
    }
    let temp_guard = TempDirGuard::new("rtkk-push-blobs")?;
    let extracted = extract_needed_blobs(tar_path, &needed_files, temp_guard.path()).await?;
    let mut uploaded_files = HashMap::new();
    let mut uploaded_repo_digests: HashSet<(String, String)> = HashSet::new();
    for (path_str, digest, size, local_path) in extracted {
        crate::log_println!("Processing: {path_str} (size: {size}, digest: {digest})");
        for img in images_to_push {
            if img.config_path == path_str || img.layers.contains(&path_str) {
                let key = (img.repo_name.clone(), digest.clone());
                if uploaded_repo_digests.insert(key) {
                    session
                        .upload_blob_from_path(&img.repo_name, &digest, &local_path, size)
                        .await?;
                }
            }
        }
        uploaded_files.insert(path_str, (digest, size, local_path));
    }
    Ok((uploaded_files, temp_guard))
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
                validate_tar_entry_path(&path)?
            };
            if needed_files_owned.contains(&path_str) {
                let item = stream_entry_to_temp(&mut entry, &path_str, &temp_root_owned)?;
                if tx.blocking_send(item).is_err() {
                    break;
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
    let safe_name = path_str.replace(['/', '\\'], "_");
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
    config_mt: &str,
) -> String {
    let from_img = img
        .layer_media_types
        .get(layer_idx)
        .and_then(|o| o.as_ref())
        .cloned();
    if let Some(mt) = from_img
        && !mt.is_empty()
    {
        return super::media::normalize_layer_media_type(&mt);
    }
    let prefer_oci = config_mt == OCI_CONFIG || config_mt.contains("vnd.oci.image.config");
    resolve_layer_media_type(None, layer_path, blob_path, prefer_oci)
}

pub(super) fn config_media_for_image(img: &ImageToPush) -> String {
    if let Some(ref mt) = img.config_media_type
        && !mt.is_empty()
    {
        return super::media::normalize_config_media_type(mt);
    }
    resolve_config_media_type(None)
}
