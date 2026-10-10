use super::RegistryConfig;
use super::image_list::{ImageSpec, ImageSyncConfig, parse_platform};
use super::manifest::{RemoteDescriptor, RemoteImageManifest, RemoteIndex};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;
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
    pub target_ref: String,
    pub config_digest: String,
    pub config_path: PathBuf,
    pub config_media_type: String,
    pub layers: Vec<PulledLayer>,
    pub temp_dir: PathBuf,
}
pub(super) fn parse_image_ref(raw: &str) -> Result<ImageRef, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Err("empty image reference".to_string());
    }
    let (name_part, digest) = if let Some(idx) = s.find('@') {
        let name = s.get(..idx).unwrap_or("").trim();
        let dig = s.get(idx + 1..).unwrap_or("").trim();
        if name.is_empty() || dig.is_empty() {
            return Err(format!("invalid digest reference: {raw}"));
        }
        (name, Some(dig.to_string()))
    } else {
        (s, None)
    };
    let (without_tag, tag) = if digest.is_some() {
        (name_part.to_string(), None)
    } else {
        split_name_and_tag(name_part)?
    };
    let (host, repository) = split_host_and_repo(&without_tag);
    if repository.is_empty() {
        return Err(format!("missing repository in reference: {raw}"));
    }
    if digest.is_none() && tag.is_none() {
        return Err(format!("image reference missing tag or digest: {raw}"));
    }
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
        if right.chars().all(|c| c.is_ascii_digit()) && left.contains('.') {
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
        let repository = if host == "docker.io" || host == "registry-1.docker.io" {
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
    if host == "registry-1.docker.io" {
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
struct AuthSession {
    authorization: Option<String>,
}
struct PullContext<'a> {
    client: &'a reqwest::Client,
    base_url: String,
    repository: String,
    creds: Option<(String, String)>,
    session: AuthSession,
    retries: u32,
}
pub(super) async fn pull_image(
    client: &reqwest::Client,
    spec: &ImageSpec,
    sync: &ImageSyncConfig,
    registry: &RegistryConfig,
    platform: &(String, String, Option<String>),
) -> Result<PulledImage, String> {
    let image_ref = parse_image_ref(&spec.source)?;
    let base_url = resolve_registry_base_url(client, image_ref.api_host()).await?;
    let creds = resolve_source_creds(&image_ref.host, sync, registry);
    let mut ctx = PullContext {
        client,
        base_url,
        repository: image_ref.repository.clone(),
        creds,
        session: AuthSession {
            authorization: None,
        },
        retries: sync.retries,
    };
    ensure_auth(&mut ctx).await?;
    let reference = image_ref
        .digest
        .clone()
        .or_else(|| image_ref.tag.clone())
        .ok_or_else(|| "missing tag/digest".to_string())?;
    let manifest_bytes = ctx.fetch_manifest(&reference).await?;
    let image_manifest = resolve_image_manifest(&mut ctx, &manifest_bytes, platform).await?;
    let temp_dir = make_temp_dir(&spec.name)?;
    match download_blobs(
        &mut ctx,
        &image_manifest,
        &temp_dir,
        &spec.target,
        sync.layer_concurrency,
    )
    .await
    {
        Ok(pulled) => Ok(pulled),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&temp_dir);
            Err(e)
        }
    }
}
async fn resolve_image_manifest(
    ctx: &mut PullContext<'_>,
    manifest_bytes: &[u8],
    platform: &(String, String, Option<String>),
) -> Result<RemoteImageManifest, String> {
    if sniff_manifest_kind(manifest_bytes) == ManifestKind::Index {
        let index: RemoteIndex = serde_json::from_slice(manifest_bytes)
            .map_err(|e| format!("failed to parse manifest list/index: {e}"))?;
        let desc = pick_platform(&index, platform)?;
        let child = ctx.fetch_manifest(&desc.digest).await?;
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
        if let Some(mt) = v.get("mediaType").and_then(|m| m.as_str()) {
            if mt.contains("manifest.list") || mt.contains("image.index") {
                return ManifestKind::Index;
            }
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
    Err(format!(
        "no matching platform {want_os}/{want_arch}{} in manifest list",
        want_variant
            .as_ref()
            .map(|v| format!("/{v}"))
            .unwrap_or_default()
    ))
}
fn resolve_source_creds(
    host: &str,
    sync: &ImageSyncConfig,
    registry: &RegistryConfig,
) -> Option<(String, String)> {
    if let Some(auth) = sync.source_auths.get(host) {
        if let (Some(u), Some(p)) = (&auth.username, &auth.password) {
            if !u.is_empty() {
                return Some((u.clone(), p.clone()));
            }
        }
    }
    let reg_host = registry_host_from_url(&registry.url);
    if host == reg_host.as_str() {
        if let (Some(u), Some(p)) = (&registry.username, &registry.password) {
            return Some((u.clone(), p.clone()));
        }
    }
    None
}
fn registry_host_from_url(url: &str) -> String {
    let clean = url.trim_end_matches('/');
    clean.split('/').next().unwrap_or(clean).to_string()
}
async fn resolve_registry_base_url(
    client: &reqwest::Client,
    api_host: &str,
) -> Result<String, String> {
    let https = format!("https://{api_host}/v2/");
    match client.get(&https).send().await {
        Ok(res)
            if res.status().is_success()
                || res.status() == reqwest::StatusCode::UNAUTHORIZED
                || res.status() == reqwest::StatusCode::FORBIDDEN =>
        {
            return Ok(format!("https://{api_host}"));
        }
        _ => {}
    }
    let http = format!("http://{api_host}/v2/");
    match client.get(&http).send().await {
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
async fn ensure_auth(ctx: &mut PullContext<'_>) -> Result<(), String> {
    let url = format!("{}/v2/", ctx.base_url);
    let mut req = ctx.client.get(&url);
    req = apply_auth(req, ctx);
    let res = req
        .send()
        .await
        .map_err(|e| format!("GET {url} failed: {e}"))?;
    if res.status().is_success() {
        return Ok(());
    }
    if res.status() != reqwest::StatusCode::UNAUTHORIZED {
        return Ok(());
    }
    let www = res
        .headers()
        .get(reqwest::header::WWW_AUTHENTICATE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    refresh_auth_from_www(ctx, &www).await
}
fn apply_auth(mut req: reqwest::RequestBuilder, ctx: &PullContext<'_>) -> reqwest::RequestBuilder {
    if let Some(auth) = &ctx.session.authorization {
        req = req.header(reqwest::header::AUTHORIZATION, auth.clone());
    } else if let Some((u, p)) = &ctx.creds {
        req = req.basic_auth(u, Some(p));
    }
    req
}
async fn refresh_auth_from_www(ctx: &mut PullContext<'_>, www: &str) -> Result<(), String> {
    let lower = www.to_ascii_lowercase();
    if lower.starts_with("basic") {
        if let Some((u, p)) = &ctx.creds {
            ctx.session.authorization = Some(basic_auth_header(u, p));
            return Ok(());
        }
        return Err(format!(
            "registry requires Basic auth but no credentials configured for {}",
            ctx.base_url
        ));
    }
    if lower.starts_with("bearer") {
        let token =
            fetch_bearer_token(ctx.client, www, &ctx.repository, "pull", ctx.creds.as_ref())
                .await?;
        ctx.session.authorization = Some(format!("Bearer {token}"));
        return Ok(());
    }
    Ok(())
}
fn basic_auth_header(user: &str, pass: &str) -> String {
    let raw = format!("{user}:{pass}");
    format!("Basic {}", base64_encode(raw.as_bytes()))
}
fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    let mut idx = 0usize;
    while idx < input.len() {
        let remaining = input.len() - idx;
        let b0 = *input.get(idx).unwrap_or(&0);
        let b1 = if remaining > 1 {
            *input.get(idx + 1).unwrap_or(&0)
        } else {
            0
        };
        let b2 = if remaining > 2 {
            *input.get(idx + 2).unwrap_or(&0)
        } else {
            0
        };
        out.push(char::from(
            *TABLE.get(usize::from(b0 >> 2)).unwrap_or(&b'A'),
        ));
        out.push(char::from(
            *TABLE
                .get(usize::from(((b0 & 0x03) << 4) | (b1 >> 4)))
                .unwrap_or(&b'A'),
        ));
        if remaining > 1 {
            out.push(char::from(
                *TABLE
                    .get(usize::from(((b1 & 0x0f) << 2) | (b2 >> 6)))
                    .unwrap_or(&b'A'),
            ));
        } else {
            out.push('=');
        }
        if remaining > 2 {
            out.push(char::from(
                *TABLE.get(usize::from(b2 & 0x3f)).unwrap_or(&b'A'),
            ));
        } else {
            out.push('=');
        }
        idx += 3;
    }
    out
}
async fn fetch_bearer_token(
    client: &reqwest::Client,
    www: &str,
    repository: &str,
    action: &str,
    creds: Option<&(String, String)>,
) -> Result<String, String> {
    let params = parse_www_authenticate(www);
    let realm = params
        .get("realm")
        .ok_or_else(|| format!("Bearer challenge missing realm: {www}"))?;
    let service = params.get("service").cloned().unwrap_or_default();
    let scope = params
        .get("scope")
        .cloned()
        .unwrap_or_else(|| format!("repository:{repository}:{action}"));
    let mut url =
        reqwest::Url::parse(realm).map_err(|e| format!("invalid token realm {realm}: {e}"))?;
    {
        let mut q = url.query_pairs_mut();
        if !service.is_empty() {
            q.append_pair("service", &service);
        }
        q.append_pair("scope", &scope);
    }
    let mut req = client.get(url);
    if let Some((u, p)) = creds {
        req = req.basic_auth(u, Some(p));
    }
    let res = req
        .send()
        .await
        .map_err(|e| format!("token request failed: {e}"))?;
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(format!(
            "token endpoint returned {status}: {}",
            truncate_body(&body)
        ));
    }
    let body: serde_json::Value = res
        .json()
        .await
        .map_err(|e| format!("failed to parse token JSON: {e}"))?;
    body.get("token")
        .or_else(|| body.get("access_token"))
        .and_then(|t| t.as_str())
        .map(str::to_string)
        .ok_or_else(|| "token response missing token/access_token".to_string())
}
fn parse_www_authenticate(header: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let rest = header
        .find(' ')
        .and_then(|i| header.get(i + 1..))
        .unwrap_or("");
    for part in rest.split(',') {
        let part = part.trim();
        if let Some((k, v)) = part.split_once('=') {
            let v = v.trim().trim_matches('"').to_string();
            map.insert(k.trim().to_ascii_lowercase(), v);
        }
    }
    map
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
impl PullContext<'_> {
    async fn fetch_manifest(&mut self, reference: &str) -> Result<Vec<u8>, String> {
        let url = format!(
            "{}/v2/{}/manifests/{}",
            self.base_url, self.repository, reference
        );
        let attempts = self.retries.max(1);
        let mut last_err = String::new();
        for attempt in 1..=attempts {
            let mut req = self
                .client
                .get(&url)
                .header(reqwest::header::ACCEPT, accept_header());
            req = apply_auth(req, self);
            match req.send().await {
                Ok(res) if res.status() == reqwest::StatusCode::UNAUTHORIZED => {
                    let www = res
                        .headers()
                        .get(reqwest::header::WWW_AUTHENTICATE)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    match refresh_auth_from_www(self, &www).await {
                        Ok(()) => {
                            last_err = "auth refreshed, retry".to_string();
                        }
                        Err(e) => last_err = e,
                    }
                }
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
}
async fn download_blobs(
    ctx: &mut PullContext<'_>,
    manifest: &RemoteImageManifest,
    temp_dir: &Path,
    target_ref: &str,
    layer_concurrency: usize,
) -> Result<PulledImage, String> {
    let config_digest = manifest.config.digest.clone();
    let config_media_type = manifest
        .config
        .media_type
        .clone()
        .unwrap_or_else(|| super::media::DOCKER_CONFIG.to_string());
    let config_path = temp_dir.join("config.json");
    download_blob_to_file(ctx, &config_digest, &config_path, "config").await?;
    let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(layer_concurrency.max(1)));
    let mut layers = Vec::with_capacity(manifest.layers.len());
    for (idx, layer) in manifest.layers.iter().enumerate() {
        let _permit = gate
            .acquire()
            .await
            .map_err(|e| format!("layer concurrency semaphore closed: {e}"))?;
        let layer_path = temp_dir.join(format!("layer-{idx}.bin"));
        download_blob_to_file(ctx, &layer.digest, &layer_path, &format!("layer-{idx}")).await?;
        let media_type = layer.media_type.clone().unwrap_or_else(|| {
            super::media::sniff_layer_media_type_from_path(&layer_path)
                .unwrap_or(super::media::DOCKER_LAYER_GZIP)
                .to_string()
        });
        layers.push(PulledLayer {
            digest: layer.digest.clone(),
            path: layer_path,
            media_type,
        });
    }
    Ok(PulledImage {
        target_ref: target_ref.to_string(),
        config_digest,
        config_path,
        config_media_type,
        layers,
        temp_dir: temp_dir.to_path_buf(),
    })
}
async fn download_blob_to_file(
    ctx: &mut PullContext<'_>,
    digest: &str,
    dest: &Path,
    label: &str,
) -> Result<(), String> {
    let url = format!("{}/v2/{}/blobs/{}", ctx.base_url, ctx.repository, digest);
    let task_name = format!("Pull {label} {}", short_digest(digest));
    crate::logging::start_task("localhost", &task_name);
    let attempts = ctx.retries.max(1);
    let mut last_err = String::new();
    for attempt in 1..=attempts {
        match try_download_blob_once(ctx, &url, dest, digest).await {
            Ok(()) => {
                crate::logging::finish_task_success("localhost", &task_name);
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
    crate::logging::finish_task_failed("localhost", &task_name, &last_err);
    Err(last_err)
}
async fn try_download_blob_once(
    ctx: &mut PullContext<'_>,
    url: &str,
    dest: &Path,
    expected_digest: &str,
) -> Result<(), String> {
    use futures_util::StreamExt as _;
    use tokio::io::AsyncWriteExt as _;
    let mut req = ctx.client.get(url);
    req = apply_auth(req, ctx);
    let res = req
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
        refresh_auth_from_www(ctx, &www).await?;
        return Err("auth refreshed, retry".to_string());
    }
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(format!(
            "GET blob {url} status {status}: {}",
            truncate_body(&body)
        ));
    }
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
        if let Some(pct_u64) = downloaded.saturating_mul(100).checked_div(total) {
            let pct = u32::try_from(pct_u64).unwrap_or(0);
            let downloaded_mb = crate::utils::u64_to_f64(downloaded) / (1024.0 * 1024.0);
            let total_mb = crate::utils::u64_to_f64(total) / (1024.0 * 1024.0);
            crate::logging::update_task(
                "localhost",
                &format!("blob {}", short_digest(expected_digest)),
                pct.min(100),
                &format!("{downloaded_mb:.1}MB/{total_mb:.1}MB"),
            );
        }
    }
    file.flush()
        .await
        .map_err(|e| format!("flush blob failed: {e}"))?;
    let got = format!("sha256:{:x}", hasher.finalize());
    let expected = if expected_digest.starts_with("sha256:") {
        expected_digest.to_ascii_lowercase()
    } else {
        format!("sha256:{}", expected_digest.to_ascii_lowercase())
    };
    if got != expected {
        return Err(format!(
            "digest mismatch for blob: expected {expected}, got {got}"
        ));
    }
    Ok(())
}
fn short_digest(digest: &str) -> String {
    let hex = digest.strip_prefix("sha256:").unwrap_or(digest);
    hex.get(..12).unwrap_or(hex).to_string()
}
fn make_temp_dir(name: &str) -> Result<PathBuf, String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!("rtkk-pull-{name}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("failed to create temp dir {}: {e}", dir.display()))?;
    Ok(dir)
}
pub(super) fn build_pull_client(
    insecure: bool,
    timeout_secs: u64,
) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .danger_accept_invalid_certs(insecure)
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(timeout_secs.max(1)))
        .build()
        .map_err(|e| format!("failed to build pull HTTP client: {e}"))
}
pub(super) fn platform_tuple(platform: &str) -> Result<(String, String, Option<String>), String> {
    parse_platform(platform)
}
