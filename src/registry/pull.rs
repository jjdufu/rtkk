//! Registry `HTTP` pull client with `SHA256` manifest verification and concurrent layer downloads.

use super::RegistryConfig;
use super::client::{
    fetch_bearer_token, normalize_registry_host_prefix, parse_www_authenticate_bearer,
};
use super::image_list::{ImageSpec, ImageSyncConfig, parse_platform};
use super::manifest::{RemoteDescriptor, RemoteImageManifest, RemoteIndex};
use super::media::TempDirGuard;
use futures_util::StreamExt as _;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt as _;
use tokio::sync::{RwLock, Semaphore};
use tokio::task::JoinSet;

const PROBE_TIMEOUT_SECS: u64 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ImageRef {
    pub host: String,
    pub repository: String,
    pub tag: Option<String>,
    pub digest: Option<String>,
}

impl ImageRef {
    pub(super) fn api_host(&self) -> &str {
        if self.host == "docker.io" {
            "registry-1.docker.io"
        } else {
            self.host.as_str()
        }
    }
}

#[derive(Debug)]
pub(super) struct PulledLayer {
    pub digest: String,
    pub path: PathBuf,
    pub media_type: String,
}

#[derive(Debug)]
pub(super) struct PulledImage {
    pub source_ref: String,
    pub target_ref: String,
    pub config_digest: String,
    pub config_path: PathBuf,
    pub config_media_type: String,
    pub layers: Vec<PulledLayer>,
    pub temp_dir: TempDirGuard,
}

impl PulledImage {
    pub(super) fn cleanup_temp(self) {
        drop(self.temp_dir);
    }
}

pub(super) fn normalize_sha256_digest(digest: &str) -> Result<String, String> {
    let trimmed = digest.trim();
    let hex = trimmed
        .strip_prefix("sha256:")
        .or_else(|| {
            trimmed
                .get(..7)
                .filter(|p| p.eq_ignore_ascii_case("sha256:"))
                .and_then(|_| trimmed.get(7..))
        })
        .ok_or_else(|| {
            format!("unsupported or malformed digest (expected sha256:<hex>): {digest}")
        })?
        .to_ascii_lowercase();
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("invalid sha256 hex digest: {digest}"));
    }
    Ok(format!("sha256:{hex}"))
}

pub(super) fn verify_bytes_sha256(bytes: &[u8], expected_digest: &str) -> Result<(), String> {
    let expected = normalize_sha256_digest(expected_digest)?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let got = format!("sha256:{:x}", hasher.finalize());
    if got != expected {
        return Err(format!(
            "manifest digest mismatch: expected {expected}, got {got}"
        ));
    }
    Ok(())
}

pub(super) fn parse_image_ref(raw: &str) -> Result<ImageRef, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Err("empty image reference".to_string());
    }
    let (name_part, digest) = if let Some(idx) = s.find('@') {
        let name = s.get(..idx).unwrap_or("").trim();
        let dig_raw = s.get(idx + 1..).unwrap_or("").trim();
        if name.is_empty() || dig_raw.is_empty() {
            return Err(format!("invalid digest reference: {raw}"));
        }
        let normalized_dig = normalize_sha256_digest(dig_raw)?;
        (name, Some(normalized_dig))
    } else {
        (s, None)
    };
    let (without_tag, parsed_tag) = split_name_and_tag(name_part)?;
    let (host, repository) = split_host_and_repo(&without_tag);
    if repository.is_empty() {
        return Err(format!("missing repository in reference: {raw}"));
    }
    let tag = if digest.is_none() && parsed_tag.is_none() {
        Some("latest".to_string())
    } else {
        parsed_tag
    };
    Ok(ImageRef {
        host,
        repository,
        tag,
        digest,
    })
}

fn split_name_and_tag(name_part: &str) -> Result<(String, Option<String>), String> {
    if let Some(slash) = name_part.rfind('/') {
        let after = name_part.get(slash + 1..).unwrap_or("");
        if let Some(colon) = after.rfind(':') {
            let name = name_part
                .get(..slash + 1 + colon)
                .ok_or_else(|| format!("invalid name: {name_part}"))?;
            let tag = after
                .get(colon + 1..)
                .ok_or_else(|| format!("invalid tag: {name_part}"))?;
            if tag.is_empty() {
                return Err(format!("empty tag in reference: {name_part}"));
            }
            return Ok((name.to_string(), Some(tag.to_string())));
        }
        return Ok((name_part.to_string(), None));
    }
    if let Some(colon) = name_part.rfind(':') {
        let left = name_part.get(..colon).unwrap_or("");
        let right = name_part.get(colon + 1..).unwrap_or("");
        if right.chars().all(|c| c.is_ascii_digit()) && (left.contains('.') || left == "localhost")
        {
            return Ok((name_part.to_string(), None));
        }
        if right.is_empty() {
            return Err(format!("empty tag in reference: {name_part}"));
        }
        return Ok((left.to_string(), Some(right.to_string())));
    }
    Ok((name_part.to_string(), None))
}

