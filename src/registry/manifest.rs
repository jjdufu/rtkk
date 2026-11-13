//! Container image manifest, descriptor, and index data structures for `Docker` and `OCI`.

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug)]
pub(super) struct ManifestEntry {
    #[serde(rename = "Config")]
    pub config: String,
    #[serde(rename = "RepoTags")]
    pub repo_tags: Option<Vec<String>>,
    #[serde(rename = "Layers")]
    pub layers: Vec<String>,
}

#[derive(Serialize, Debug)]
pub(super) struct ManifestV2 {
    #[serde(rename = "schemaVersion")]
    pub schema_version: i32,
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub config: Descriptor,
    pub layers: Vec<Descriptor>,
}

#[derive(Serialize, Debug)]
pub(super) struct Descriptor {
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub size: usize,
    pub digest: String,
}

#[derive(Debug, Clone)]
pub(super) struct ImageToPush {
    pub config_path: String,
    pub layers: Vec<String>,
    pub registry_domain: String,
    pub repo_name: String,
    pub tag_name: String,
    pub config_media_type: Option<String>,
    pub layer_media_types: Vec<Option<String>>,
}

#[derive(Deserialize, Debug, Clone)]
pub(super) struct RemotePlatform {
    pub architecture: Option<String>,
    pub os: Option<String>,
    pub variant: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub(super) struct RemoteDescriptor {
    pub digest: String,
    #[serde(default, rename = "mediaType")]
    pub media_type: Option<String>,
    pub platform: Option<RemotePlatform>,
}

#[derive(Deserialize, Debug)]
pub(super) struct RemoteIndex {
    pub manifests: Option<Vec<RemoteDescriptor>>,
}

#[derive(Deserialize, Debug)]
pub(super) struct RemoteImageManifest {
    pub config: RemoteDescriptor,
    pub layers: Vec<RemoteDescriptor>,
}
