//! Registry `HTTP` push client, `Basic`/`Bearer` authentication exchange, and `Harbor` project auto-creation.

use super::manifest::{Descriptor, ImageToPush, ManifestV2};
use super::media::outer_manifest_media_type;
use super::tar::{config_media_for_image, layer_media_for_image};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncReadExt as _;
use tokio::sync::Mutex;

const PROBE_TIMEOUT_SECS: u64 = 5;

/// Strips any `http://` or `https://` scheme prefix and trailing `/` from a registry `URL`.
#[must_use]
pub(super) fn normalize_registry_host_prefix(url: &str) -> &str {
    let trimmed = url.trim();
    let without_scheme = if let Some(rest) = trimmed.strip_prefix("https://") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("http://") {
        rest
    } else if trimmed
        .get(..8)
        .is_some_and(|p| p.eq_ignore_ascii_case("https://"))
    {
        trimmed.get(8..).unwrap_or(trimmed)
    } else if trimmed
        .get(..7)
        .is_some_and(|p| p.eq_ignore_ascii_case("http://"))
    {
        trimmed.get(7..).unwrap_or(trimmed)
    } else {
        trimmed
    };
    without_scheme.trim_end_matches('/')
}

/// Parses an `RFC 7235` `WWW-Authenticate` header into `(lowercase_scheme, params)`
/// using a quote-aware character scanner so commas inside quoted `scope` values are preserved.
#[must_use]
pub(super) fn parse_www_authenticate_bearer(header: &str) -> (String, HashMap<String, String>) {
    let trimmed = header.trim();
    let (scheme, rest) = match trimmed.find(char::is_whitespace) {
        Some(idx) => (
            trimmed.get(..idx).unwrap_or("").to_ascii_lowercase(),
            trimmed.get(idx..).unwrap_or("").trim(),
        ),
        None => (trimmed.to_ascii_lowercase(), ""),
    };
    let mut map = HashMap::new();
    let mut chars = rest.chars().peekable();
    while chars.peek().is_some() {
        while chars.peek().is_some_and(|c| c.is_whitespace() || *c == ',') {
            chars.next();
        }
        let mut key = String::new();
        while let Some(&c) = chars.peek() {
            if c == '=' || c == ',' || c.is_whitespace() {
                break;
            }
            key.push(c);
            chars.next();
        }
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        if chars.peek() != Some(&'=') {
            continue;
        }
        chars.next();
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let mut val = String::new();
        if chars.peek() == Some(&'"') {
            chars.next();
            while let Some(c) = chars.next() {
                if c == '\\' {
                    if let Some(escaped) = chars.next() {
                        val.push(escaped);
                    }
                } else if c == '"' {
                    break;
                } else {
                    val.push(c);
                }
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c == ',' || c.is_whitespace() {
                    break;
                }
                val.push(c);
                chars.next();
            }
        }
        if !key.is_empty() {
            map.insert(key.to_ascii_lowercase(), val);
        }
    }
    (scheme, map)
}

async fn send_token_request(
    client: &reqwest::Client,
    realm: &str,
    service: &str,
    scope: Option<&str>,
    creds: Option<(&str, &str)>,
) -> Result<reqwest::Response, String> {
    let mut url =
        reqwest::Url::parse(realm).map_err(|e| format!("invalid token realm {realm}: {e}"))?;
    {
        let mut q = url.query_pairs_mut();
        if !service.is_empty() {
            q.append_pair("service", service);
        }
        if let Some(sc) = scope
            && !sc.is_empty()
        {
            q.append_pair("scope", sc);
        }
    }
    let mut req = client.get(url);
    if let Some((u, p)) = creds {
        req = req.basic_auth(u, Some(p));
    }
    req.send()
        .await
        .map_err(|e| format!("token request to {realm} failed: {e}"))
}