fn split_host_and_repo(name: &str) -> (String, String) {
    let parts: Vec<&str> = name.splitn(2, '/').collect();
    let first = *parts.first().unwrap_or(&"");
    let has_host =
        first.contains('.') || first.contains(':') || first == "localhost" || first == "docker.io";
    if has_host {
        let host = first.to_string();
        let rest = parts.get(1).copied().unwrap_or("").to_string();
        let repository =
            if host == "docker.io" || host == "registry-1.docker.io" || host == "index.docker.io" {
                normalize_docker_hub_repo(&rest)
            } else {
                rest
            };
        (normalize_host_name(&host), repository)
    } else {
        let repository = normalize_docker_hub_repo(name);
        ("docker.io".to_string(), repository)
    }
}

fn normalize_host_name(host: &str) -> String {
    if host == "registry-1.docker.io" || host == "index.docker.io" {
        "docker.io".to_string()
    } else {
        host.to_string()
    }
}

fn normalize_docker_hub_repo(repo: &str) -> String {
    if repo.is_empty() {
        return String::new();
    }
    if repo.contains('/') {
        repo.to_string()
    } else {
        format!("library/{repo}")
    }
}

struct PullWorker {
    client: reqwest::Client,
    base_url: String,
    repository: String,
    creds: Option<(String, String)>,
    authorization: RwLock<Option<String>>,
    retries: u32,
}

