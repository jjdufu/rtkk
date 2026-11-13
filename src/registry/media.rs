//! Media type constants, `OCI` / `Docker` manifest sniffing, and `RAII` temporary directory guard.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};

pub(super) const MEDIA_META_NAME: &str = "rtkk-media.json";
pub(super) const DOCKER_MANIFEST_V2: &str = "application/vnd.docker.distribution.manifest.v2+json";
pub(super) const OCI_MANIFEST_V1: &str = "application/vnd.oci.image.manifest.v1+json";
pub(super) const DOCKER_LAYER_GZIP: &str = "application/vnd.docker.image.rootfs.diff.tar.gzip";
pub(super) const DOCKER_LAYER_TAR: &str = "application/vnd.docker.image.rootfs.diff.tar";
pub(super) const DOCKER_CONFIG: &str = "application/vnd.docker.container.image.v1+json";
pub(super) const OCI_LAYER_GZIP: &str = "application/vnd.oci.image.layer.v1.tar+gzip";
pub(super) const OCI_LAYER_TAR: &str = "application/vnd.oci.image.layer.v1.tar";
pub(super) const OCI_CONFIG: &str = "application/vnd.oci.image.config.v1+json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct MediaMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<String>,
    #[serde(default)]
    pub layers: HashMap<String, String>,
}

/// `RAII` guard that deletes its temporary directory when dropped.
#[derive(Debug)]
pub(super) struct TempDirGuard(pub(super) PathBuf);

impl TempDirGuard {
    pub(super) fn new(prefix: &str) -> Result<Self, String> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("failed to create temp dir {}: {e}", dir.display()))?;
        Ok(Self(dir))
    }

    #[must_use]
    pub(super) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[must_use]
pub(super) fn sniff_layer_media_type_from_bytes(head: &[u8], prefer_oci: bool) -> &'static str {
    let is_gzip = matches!((head.first(), head.get(1)), (Some(0x1f), Some(0x8b)));
    match (prefer_oci, is_gzip) {
        (true, true) => OCI_LAYER_GZIP,
        (true, false) => OCI_LAYER_TAR,
        (false, true) => DOCKER_LAYER_GZIP,
        (false, false) => DOCKER_LAYER_TAR,
    }
}

pub(super) fn sniff_layer_media_type_from_path(
    path: &Path,
    prefer_oci: bool,
) -> Result<&'static str, String> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("failed to open {} for media sniff: {e}", path.display()))?;
    let mut head = [0u8; 2];
    let n = file
        .read(&mut head)
        .map_err(|e| format!("failed to read head of {}: {e}", path.display()))?;
    let slice = head.get(..n).unwrap_or(&[]);
    Ok(sniff_layer_media_type_from_bytes(slice, prefer_oci))
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

#[must_use]
pub(super) fn is_oci_config_or_layers(config_mt: &str, layer_mts: &[String]) -> bool {
    config_mt == OCI_CONFIG
        || config_mt.contains("vnd.oci.image.config")
        || layer_mts
            .iter()
            .any(|mt| mt.starts_with("application/vnd.oci.image.layer."))
}

#[must_use]
pub(super) fn outer_manifest_media_type(config_mt: &str, layer_mts: &[String]) -> &'static str {
    if is_oci_config_or_layers(config_mt, layer_mts) {
        OCI_MANIFEST_V1
    } else {
        DOCKER_MANIFEST_V2
    }
}

pub(super) fn resolve_layer_media_type(
    meta: Option<&MediaMeta>,
    layer_path: &str,
    blob_path: Option<&Path>,
    prefer_oci: bool,
) -> String {
    if let Some(m) = meta
        && let Some(mt) = m.layers.get(layer_path)
        && !mt.is_empty()
    {
        return normalize_layer_media_type(mt);
    }
    if layer_path.contains("vnd.oci.image.layer.v1.tar") {
        if layer_path.contains("gzip") {
            return OCI_LAYER_GZIP.to_string();
        }
        return OCI_LAYER_TAR.to_string();
    }
    if let Some(p) = blob_path
        && let Ok(mt) = sniff_layer_media_type_from_path(p, prefer_oci)
    {
        return mt.to_string();
    }
    if prefer_oci {
        OCI_LAYER_GZIP.to_string()
    } else {
        DOCKER_LAYER_GZIP.to_string()
    }
}

pub(super) fn resolve_config_media_type(meta: Option<&MediaMeta>) -> String {
    if let Some(m) = meta
        && let Some(ref mt) = m.config
        && !mt.is_empty()
    {
        return normalize_config_media_type(mt);
    }
    DOCKER_CONFIG.to_string()
}