/// Exchanges a `WWW-Authenticate: Bearer` challenge at `realm` for an access token.
pub(super) async fn fetch_bearer_token(
    client: &reqwest::Client,
    www_header: &str,
    repository: &str,
    default_actions: &str,
    creds: Option<(&str, &str)>,
) -> Result<String, String> {
    let (_, params) = parse_www_authenticate_bearer(www_header);
    let realm = params
        .get("realm")
        .ok_or_else(|| format!("Bearer challenge missing realm: {www_header}"))?;
    let service = params.get("service").map_or("", String::as_str);
    let challenge_scope = params.get("scope").cloned();
    let effective_scope = match challenge_scope.as_deref() {
        Some(sc) if default_actions.contains("push") && !sc.contains("push") => {
            format!("repository:{repository}:{default_actions}")
        }
        Some(sc) if !sc.is_empty() => sc.to_string(),
        _ => format!("repository:{repository}:{default_actions}"),
    };
    let mut res = send_token_request(client, realm, service, Some(&effective_scope), creds).await?;
    if !res.status().is_success() && challenge_scope.is_none() {
        res = send_token_request(client, realm, service, None, creds).await?;
    }
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        let snippet = body.get(..200.min(body.len())).unwrap_or(&body);
        return Err(format!("token endpoint returned {status}: {snippet}"));
    }
    let body: serde_json::Value = res
        .json()
        .await
        .map_err(|e| format!("failed to parse token JSON: {e}"))?;
    body.get("token")
        .or_else(|| body.get("access_token"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "token response missing token/access_token".to_string())
}

/// Probes or selects the `HTTP`/`HTTPS` base `URL` for a registry domain according to `registry.scheme`.
pub(super) async fn probe_registry_base_url(
    client: &reqwest::Client,
    domain: &str,
    registry: &super::RegistryConfig,
) -> Result<String, String> {
    if let Some(ref raw_scheme) = registry.scheme {
        let norm = raw_scheme.trim().to_ascii_lowercase();
        match norm.as_str() {
            "https" => return Ok(format!("https://{domain}")),
            "http" => return Ok(format!("http://{domain}")),
            "" | "auto" => {}
            other => {
                return Err(format!(
                    "unsupported registry.scheme '{other}': expected 'auto', 'https', or 'http'"
                ));
            }
        }
    }
    let https_url = format!("https://{domain}/v2/");
    let mut req = client
        .get(&https_url)
        .timeout(Duration::from_secs(PROBE_TIMEOUT_SECS));
    if let (Some(u), Some(p)) = (&registry.username, &registry.password) {
        req = req.basic_auth(u, Some(p));
    }
    if let Ok(res) = req.send().await
        && (res.status().is_success()
            || res.status() == reqwest::StatusCode::UNAUTHORIZED
            || res.status() == reqwest::StatusCode::FORBIDDEN)
    {
        return Ok(format!("https://{domain}"));
    }
    Ok(format!("http://{domain}"))
}

fn response_www_bearer(res: &reqwest::Response) -> Option<String> {
    if res.status() != reqwest::StatusCode::UNAUTHORIZED {
        return None;
    }
    let www = res
        .headers()
        .get(reqwest::header::WWW_AUTHENTICATE)
        .and_then(|v| v.to_str().ok())?;
    let (scheme, _) = parse_www_authenticate_bearer(www);
    if scheme == "bearer" {
        Some(www.to_string())
    } else {
        None
    }
}

fn short_digest(digest: &str) -> &str {
    let hex = digest.strip_prefix("sha256:").unwrap_or(digest);
    hex.get(..12.min(hex.len())).unwrap_or(hex)
}

fn resolve_upload_location(
    base_url: &str,
    repo_name: &str,
    location: &str,
) -> Result<String, String> {
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
        Ok(format!(
            "{base_url}/v2/{repo_name}/blobs/uploads/{location}"
        ))
    }
}

/// Stateful push session caching negotiated per-repository `Bearer` tokens.
pub(super) struct PushSession<'a> {
    client: &'a reqwest::Client,
    base_url: String,
    registry: &'a super::RegistryConfig,
    bearer_tokens: Mutex<HashMap<String, String>>,
}