impl PullWorker {
    async fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let guard = self.authorization.read().await;
        if let Some(ref auth) = *guard {
            req.header(reqwest::header::AUTHORIZATION, auth.clone())
        } else {
            req
        }
    }

    async fn refresh_auth_from_www(&self, www: &str) -> Result<(), String> {
        let (scheme, _) = parse_www_authenticate_bearer(www);
        if scheme == "basic" {
            if let Some((ref u, ref p)) = self.creds {
                let raw = format!("{u}:{p}");
                let header = format!("Basic {}", crate::utils::base64_encode(raw.as_bytes()));
                let mut guard = self.authorization.write().await;
                *guard = Some(header);
                return Ok(());
            }
            return Err(format!(
                "registry requires Basic auth but no credentials configured for {}",
                self.base_url
            ));
        }
        if scheme == "bearer" {
            let creds_ref = self.creds.as_ref().map(|(u, p)| (u.as_str(), p.as_str()));
            let token =
                fetch_bearer_token(&self.client, www, &self.repository, "pull", creds_ref).await?;
            let mut guard = self.authorization.write().await;
            *guard = Some(format!("Bearer {token}"));
            return Ok(());
        }
        Ok(())
    }

    async fn ensure_auth(&self) -> Result<(), String> {
        let url = format!("{}/v2/", self.base_url);
        let req = self.apply_auth(self.client.get(&url)).await;
        let res = req
            .send()
            .await
            .map_err(|e| format!("GET {url} failed: {e}"))?;
        if res.status() != reqwest::StatusCode::UNAUTHORIZED {
            return Ok(());
        }
        let www = res
            .headers()
            .get(reqwest::header::WWW_AUTHENTICATE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        self.refresh_auth_from_www(&www).await
    }

    async fn fetch_manifest(&self, reference: &str) -> Result<Vec<u8>, String> {
        let url = format!(
            "{}/v2/{}/manifests/{reference}",
            self.base_url, self.repository
        );
        let attempts = self.retries.max(1);
        let mut last_err = String::new();
        for attempt in 1..=attempts {
            let mut res_opt = self.send_manifest_request(&url).await;
            if let Ok(ref res) = res_opt
                && res.status() == reqwest::StatusCode::UNAUTHORIZED
            {
                let www = res
                    .headers()
                    .get(reqwest::header::WWW_AUTHENTICATE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                if self.refresh_auth_from_www(&www).await.is_ok() {
                    res_opt = self.send_manifest_request(&url).await;
                }
            }
            match res_opt {
                Ok(res) if res.status().is_success() => {
                    return res
                        .bytes()
                        .await
                        .map(|b| b.to_vec())
                        .map_err(|e| format!("failed to read manifest body: {e}"));
                }
                Ok(res) => {
                    let status = res.status();
                    let body = res.text().await.unwrap_or_default();
                    last_err = format!(
                        "GET manifest {url} status {status}: {}",
                        truncate_body(&body)
                    );
                }
                Err(e) => {
                    last_err = format!("GET manifest {url} failed: {e}");
                }
            }
            if attempt < attempts {
                let delay_ms = 500u64.saturating_mul(u64::from(attempt));
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
        }
        Err(last_err)
    }

    async fn send_manifest_request(&self, url: &str) -> Result<reqwest::Response, reqwest::Error> {
        let req = self
            .client
            .get(url)
            .header(reqwest::header::ACCEPT, accept_header());
        self.apply_auth(req).await.send().await
    }

    async fn download_blob_to_file(
        &self,
        digest: &str,
        dest: &Path,
        label: &str,
    ) -> Result<(), String> {
        let url = format!("{}/v2/{}/blobs/{digest}", self.base_url, self.repository);
        let progress_label = format!("Pull {label} {}", short_digest(digest));
        crate::logging::start_task("localhost", &progress_label);
        let attempts = self.retries.max(1);
        let mut last_err = String::new();
        for attempt in 1..=attempts {
            match self
                .try_download_blob_with_auth(&url, dest, digest, &progress_label)
                .await
            {
                Ok(()) => {
                    crate::logging::finish_task_success("localhost", &progress_label);
                    return Ok(());
                }
                Err(e) => {
                    last_err = e;
                    let _ = std::fs::remove_file(dest);
                    if attempt < attempts {
                        let delay_ms = 500u64.saturating_mul(u64::from(attempt));
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    }
                }
            }
        }
        crate::logging::finish_task_failed("localhost", &progress_label, &last_err);
        Err(last_err)
    }

    async fn try_download_blob_with_auth(
        &self,
        url: &str,
        dest: &Path,
        expected_digest: &str,
        progress_label: &str,
    ) -> Result<(), String> {
        let mut res = self
            .apply_auth(self.client.get(url))
            .await
            .send()
            .await
            .map_err(|e| format!("GET blob {url} failed: {e}"))?;
        if res.status() == reqwest::StatusCode::UNAUTHORIZED {
            let www = res
                .headers()
                .get(reqwest::header::WWW_AUTHENTICATE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            self.refresh_auth_from_www(&www).await?;
            res = self
                .apply_auth(self.client.get(url))
                .await
                .send()
                .await
                .map_err(|e| format!("GET blob {url} after auth refresh failed: {e}"))?;
        }
        stream_and_verify_blob(res, url, dest, expected_digest, progress_label).await
    }
}

async fn stream_and_verify_blob(
    res: reqwest::Response,
    url: &str,
    dest: &Path,
    expected_digest: &str,
    progress_label: &str,
) -> Result<(), String> {
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(format!(
            "GET blob {url} status {status}: {}",
            truncate_body(&body)
        ));
    }
    let expected = normalize_sha256_digest(expected_digest)?;
    let total = res.content_length().unwrap_or(0);
    let mut stream = res.bytes_stream();
    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| format!("failed to create {}: {e}", dest.display()))?;
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("blob stream error: {e}"))?;
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("write blob failed: {e}"))?;
        downloaded = downloaded.saturating_add(u64::try_from(chunk.len()).unwrap_or(0));
        let _ = crate::logging::emit_event(crate::logging::ExecutionEvent::ArtifactProgress {
            item: progress_label.to_string(),
            bytes_done: downloaded,
            bytes_total: (total > 0).then_some(total),
            phase: "pull_blob".to_string(),
        });
        if let Some(pct_u64) = downloaded.saturating_mul(100).checked_div(total) {
            let pct = u32::try_from(pct_u64).unwrap_or(0);
            let downloaded_mb = crate::utils::u64_to_f64(downloaded) / (1024.0 * 1024.0);
            let total_mb = crate::utils::u64_to_f64(total) / (1024.0 * 1024.0);
            crate::logging::update_task(
                "localhost",
                progress_label,
                pct.min(100),
                &format!("{downloaded_mb:.1}MB/{total_mb:.1}MB"),
            );
        }
    }
    file.flush()
        .await
        .map_err(|e| format!("flush blob failed: {e}"))?;
    let got = format!("sha256:{:x}", hasher.finalize());
    if got != expected {
        return Err(format!(
            "digest mismatch for blob: expected {expected}, got {got}"
        ));
    }
    Ok(())
}

