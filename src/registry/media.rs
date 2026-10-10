use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read as _;
use std::path::Path;
pub(super) const MEDIA_META_NAME: &str = "rtkk-media.json";
pub(super) const DOCKER_LAYER_GZIP: &str =
    "application/vnd.docker.image.rootfs.diff.tar.gzip";
pub(super) const DOCKER_LAYER_TAR: &str = "application/vnd.docker.image.rootfs.diff.tar";
pub(super) const DOCKER_CONFIG: &str = "application/vnd.docker.container.image.v1+json";
pub(super) const OCI_LAYER_GZIP: &str = "application/vnd.oci.image.layer.v1.tar+gzip";
pub(super) const OCI_LAYER_TAR: &str = "application/vnd.oci.image.layer.v1.tar";
pub(super) const OCI_CONFIG: &str = "application/vnd.oci.image.config.v1+json";
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct MediaMeta {
    #[serde(default)]
    pub config: Option<String>,
    #[serde(default)]
    pub layers: HashMap<String, String>,
}
#[must_use]
pub(super) fn sniff_layer_media_type_from_bytes(head: &[u8]) -> &'static str {
    match (head.first(), head.get(1)) {
        (Some(0x1f), Some(0x8b)) => DOCKER_LAYER_GZIP,
        _ => DOCKER_LAYER_TAR,
    }
}
pub(super) fn sniff_layer_media_type_from_path(path: &Path) -> Result<&'static str, String> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("failed to open {} for media sniff: {e}", path.display()))?;
    let mut head = [0u8; 2];
    let n = file
        .read(&mut head)
        .map_err(|e| format!("failed to read head of {}: {e}", path.display()))?;
    let slice = head.get(..n).unwrap_or(&[]);
    Ok(sniff_layer_media_type_from_bytes(slice))
}
#[must_use]
pub(super) fn normalize_layer_media_type(mt: &str) -> String {
    let lower = mt.to_ascii_lowercase();
    if lower.contains("zstd") {
        return mt.to_string();
    }
    if lower.contains("gzip") || lower.ends_with("+gzip") {
        if lower.contains("oci") {
            return OCI_LAYER_GZIP.to_string();
        }
        return DOCKER_LAYER_GZIP.to_string();
    }
    if lower.contains("oci") {
        return OCI_LAYER_TAR.to_string();
    }
    if lower.is_empty() {
        return DOCKER_LAYER_GZIP.to_string();
    }
    mt.to_string()
}
#[must_use]
pub(super) fn normalize_config_media_type(mt: &str) -> String {
    let lower = mt.to_ascii_lowercase();
    if lower.contains("oci") {
        OCI_CONFIG.to_string()
    } else if lower.is_empty() {
        DOCKER_CONFIG.to_string()
    } else {
        mt.to_string()
    }
}
pub(super) fn resolve_layer_media_type(
    meta: Option<&MediaMeta>,
    layer_path: &str,
    blob_path: Option<&Path>,
) -> String {
    if let Some(m) = meta {
        if let Some(mt) = m.layers.get(layer_path) {
            if !mt.is_empty() {
                return normalize_layer_media_type(mt);
            }
        }
    }
    if let Some(p) = blob_path {
        if let Ok(mt) = sniff_layer_media_type_from_path(p) {
            return mt.to_string();
        }
    }
    DOCKER_LAYER_GZIP.to_string()
}
pub(super) fn resolve_config_media_type(meta: Option<&MediaMeta>) -> String {
    if let Some(m) = meta {
        if let Some(ref mt) = m.config {
            if !mt.is_empty() {
                return normalize_config_media_type(mt);
            }
        }
    }
    DOCKER_CONFIG.to_string()
}