impl<'a> PushSession<'a> {
    #[must_use]
    pub(super) fn new(
        client: &'a reqwest::Client,
        base_url: String,
        registry: &'a super::RegistryConfig,
    ) -> Self {
        Self {
            client,
            base_url,
            registry,
            bearer_tokens: Mutex::new(HashMap::new()),
        }
    }

    fn basic_creds(&self) -> Option<(&str, &str)> {
        match (&self.registry.username, &self.registry.password) {
            (Some(u), Some(p)) if !u.is_empty() => Some((u.as_str(), p.as_str())),
            _ => None,
        }
    }

    async fn apply_push_auth(
        &self,
        req: reqwest::RequestBuilder,
        repo_name: &str,
    ) -> reqwest::RequestBuilder {
        let cached = {
            let guard = self.bearer_tokens.lock().await;
            guard.get(repo_name).cloned()
        };
        if let Some(token) = cached {
            req.bearer_auth(token)
        } else if let Some((u, p)) = self.basic_creds() {
            req.basic_auth(u, Some(p))
        } else {
            req
        }
    }

    async fn refresh_bearer_token(&self, repo_name: &str, www_header: &str) -> Result<(), String> {
        let token = fetch_bearer_token(
            self.client,
            www_header,
            repo_name,
            "pull,push",
            self.basic_creds(),
        )
        .await?;
        let mut guard = self.bearer_tokens.lock().await;
        guard.insert(repo_name.to_string(), token);
        Ok(())
    }

    async fn invalidate_repo_token(&self, repo_name: &str) {
        let mut guard = self.bearer_tokens.lock().await;
        guard.remove(repo_name);
    }

    async fn send_with_bearer_retry<F>(
        &self,
        repo_name: &str,
        build_req: F,
    ) -> Result<reqwest::Response, reqwest::Error>
    where
        F: Fn(&reqwest::Client) -> reqwest::RequestBuilder,
    {
        let req = self
            .apply_push_auth(build_req(self.client), repo_name)
            .await;
        let res = req.send().await?;
        if let Some(www) = response_www_bearer(&res)
            && self.refresh_bearer_token(repo_name, &www).await.is_ok()
        {
            let replay = self
                .apply_push_auth(build_req(self.client), repo_name)
                .await;
            return replay.send().await;
        }
        Ok(res)
    }

    async fn auto_create_harbor_project(
        &self,
        repo_name: &str,
        body: &str,
    ) -> Result<bool, String> {
        let Some(slash_idx) = repo_name.find('/') else {
            return Ok(false);
        };
        let project_name = repo_name
            .get(..slash_idx)
            .ok_or("invalid project name slice")?;
        if project_name.is_empty() {
            return Ok(false);
        }
        crate::log_println!(
            "Harbor project '{project_name}' not found. Attempting to create it..."
        );
        let is_public = self.registry.harbor_project_public;
        let public_str = if is_public { "true" } else { "false" };
        let project_payload = serde_json::json!({
            "project_name": project_name,
            "public": is_public,
            "metadata": { "public": public_str }
        });
        let create_urls = [
            format!("{}/api/v2.0/projects", self.base_url),
            format!("{}/api/projects", self.base_url),
        ];
        for create_url in create_urls {
            let mut create_req = self
                .client
                .post(&create_url)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .json(&project_payload);
            if let Some((u, p)) = self.basic_creds() {
                create_req = create_req.basic_auth(u, Some(p));
            }
            if let Ok(create_res) = create_req.send().await {
                let c_status = create_res.status();
                if c_status.is_success() || c_status == reqwest::StatusCode::CONFLICT {
                    crate::log_println!(
                        "Successfully created Harbor project '{project_name}' (or it already exists). Retrying blob upload..."
                    );
                    return Ok(true);
                }
            }
        }
        Err(format!(
            "Harbor project '{project_name}' not found and auto-creation failed (body: {body}). Please create the project manually in Harbor."
        ))
    }