pub(super) async fn pull_image(
    client: &reqwest::Client,
    spec: &ImageSpec,
    sync: &ImageSyncConfig,
    registry: &RegistryConfig,
    platform: &(String, String, Option<String>),
) -> Result<PulledImage, String> {
    let image_ref = parse_image_ref(&spec.source)?;
    let (base_url, pull_repo) =
        resolve_source_mirror_endpoint(client, &image_ref, sync, registry).await?;
    let creds = resolve_source_creds(&image_ref.host, sync, registry);
    let worker = Arc::new(PullWorker {
        client: client.clone(),
        base_url,
        repository: pull_repo,
        creds,
        authorization: RwLock::new(None),
        retries: sync.retries,
    });
    worker.ensure_auth().await?;
    let reference = image_ref
        .digest
        .clone()
        .or_else(|| image_ref.tag.clone())
        .ok_or_else(|| "missing tag/digest".to_string())?;
    let manifest_bytes = worker.fetch_manifest(&reference).await?;
    if let Some(ref pinned_digest) = image_ref.digest {
        verify_bytes_sha256(&manifest_bytes, pinned_digest)?;
    }
    let image_manifest = resolve_image_manifest(&worker, &manifest_bytes, platform).await?;
    let temp_guard = TempDirGuard::new(&format!("rtkk-pull-{}", spec.name))?;
    download_blobs(
        &worker,
        &image_manifest,
        temp_guard,
        &spec.source,
        &spec.target,
        sync.layer_concurrency,
    )
    .await
}

async fn resolve_image_manifest(
    worker: &PullWorker,
    manifest_bytes: &[u8],
    platform: &(String, String, Option<String>),
) -> Result<RemoteImageManifest, String> {
    if sniff_manifest_kind(manifest_bytes) == ManifestKind::Index {
        let index: RemoteIndex = serde_json::from_slice(manifest_bytes)
            .map_err(|e| format!("failed to parse manifest list/index: {e}"))?;
        let desc = pick_platform(&index, platform)?;
        let child = worker.fetch_manifest(&desc.digest).await?;
        verify_bytes_sha256(&child, &desc.digest)?;
        serde_json::from_slice(&child)
            .map_err(|e| format!("failed to parse image manifest {}: {e}", desc.digest))
    } else {
        serde_json::from_slice(manifest_bytes)
            .map_err(|e| format!("failed to parse image manifest: {e}"))
    }
}

#[derive(PartialEq, Eq)]
enum ManifestKind {
    Index,
    Image,
}

fn sniff_manifest_kind(bytes: &[u8]) -> ManifestKind {
    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(bytes) {
        if v.get("manifests").is_some() {
            return ManifestKind::Index;
        }
        if let Some(mt) = v.get("mediaType").and_then(serde_json::Value::as_str)
            && (mt.contains("manifest.list") || mt.contains("image.index"))
        {
            return ManifestKind::Index;
        }
    }
    ManifestKind::Image
}

fn pick_platform(
    index: &RemoteIndex,
    want: &(String, String, Option<String>),
) -> Result<RemoteDescriptor, String> {
    let manifests = index
        .manifests
        .as_ref()
        .ok_or_else(|| "manifest list has no manifests".to_string())?;
    let (want_os, want_arch, want_variant) = want;
    for m in manifests {
        let Some(ref p) = m.platform else {
            continue;
        };
        let os = p.os.as_deref().unwrap_or("");
        let arch = p.architecture.as_deref().unwrap_or("");
        if os != want_os.as_str() || arch != want_arch.as_str() {
            continue;
        }
        match (want_variant.as_ref(), p.variant.as_ref()) {
            (None, _) => return Ok(m.clone()),
            (Some(wv), Some(pv)) if wv == pv => return Ok(m.clone()),
            (Some(_), None | Some(_)) => {}
        }
    }
    let variant_suffix = want_variant
        .as_ref()
        .map_or_else(String::new, |v| format!("/{v}"));
    Err(format!(
        "no matching platform {want_os}/{want_arch}{variant_suffix} in manifest list"
    ))
}

fn resolve_source_creds(
    host: &str,
    sync: &ImageSyncConfig,
    registry: &RegistryConfig,
) -> Option<(String, String)> {
    for (cfg_host, auth) in &sync.source_auths {
        let norm_cfg = normalize_registry_host_prefix(cfg_host)
            .split('/')
            .next()
            .unwrap_or("");
        if (cfg_host == host || norm_cfg == host)
            && let (Some(u), Some(p)) = (&auth.username, &auth.password)
            && !u.is_empty()
        {
            return Some((u.clone(), p.clone()));
        }
    }
    let reg_host = registry_host_from_url(&registry.url);
    if !reg_host.is_empty()
        && host == reg_host.as_str()
        && let (Some(u), Some(p)) = (&registry.username, &registry.password)
        && !u.is_empty()
    {
        return Some((u.clone(), p.clone()));
    }
    None
}

fn registry_host_from_url(url: &str) -> String {
    let clean = normalize_registry_host_prefix(url);
    clean.split('/').next().unwrap_or(clean).to_string()
}

async fn resolve_registry_base_url(
    client: &reqwest::Client,
    api_host: &str,
    registry: &RegistryConfig,
) -> Result<String, String> {
    let reg_host = registry_host_from_url(&registry.url);
    if !reg_host.is_empty()
        && api_host == reg_host.as_str()
        && let Some(ref raw_scheme) = registry.scheme
    {
        match raw_scheme.trim().to_ascii_lowercase().as_str() {
            "https" => return Ok(format!("https://{api_host}")),
            "http" => return Ok(format!("http://{api_host}")),
            _ => {}
        }
    }
    let https = format!("https://{api_host}/v2/");
    if let Ok(res) = client
        .get(&https)
        .timeout(Duration::from_secs(PROBE_TIMEOUT_SECS))
        .send()
        .await
        && (res.status().is_success()
            || res.status() == reqwest::StatusCode::UNAUTHORIZED
            || res.status() == reqwest::StatusCode::FORBIDDEN)
    {
        return Ok(format!("https://{api_host}"));
    }
    let http = format!("http://{api_host}/v2/");
    match client
        .get(&http)
        .timeout(Duration::from_secs(PROBE_TIMEOUT_SECS))
        .send()
        .await
    {
        Ok(res)
            if res.status().is_success()
                || res.status() == reqwest::StatusCode::UNAUTHORIZED
                || res.status() == reqwest::StatusCode::FORBIDDEN =>
        {
            Ok(format!("http://{api_host}"))
        }
        Ok(res) => Err(format!(
            "registry {api_host} probe failed with status {}",
            res.status()
        )),
        Err(e) => Err(format!("registry {api_host} unreachable: {e}")),
    }
}

fn truncate_body(body: &str) -> String {
    const MAX: usize = 200;
    if body.len() <= MAX {
        body.to_string()
    } else {
        format!("{}…", body.get(..MAX).unwrap_or(body))
    }
}

fn accept_header() -> &'static str {
    concat!(
        "application/vnd.docker.distribution.manifest.list.v2+json, ",
        "application/vnd.oci.image.index.v1+json, ",
        "application/vnd.docker.distribution.manifest.v2+json, ",
        "application/vnd.oci.image.manifest.v1+json, ",
        "*/*"
    )
}