    async fn initiate_blob_upload(&self, repo_name: &str) -> Result<String, String> {
        let init_url = format!("{}/v2/{repo_name}/blobs/uploads/", self.base_url);
        let mut init_response = self
            .send_with_bearer_retry(repo_name, |c| {
                c.post(&init_url).header(reqwest::header::CONTENT_LENGTH, 0)
            })
            .await
            .map_err(|e| format!("failed to initiate blob upload for {repo_name}: {e}"))?;
        if !init_response.status().is_success() {
            let status = init_response.status();
            let body = init_response.text().await.unwrap_or_default();
            let body_lower = body.to_ascii_lowercase();
            let missing_project = (status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
                || status == reqwest::StatusCode::NOT_FOUND)
                && ((body_lower.contains("project") && body_lower.contains("not found"))
                    || body.contains("PROJECT_NOT_FOUND"));
            if missing_project && self.auto_create_harbor_project(repo_name, &body).await? {
                self.invalidate_repo_token(repo_name).await;
                init_response = self
                    .send_with_bearer_retry(repo_name, |c| {
                        c.post(&init_url).header(reqwest::header::CONTENT_LENGTH, 0)
                    })
                    .await
                    .map_err(|e| {
                        format!("failed to retry blob upload initiation for {repo_name}: {e}")
                    })?;
                if !init_response.status().is_success() {
                    let retry_status = init_response.status();
                    let retry_body = init_response.text().await.unwrap_or_default();
                    return Err(format!(
                        "blob upload initiation failed after Harbor project creation, status: {retry_status}, body: {retry_body}"
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
            .ok_or_else(|| {
                "missing Location header in blob upload initiation response".to_string()
            })?;
        resolve_upload_location(&self.base_url, repo_name, location)
    }

    async fn put_blob_once(
        &self,
        repo_name: &str,
        upload_url_with_digest: &str,
        path: &Path,
        size: usize,
        task_name: &str,
    ) -> Result<reqwest::Response, String> {
        let file = tokio::fs::File::open(path)
            .await
            .map_err(|e| format!("failed to open blob {}: {e}", path.display()))?;
        let task_name_owned = task_name.to_string();
        let total = size;
        let stream =
            futures_util::stream::try_unfold((file, 0usize), move |(mut file, mut uploaded)| {
                let task_label = task_name_owned.clone();
                async move {
                    let mut buf = vec![0u8; 65_536];
                    let n = file.read(&mut buf).await?;
                    if n == 0 {
                        return Ok::<Option<(bytes::Bytes, _)>, std::io::Error>(None);
                    }
                    buf.truncate(n);
                    uploaded = uploaded.saturating_add(n);
                    if let Some(pct) = uploaded.checked_mul(100).and_then(|p| p.checked_div(total))
                    {
                        let uploaded_mb = crate::utils::usize_to_f64(uploaded) / (1024.0 * 1024.0);
                        let total_mb = crate::utils::usize_to_f64(total) / (1024.0 * 1024.0);
                        let pct_u32 = u32::try_from(pct).unwrap_or(0);
                        crate::logging::update_task(
                            "localhost",
                            &task_label,
                            pct_u32,
                            &format!("{uploaded_mb:.1}MB/{total_mb:.1}MB"),
                        );
                    }
                    Ok(Some((bytes::Bytes::from(buf), (file, uploaded))))
                }
            });
        let body = reqwest::Body::wrap_stream(stream);
        let req = self
            .client
            .put(upload_url_with_digest)
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .header(reqwest::header::CONTENT_LENGTH, size)
            .body(body);
        let authed = self.apply_push_auth(req, repo_name).await;
        authed
            .send()
            .await
            .map_err(|e| format!("failed to upload blob data for {upload_url_with_digest}: {e}"))
    }

    pub(super) async fn upload_blob_from_path(
        &self,
        repo_name: &str,
        digest: &str,
        path: &Path,
        size: usize,
    ) -> Result<(), String> {
        let head_url = format!("{}/v2/{repo_name}/blobs/{digest}", self.base_url);
        let head_response = self
            .send_with_bearer_retry(repo_name, |c| c.head(&head_url))
            .await
            .map_err(|e| format!("HEAD request to check blob {digest} failed: {e}"))?;
        let task_name = format!("Push Blob {}", short_digest(digest));
        if head_response.status() == reqwest::StatusCode::OK {
            crate::logging::start_task("localhost", &task_name);
            crate::logging::finish_task_success("localhost", &task_name);
            return Ok(());
        }
        let upload_url = self.initiate_blob_upload(repo_name).await?;
        let upload_url_with_digest = if upload_url.contains('?') {
            format!("{upload_url}&digest={digest}")
        } else {
            format!("{upload_url}?digest={digest}")
        };
        crate::logging::start_task("localhost", &task_name);
        let mut put_response = match self
            .put_blob_once(repo_name, &upload_url_with_digest, path, size, &task_name)
            .await
        {
            Ok(res) => res,
            Err(e) => {
                crate::logging::finish_task_failed("localhost", &task_name, &e);
                return Err(e);
            }
        };
        if let Some(www) = response_www_bearer(&put_response)
            && self.refresh_bearer_token(repo_name, &www).await.is_ok()
        {
            put_response = match self
                .put_blob_once(repo_name, &upload_url_with_digest, path, size, &task_name)
                .await
            {
                Ok(res) => res,
                Err(e) => {
                    crate::logging::finish_task_failed("localhost", &task_name, &e);
                    return Err(e);
                }
            };
        }
        if !put_response.status().is_success() {
            let status = put_response.status();
            let body_text = put_response.text().await.unwrap_or_default();
            crate::logging::finish_task_failed(
                "localhost",
                &task_name,
                &format!("status {status}"),
            );
            return Err(format!(
                "failed to upload blob data, status: {status}, body: {body_text}"
            ));
        }
        crate::logging::finish_task_success("localhost", &task_name);
        Ok(())
    }

    pub(super) async fn push_manifests(
        &self,
        images_to_push: &[ImageToPush],
        uploaded_files: &HashMap<String, (String, usize, PathBuf)>,
    ) -> Result<(), String> {
        for img in images_to_push {
            self.push_single_manifest(img, uploaded_files).await?;
        }
        Ok(())
    }

    async fn push_single_manifest(
        &self,
        img: &ImageToPush,
        uploaded_files: &HashMap<String, (String, usize, PathBuf)>,
    ) -> Result<(), String> {
        let (config_digest, config_size, _) = uploaded_files
            .get(&img.config_path)
            .ok_or_else(|| format!("missing config file details for {}", img.config_path))?;
        let config_mt = config_media_for_image(img);
        let mut layers_descriptors = Vec::with_capacity(img.layers.len());
        let mut layer_mts = Vec::with_capacity(img.layers.len());
        for (idx, layer_path) in img.layers.iter().enumerate() {
            let (layer_digest, layer_size, layer_local) = uploaded_files
                .get(layer_path)
                .ok_or_else(|| format!("missing layer file details for {layer_path}"))?;
            let media_type = layer_media_for_image(
                img,
                layer_path,
                idx,
                Some(layer_local.as_path()),
                &config_mt,
            );
            layer_mts.push(media_type.clone());
            layers_descriptors.push(Descriptor {
                media_type,
                size: *layer_size,
                digest: layer_digest.clone(),
            });
        }
        let outer_mt = outer_manifest_media_type(&config_mt, &layer_mts);
        let manifest = ManifestV2 {
            schema_version: 2,
            media_type: outer_mt.to_string(),
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
        let manifest_url = format!("{}/v2/{repo_name}/manifests/{tag_name}", self.base_url);
        let put_response = self
            .send_with_bearer_retry(repo_name, |c| {
                c.put(&manifest_url)
                    .header(reqwest::header::CONTENT_TYPE, outer_mt)
                    .header(reqwest::header::CONTENT_LENGTH, manifest_bytes.len())
                    .body(manifest_bytes.clone())
            })
            .await
            .map_err(|e| {
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
        Ok(())
    }
}