async fn download_blobs(
    worker: &Arc<PullWorker>,
    manifest: &RemoteImageManifest,
    temp_guard: TempDirGuard,
    source_ref: &str,
    target_ref: &str,
    layer_concurrency: usize,
) -> Result<PulledImage, String> {
    let config_digest = manifest.config.digest.clone();
    let config_media_type = manifest
        .config
        .media_type
        .clone()
        .unwrap_or_else(|| super::media::DOCKER_CONFIG.to_string());
    let prefer_oci = config_media_type == super::media::OCI_CONFIG
        || config_media_type.contains("vnd.oci.image.config");
    let config_path = temp_guard.path().join("config.json");
    worker
        .download_blob_to_file(&config_digest, &config_path, "config")
        .await?;
    let gate = Arc::new(Semaphore::new(layer_concurrency.max(1)));
    let mut join_set = JoinSet::new();
    let parent_sink = crate::logging::current_event_sink();
    for (idx, layer) in manifest.layers.iter().enumerate() {
        let sem = Arc::clone(&gate);
        let w = Arc::clone(worker);
        let layer_digest = layer.digest.clone();
        let layer_mt_opt = layer.media_type.clone();
        let layer_path = temp_guard.path().join(format!("layer-{idx}.bin"));
        let layer_sink = parent_sink.clone();
        join_set.spawn(async move {
            crate::logging::with_optional_event_sink(layer_sink, async move {
                let _permit = sem
                    .acquire_owned()
                    .await
                    .map_err(|e| format!("layer concurrency semaphore closed: {e}"))?;
                w.download_blob_to_file(&layer_digest, &layer_path, &format!("layer-{idx}"))
                    .await?;
                let media_type = layer_mt_opt.unwrap_or_else(|| {
                    let default_mt = if prefer_oci {
                        super::media::OCI_LAYER_GZIP
                    } else {
                        super::media::DOCKER_LAYER_GZIP
                    };
                    super::media::sniff_layer_media_type_from_path(&layer_path, prefer_oci)
                        .unwrap_or(default_mt)
                        .to_string()
                });
                Ok::<(usize, PulledLayer), String>((
                    idx,
                    PulledLayer {
                        digest: layer_digest,
                        path: layer_path,
                        media_type,
                    },
                ))
            })
            .await
        });
    }
    let mut indexed_layers = Vec::with_capacity(manifest.layers.len());
    while let Some(joined) = join_set.join_next().await {
        match joined {
            Ok(Ok(pair)) => indexed_layers.push(pair),
            Ok(Err(err)) => {
                join_set.abort_all();
                return Err(err);
            }
            Err(join_err) => {
                join_set.abort_all();
                return Err(format!(
                    "layer download worker panicked/cancelled: {join_err}"
                ));
            }
        }
    }
    indexed_layers.sort_by_key(|(idx, _)| *idx);
    let layers = indexed_layers.into_iter().map(|(_, layer)| layer).collect();
    Ok(PulledImage {
        source_ref: source_ref.to_string(),
        target_ref: target_ref.to_string(),
        config_digest,
        config_path,
        config_media_type,
        layers,
        temp_dir: temp_guard,
    })
}

fn short_digest(digest: &str) -> String {
    let hex = digest.strip_prefix("sha256:").unwrap_or(digest);
    hex.get(..12.min(hex.len())).unwrap_or(hex).to_string()
}

pub(super) fn platform_tuple(platform: &str) -> Result<(String, String, Option<String>), String> {
    parse_platform(platform)
}

async fn resolve_source_mirror_endpoint(
    client: &reqwest::Client,
    image_ref: &ImageRef,
    sync: &ImageSyncConfig,
    registry: &RegistryConfig,
) -> Result<(String, String), String> {
    let raw_mirror = sync
        .source_mirrors
        .get(&image_ref.host)
        .or_else(|| sync.source_mirrors.get(image_ref.api_host()))
        .or_else(|| sync.source_mirrors.get("default"))
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());

    let Some(mirror_val) = raw_mirror else {
        let base = resolve_registry_base_url(client, image_ref.api_host(), registry).await?;
        return Ok((base, image_ref.repository.clone()));
    };

    let (explicit_scheme, rest) = if let Some(r) = mirror_val.strip_prefix("https://") {
        (Some("https"), r.trim_end_matches('/'))
    } else if let Some(r) = mirror_val.strip_prefix("http://") {
        (Some("http"), r.trim_end_matches('/'))
    } else {
        (None, mirror_val.trim_end_matches('/'))
    };

    let (host_part, path_prefix) = match rest.split_once('/') {
        Some((h, p)) => (h, p.trim_matches('/')),
        None => (rest, ""),
    };
    if host_part.is_empty() {
        return Err(format!("invalid source_mirrors entry '{mirror_val}'"));
    }

    let base_url = match explicit_scheme {
        Some(scheme) => format!("{scheme}://{host_part}"),
        None => resolve_registry_base_url(client, host_part, registry).await?,
    };

    let stripped_v2 = path_prefix
        .strip_prefix("v2/")
        .or_else(|| (path_prefix == "v2").then_some(""))
        .unwrap_or(path_prefix)
        .trim_matches('/');

    let repo = if stripped_v2.is_empty() {
        image_ref.repository.clone()
    } else {
        format!("{stripped_v2}/{}", image_ref.repository)
    };
    Ok((base_url, repo))
}
