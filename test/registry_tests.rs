//! End-to-end integration tests for pure-Rust `OCI` Distribution v2 & `Docker` Registry v2
//! image pull/push, `SHA256` digest verification, `RFC` 7235 `Bearer` token exchange with
//! quoted commas, and `Harbor` v2 project auto-creation (`harbor_project_public`).

use rtkk::registry::{
    PullImagesOptions, PushImagesBatchOptions, execute_pull_images, execute_push_images,
    execute_push_images_batch,
};
use rtkk::utils::base64_encode;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

/// Valid 20-byte `RFC` 1952 `gzip` stream for a layer payload.
const SAMPLE_GZIP_LAYER: &[u8] = &[
    0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00,
];

const EXPECTED_TOKEN: &str = "harbor-mock-jwt-token";

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(prefix: &str) -> Result<Self, String> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("rtkk-reg-test-{prefix}-{pid}-{nanos}"));
        std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
        Ok(Self { path })
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn ensure(cond: bool, msg: &str) -> Result<(), String> {
    if cond { Ok(()) } else { Err(msg.to_string()) }
}

fn sha256_digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

#[derive(Debug, Clone)]
struct PushedManifestRecord {
    repo: String,
    tag: String,
    content_type: String,
    media_type_field: Option<String>,
}

#[derive(Debug, Clone)]
struct CreatedProjectRecord {
    project_name: String,
    public_flag: bool,
    metadata_public: String,
}

#[derive(Default)]
struct MockRegistryState {
    port: u16,
    expected_basic_header: String,
    upstream_manifests: HashMap<String, (String, Vec<u8>)>,
    upstream_blobs: HashMap<String, Vec<u8>>,
    pull_manifest_hits: usize,
    token_scopes: Vec<String>,
    created_projects: HashSet<String>,
    created_project_calls: Vec<CreatedProjectRecord>,
    uploaded_blobs: HashSet<(String, String)>,
    pushed_manifests: Vec<PushedManifestRecord>,
    next_upload_id: usize,
}

struct HttpReq {
    method: String,
    target: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

struct HttpResp {
    status: &'static str,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl HttpResp {
    fn empty(status: &'static str, headers: Vec<(String, String)>) -> Self {
        Self {
            status,
            headers,
            body: Vec::new(),
        }
    }

    fn json_body(status: &'static str, content_type: &str, body: Vec<u8>) -> Self {
        Self {
            status,
            headers: vec![("Content-Type".to_string(), content_type.to_string())],
            body,
        }
    }
}

async fn read_http_req(stream: &mut TcpStream) -> Option<HttpReq> {
    let mut raw = Vec::new();
    let mut hdr_end_opt = None;
    while hdr_end_opt.is_none() {
        let mut chunk = [0_u8; 4_096];
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        if raw.is_empty() && !chunk.first().is_some_and(u8::is_ascii_uppercase) {
            return None;
        }
        raw.extend_from_slice(chunk.get(..n)?);
        hdr_end_opt = raw.windows(4).position(|w| w == b"\r\n\r\n");
    }
    let hdr_end = hdr_end_opt?;
    let hdr_str = std::str::from_utf8(raw.get(..hdr_end)?).ok()?;
    let mut lines = hdr_str.split("\r\n");
    let mut req_parts = lines.next()?.split_whitespace();
    let method = req_parts.next()?.to_string();
    let target = req_parts.next()?.to_string();
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    let content_len = headers
        .get("content-length")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    let body_start = hdr_end.saturating_add(4);
    let total_needed = body_start.saturating_add(content_len);
    while raw.len() < total_needed {
        let mut chunk = [0_u8; 8_192];
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            break;
        }
        raw.extend_from_slice(chunk.get(..n)?);
    }
    let body = raw.get(body_start..total_needed.min(raw.len()))?.to_vec();
    Some(HttpReq {
        method,
        target,
        headers,
        body,
    })
}

async fn write_http_resp(stream: &mut TcpStream, resp: HttpResp) {
    let mut head = format!(
        "HTTP/1.1 {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        resp.status,
        resp.body.len()
    );
    for (k, v) in resp.headers {
        head.push_str(&k);
        head.push_str(": ");
        head.push_str(&v);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes()).await;
    if !resp.body.is_empty() {
        let _ = stream.write_all(&resp.body).await;
    }
    let _ = stream.shutdown().await;
}

fn bearer_challenge_resp(port: u16, repo: &str) -> HttpResp {
    let www = format!(
        "Bearer realm=\"http://127.0.0.1:{port}/token\",service=\"harbor-registry\",scope=\"repository:{repo}:pull,push\""
    );
    HttpResp::empty(
        "401 Unauthorized",
        vec![("WWW-Authenticate".to_string(), www)],
    )
}

fn handle_token_endpoint(req: &HttpReq, st: &mut MockRegistryState) -> HttpResp {
    if req.headers.get("authorization") != Some(&st.expected_basic_header) {
        return HttpResp::empty("401 Unauthorized", Vec::new());
    }
    let Ok(parsed) = reqwest::Url::parse(&format!("http://localhost{}", req.target)) else {
        return HttpResp::empty("400 Bad Request", Vec::new());
    };
    let mut service = String::new();
    let mut scope = String::new();
    for (k, v) in parsed.query_pairs() {
        if k == "service" {
            service = v.into_owned();
        } else if k == "scope" {
            scope = v.into_owned();
        }
    }
    if service != "harbor-registry" {
        return HttpResp::empty("400 Bad Request", Vec::new());
    }
    st.token_scopes.push(scope);
    let body = serde_json::to_vec(&json!({ "token": EXPECTED_TOKEN })).unwrap_or_default();
    HttpResp::json_body("200 OK", "application/json", body)
}

fn handle_harbor_create_project(req: &HttpReq, st: &mut MockRegistryState) -> HttpResp {
    if req.headers.get("authorization") != Some(&st.expected_basic_header) {
        return HttpResp::empty("401 Unauthorized", Vec::new());
    }
    let Ok(payload) = serde_json::from_slice::<Value>(&req.body) else {
        return HttpResp::empty("400 Bad Request", Vec::new());
    };
    let Some(project_name) = payload.get("project_name").and_then(Value::as_str) else {
        return HttpResp::empty("400 Bad Request", Vec::new());
    };
    let public_flag = payload
        .get("public")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let metadata_public = payload
        .get("metadata")
        .and_then(|m| m.get("public"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    st.created_projects.insert(project_name.to_string());
    st.created_project_calls.push(CreatedProjectRecord {
        project_name: project_name.to_string(),
        public_flag,
        metadata_public,
    });
    HttpResp::json_body("201 Created", "application/json", b"{}".to_vec())
}

fn handle_upstream_pull(path_only: &str, st: &mut MockRegistryState) -> HttpResp {
    if let Some((repo, rest)) = path_only
        .strip_prefix("/v2/")
        .and_then(|s| s.split_once("/manifests/"))
    {
        st.pull_manifest_hits = st.pull_manifest_hits.saturating_add(1);
        let key = format!("{repo}:{rest}");
        if let Some((ct, bytes)) = st.upstream_manifests.get(&key) {
            return HttpResp::json_body("200 OK", ct, bytes.clone());
        }
        return HttpResp::empty("404 Not Found", Vec::new());
    }
    if let Some((_, digest)) = path_only
        .strip_prefix("/v2/")
        .and_then(|s| s.split_once("/blobs/"))
    {
        if let Some(bytes) = st.upstream_blobs.get(digest) {
            return HttpResp::json_body("200 OK", "application/octet-stream", bytes.clone());
        }
        return HttpResp::empty("404 Not Found", Vec::new());
    }
    HttpResp::empty("404 Not Found", Vec::new())
}

fn handle_push_v2(
    req: &HttpReq,
    path_only: &str,
    query_digest: Option<&str>,
    st: &mut MockRegistryState,
) -> HttpResp {
    let Some(after_v2) = path_only.strip_prefix("/v2/") else {
        return HttpResp::empty("404 Not Found", Vec::new());
    };
    let repo = extract_repo_from_v2_path(after_v2);
    let want_bearer = format!("Bearer {EXPECTED_TOKEN}");
    if req.headers.get("authorization") != Some(&want_bearer) {
        return bearer_challenge_resp(st.port, repo);
    }
    if req.method == "HEAD"
        && let Some((_, digest)) = after_v2.split_once("/blobs/")
    {
        return if st
            .uploaded_blobs
            .contains(&(repo.to_string(), digest.to_string()))
        {
            HttpResp::empty("200 OK", Vec::new())
        } else {
            HttpResp::empty("404 Not Found", Vec::new())
        };
    }
    if req.method == "POST" && after_v2.ends_with("/blobs/uploads/") {
        let project = repo.split('/').next().unwrap_or("");
        if !st.created_projects.contains(project) {
            let err_body = serde_json::to_vec(&json!({
                "errors": [{ "code": "PROJECT_NOT_FOUND", "message": "project not found" }]
            }))
            .unwrap_or_default();
            return HttpResp::json_body("404 Not Found", "application/json", err_body);
        }
        st.next_upload_id = st.next_upload_id.saturating_add(1);
        let loc = format!("/v2/{repo}/blobs/uploads/u-{}", st.next_upload_id);
        return HttpResp::empty("202 Accepted", vec![("Location".to_string(), loc)]);
    }
    handle_push_put(req, after_v2, repo, query_digest, st)
}

fn handle_push_put(
    req: &HttpReq,
    after_v2: &str,
    repo: &str,
    query_digest: Option<&str>,
    st: &mut MockRegistryState,
) -> HttpResp {
    if req.method == "PUT" && after_v2.contains("/blobs/uploads/") {
        let Some(expected_dig) = query_digest else {
            return HttpResp::empty("400 Bad Request", Vec::new());
        };
        if sha256_digest(&req.body) != expected_dig {
            return HttpResp::empty("400 Bad Request", Vec::new());
        }
        st.uploaded_blobs
            .insert((repo.to_string(), expected_dig.to_string()));
        return HttpResp::empty("201 Created", Vec::new());
    }
    if req.method == "PUT"
        && let Some((_, tag)) = after_v2.split_once("/manifests/")
    {
        let content_type = req.headers.get("content-type").cloned().unwrap_or_default();
        let media_type_field = serde_json::from_slice::<Value>(&req.body)
            .ok()
            .and_then(|v| v.get("mediaType").and_then(Value::as_str).map(String::from));
        st.pushed_manifests.push(PushedManifestRecord {
            repo: repo.to_string(),
            tag: tag.to_string(),
            content_type,
            media_type_field,
        });
        return HttpResp::empty("201 Created", Vec::new());
    }
    HttpResp::empty("404 Not Found", Vec::new())
}

fn extract_repo_from_v2_path(after_v2: &str) -> &str {
    if let Some((repo, _)) = after_v2.split_once("/blobs/") {
        repo
    } else if let Some((repo, _)) = after_v2.split_once("/manifests/") {
        repo
    } else {
        after_v2.trim_end_matches('/')
    }
}

async fn route_request(req: HttpReq, state: &Arc<Mutex<MockRegistryState>>) -> HttpResp {
    let parsed_url = reqwest::Url::parse(&format!("http://localhost{}", req.target)).ok();
    let path_only = parsed_url
        .as_ref()
        .map_or(req.target.as_str(), reqwest::Url::path);
    let query_digest = parsed_url.as_ref().and_then(|u| {
        u.query_pairs()
            .find(|(k, _)| k == "digest")
            .map(|(_, v)| v.into_owned())
    });

    let mut st = state.lock().await;
    if path_only == "/v2/" || path_only == "/v2" {
        return HttpResp::json_body("200 OK", "application/json", b"{}".to_vec());
    }
    if path_only == "/token" {
        return handle_token_endpoint(&req, &mut st);
    }
    if req.method == "POST" && path_only == "/api/v2.0/projects" {
        return handle_harbor_create_project(&req, &mut st);
    }
    if req.method == "GET" && path_only.starts_with("/v2/upstream/") {
        return handle_upstream_pull(path_only, &mut st);
    }
    handle_push_v2(&req, path_only, query_digest.as_deref(), &mut st)
}

async fn spawn_mock_registry(state: Arc<Mutex<MockRegistryState>>) -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    {
        let mut st = state.lock().await;
        st.port = port;
    }
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let st_clone = Arc::clone(&state);
            tokio::spawn(async move {
                if let Some(req) = read_http_req(&mut stream).await {
                    let resp = route_request(req, &st_clone).await;
                    write_http_resp(&mut stream, resp).await;
                }
            });
        }
    });
    Ok(port)
}

fn seed_upstream_image(
    st: &mut MockRegistryState,
    repo_tag: &str,
    oci_format: bool,
    tamper_layer_bytes: bool,
) {
    let cfg_bytes =
        br#"{"architecture":"amd64","os":"linux","rootfs":{"type":"layers","diff_ids":[]}}"#
            .to_vec();
    let cfg_digest = sha256_digest(&cfg_bytes);
    let layer_bytes = SAMPLE_GZIP_LAYER.to_vec();
    let layer_digest = sha256_digest(&layer_bytes);

    let (manifest_mt, cfg_mt, layer_mt) = if oci_format {
        (
            "application/vnd.oci.image.manifest.v1+json",
            "application/vnd.oci.image.config.v1+json",
            "application/vnd.oci.image.layer.v1.tar+gzip",
        )
    } else {
        (
            "application/vnd.docker.distribution.manifest.v2+json",
            "application/vnd.docker.container.image.v1+json",
            "application/vnd.docker.image.rootfs.diff.tar.gzip",
        )
    };
    let manifest_bytes = serde_json::to_vec(&json!({
        "schemaVersion": 2,
        "mediaType": manifest_mt,
        "config": {
            "mediaType": cfg_mt,
            "size": cfg_bytes.len(),
            "digest": cfg_digest
        },
        "layers": [{
            "mediaType": layer_mt,
            "size": layer_bytes.len(),
            "digest": layer_digest
        }]
    }))
    .unwrap_or_default();

    st.upstream_blobs.insert(cfg_digest, cfg_bytes);
    if tamper_layer_bytes {
        st.upstream_blobs
            .insert(layer_digest, b"tampered-corrupt-layer-stream".to_vec());
    } else {
        st.upstream_blobs.insert(layer_digest, layer_bytes);
    }
    st.upstream_manifests.insert(
        repo_tag.to_string(),
        (manifest_mt.to_string(), manifest_bytes),
    );
}

fn inspect_tar_json_members(tar_path: &Path) -> Result<(Value, Value), String> {
    let file = std::fs::File::open(tar_path).map_err(|e| e.to_string())?;
    let mut archive = tar::Archive::new(file);
    let mut manifest_val = None;
    let mut media_val = None;
    for entry_res in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry_res.map_err(|e| e.to_string())?;
        let name = entry
            .path()
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .into_owned();
        if name == "manifest.json" {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).map_err(|e| e.to_string())?;
            manifest_val = Some(serde_json::from_slice(&buf).map_err(|e| e.to_string())?);
        } else if name == "rtkk-media.json" {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).map_err(|e| e.to_string())?;
            media_val = Some(serde_json::from_slice(&buf).map_err(|e| e.to_string())?);
        }
    }
    Ok((
        manifest_val.ok_or("manifest.json missing in tar")?,
        media_val.ok_or("rtkk-media.json missing in tar")?,
    ))
}

fn write_registry_inventory(
    path: &Path,
    port: u16,
    project: &str,
    harbor_public: bool,
    out_dir: &Path,
) -> Result<(), String> {
    let yaml = format!(
        r#"vars:
  registry:
    url: "127.0.0.1:{port}/{project}"
    username: "admin"
    password: "harbor-secret"
    insecure: true
    scheme: "auto"
    harbor_project_public: {harbor_public}
  image_sync:
    output_dir: "{}"
    platform: "linux/amd64"
    insecure: true
    retries: 1
    layer_concurrency: 2
    http_timeout_secs: 30
hosts:
  n1: 127.0.0.1
roles:
  kube_control_plane: [n1]
"#,
        out_dir.display()
    );
    std::fs::write(path, yaml).map_err(|e| e.to_string())
}

#[tokio::test]
async fn pull_images_creates_legacy_tar_skips_when_unchanged_and_repulls_on_tag_change()
-> Result<(), String> {
    let dir = TestDir::new("pull-lifecycle")?;
    let state = Arc::new(Mutex::new(MockRegistryState {
        expected_basic_header: format!("Basic {}", base64_encode(b"admin:harbor-secret")),
        ..MockRegistryState::default()
    }));
    {
        let mut st = state.lock().await;
        seed_upstream_image(&mut st, "upstream/pause:3.10.1", true, false);
        seed_upstream_image(&mut st, "upstream/pause:3.10.2", true, false);
    }
    let port = spawn_mock_registry(Arc::clone(&state)).await?;

    let inv_path = dir.path.join("inventory.yaml");
    let out_dir = dir.path.join("images_out");
    write_registry_inventory(&inv_path, port, "rtkk", false, &out_dir)?;

    let images_yaml = dir.path.join("images.yaml");
    std::fs::write(
        &images_yaml,
        format!(
            "images:\n  - name: pause\n    source: \"127.0.0.1:{port}/upstream/pause:3.10.1\"\n    target: \"{{{{ .registry.url }}}}/pause:3.10.1\"\n"
        ),
    )
    .map_err(|e| e.to_string())?;

    let opts = PullImagesOptions {
        inventory_path: inv_path.to_string_lossy().into_owned(),
        images_file: images_yaml.to_string_lossy().into_owned(),
        output_dir: None,
        platform: None,
        force: false,
        insecure: Some(true),
    };
    execute_pull_images(&opts).await?;

    let tar_file = out_dir.join("pause.tar");
    let (manifest_json, media_json) = inspect_tar_json_members(&tar_file)?;
    verify_pulled_tar_metadata(&manifest_json, &media_json, port, "3.10.1")?;

    execute_pull_images(&opts).await?;
    let hits_after_skip = state.lock().await.pull_manifest_hits;
    ensure(
        hits_after_skip == 1,
        &format!("second pull without --force must skip network fetch, got {hits_after_skip} hits"),
    )?;

    std::fs::write(
        &images_yaml,
        format!(
            "images:\n  - name: pause\n    source: \"127.0.0.1:{port}/upstream/pause:3.10.2\"\n    target: \"{{{{ .registry.url }}}}/pause:3.10.1\"\n"
        ),
    )
    .map_err(|e| e.to_string())?;
    execute_pull_images(&opts).await?;
    let hits_after_tag_change = state.lock().await.pull_manifest_hits;
    ensure(
        hits_after_tag_change == 2,
        "changing source tag must invalidate cached tar and re-pull",
    )?;
    let (_, updated_media) = inspect_tar_json_members(&tar_file)?;
    ensure(
        updated_media.get("source_ref").and_then(Value::as_str)
            == Some(&format!("127.0.0.1:{port}/upstream/pause:3.10.2")),
        "re-pulled tar must update rtkk-media.json source_ref",
    )?;
    Ok(())
}

fn verify_pulled_tar_metadata(
    manifest_json: &Value,
    media_json: &Value,
    port: u16,
    source_tag: &str,
) -> Result<(), String> {
    let first_entry = manifest_json
        .as_array()
        .and_then(|a| a.first())
        .ok_or("manifest.json must contain one entry")?;
    let repo_tag = first_entry
        .get("RepoTags")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(Value::as_str);
    ensure(
        repo_tag == Some(&format!("127.0.0.1:{port}/rtkk/pause:3.10.1")),
        "manifest.json RepoTags must match target ref",
    )?;
    ensure(
        media_json.get("source_ref").and_then(Value::as_str)
            == Some(&format!("127.0.0.1:{port}/upstream/pause:{source_tag}")),
        "rtkk-media.json must record upstream source_ref",
    )?;
    ensure(
        media_json.get("config").and_then(Value::as_str)
            == Some("application/vnd.oci.image.config.v1+json"),
        "rtkk-media.json must preserve OCI config mediaType",
    )?;
    Ok(())
}

#[tokio::test]
async fn tampered_manifest_and_blob_sha256_digests_are_rejected() -> Result<(), String> {
    let dir = TestDir::new("tampered")?;
    let state = Arc::new(Mutex::new(MockRegistryState {
        expected_basic_header: format!("Basic {}", base64_encode(b"admin:harbor-secret")),
        ..MockRegistryState::default()
    }));
    let wrong_digest = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
    {
        let mut st = state.lock().await;
        seed_upstream_image(
            &mut st,
            &format!("upstream/pause:{wrong_digest}"),
            true,
            false,
        );
        seed_upstream_image(&mut st, "upstream/badblob:1.0", true, true);
    }
    let port = spawn_mock_registry(Arc::clone(&state)).await?;

    let inv_path = dir.path.join("inventory.yaml");
    let out_dir = dir.path.join("out");
    write_registry_inventory(&inv_path, port, "rtkk", true, &out_dir)?;

    let bad_manifest_list = dir.path.join("bad_manifest.yaml");
    std::fs::write(
        &bad_manifest_list,
        format!(
            "images:\n  - name: badm\n    source: \"127.0.0.1:{port}/upstream/pause@{wrong_digest}\"\n    target: \"{{{{ .registry.url }}}}/pause:bad\"\n"
        ),
    )
    .map_err(|e| e.to_string())?;
    let res_manifest = execute_pull_images(&PullImagesOptions {
        inventory_path: inv_path.to_string_lossy().into_owned(),
        images_file: bad_manifest_list.to_string_lossy().into_owned(),
        output_dir: None,
        platform: None,
        force: true,
        insecure: Some(true),
    })
    .await;
    let Err(m_err) = res_manifest else {
        return Err("tampered manifest SHA256 must fail with Err".to_string());
    };
    ensure(
        m_err.contains("manifest digest mismatch"),
        &format!("expected manifest digest mismatch error, got: {m_err}"),
    )?;

    let bad_blob_list = dir.path.join("bad_blob.yaml");
    std::fs::write(
        &bad_blob_list,
        format!(
            "images:\n  - name: badb\n    source: \"127.0.0.1:{port}/upstream/badblob:1.0\"\n    target: \"{{{{ .registry.url }}}}/badblob:1.0\"\n"
        ),
    )
    .map_err(|e| e.to_string())?;
    let res_blob = execute_pull_images(&PullImagesOptions {
        inventory_path: inv_path.to_string_lossy().into_owned(),
        images_file: bad_blob_list.to_string_lossy().into_owned(),
        output_dir: None,
        platform: None,
        force: true,
        insecure: Some(true),
    })
    .await;
    let Err(b_err) = res_blob else {
        return Err("tampered blob SHA256 must fail with Err".to_string());
    };
    ensure(
        b_err.contains("digest mismatch for blob"),
        &format!("expected blob digest mismatch error, got: {b_err}"),
    )?;
    Ok(())
}

#[tokio::test]
async fn push_single_and_batch_handle_bearer_comma_scope_and_harbor_project_creation()
-> Result<(), String> {
    let dir = TestDir::new("push-harbor")?;
    let state = Arc::new(Mutex::new(MockRegistryState {
        expected_basic_header: format!("Basic {}", base64_encode(b"admin:harbor-secret")),
        ..MockRegistryState::default()
    }));
    {
        let mut st = state.lock().await;
        seed_upstream_image(&mut st, "upstream/pause:3.10.1", true, false);
        seed_upstream_image(&mut st, "upstream/coredns:v1.13.1", false, false);
    }
    let port = spawn_mock_registry(Arc::clone(&state)).await?;

    let out_priv = dir.path.join("out_priv");
    let inv_priv = dir.path.join("inv_priv.yaml");
    write_registry_inventory(&inv_priv, port, "rtkk", false, &out_priv)?;
    let imgs_priv = dir.path.join("imgs_priv.yaml");
    std::fs::write(
        &imgs_priv,
        format!(
            "images:\n  - name: pause\n    source: \"127.0.0.1:{port}/upstream/pause:3.10.1\"\n    target: \"{{{{ .registry.url }}}}/pause:3.10.1\"\n"
        ),
    )
    .map_err(|e| e.to_string())?;

    execute_pull_images(&PullImagesOptions {
        inventory_path: inv_priv.to_string_lossy().into_owned(),
        images_file: imgs_priv.to_string_lossy().into_owned(),
        output_dir: None,
        platform: None,
        force: true,
        insecure: Some(true),
    })
    .await?;

    execute_push_images(
        &out_priv.join("pause.tar").to_string_lossy(),
        &inv_priv.to_string_lossy(),
    )
    .await?;

    run_public_batch_push(&dir.path, port).await?;
    verify_harbor_push_records(&state).await
}

async fn run_public_batch_push(root: &Path, port: u16) -> Result<(), String> {
    let out_pub = root.join("out_pub");
    let inv_pub = root.join("inv_pub.yaml");
    write_registry_inventory(&inv_pub, port, "pubproj", true, &out_pub)?;
    let imgs_pub = root.join("imgs_pub.yaml");
    std::fs::write(
        &imgs_pub,
        format!(
            "images:\n  - name: coredns\n    source: \"127.0.0.1:{port}/upstream/coredns:v1.13.1\"\n    target: \"{{{{ .registry.url }}}}/coredns:v1.13.1\"\n"
        ),
    )
    .map_err(|e| e.to_string())?;

    execute_pull_images(&PullImagesOptions {
        inventory_path: inv_pub.to_string_lossy().into_owned(),
        images_file: imgs_pub.to_string_lossy().into_owned(),
        output_dir: None,
        platform: None,
        force: true,
        insecure: Some(true),
    })
    .await?;

    execute_push_images_batch(&PushImagesBatchOptions {
        inventory_path: inv_pub.to_string_lossy().into_owned(),
        images_file: imgs_pub.to_string_lossy().into_owned(),
        output_dir: None,
    })
    .await
}

async fn verify_harbor_push_records(state: &Arc<Mutex<MockRegistryState>>) -> Result<(), String> {
    let st = state.lock().await;
    ensure(
        st.token_scopes
            .iter()
            .any(|s| s == "repository:rtkk/pause:pull,push"),
        &format!(
            "Bearer token exchange must preserve comma in quoted scope, got {:?}",
            st.token_scopes
        ),
    )?;
    let priv_proj = st
        .created_project_calls
        .iter()
        .find(|c| c.project_name == "rtkk")
        .ok_or("expected Harbor POST /api/v2.0/projects for rtkk")?;
    ensure(
        !priv_proj.public_flag && priv_proj.metadata_public == "false",
        "harbor_project_public: false must send public=false and metadata.public=\"false\"",
    )?;

    let pub_proj = st
        .created_project_calls
        .iter()
        .find(|c| c.project_name == "pubproj")
        .ok_or("expected Harbor POST /api/v2.0/projects for pubproj")?;
    ensure(
        pub_proj.public_flag && pub_proj.metadata_public == "true",
        "harbor_project_public: true must send public=true and metadata.public=\"true\"",
    )?;

    let oci_manifest = st
        .pushed_manifests
        .iter()
        .find(|m| m.repo == "rtkk/pause" && m.tag == "3.10.1")
        .ok_or("missing pushed manifest for rtkk/pause:3.10.1")?;
    ensure(
        oci_manifest.content_type == "application/vnd.oci.image.manifest.v1+json"
            && oci_manifest.media_type_field.as_deref()
                == Some("application/vnd.oci.image.manifest.v1+json"),
        "OCI image must be pushed with OCI manifest v1 mediaType",
    )?;

    let docker_manifest = st
        .pushed_manifests
        .iter()
        .find(|m| m.repo == "pubproj/coredns" && m.tag == "v1.13.1")
        .ok_or("missing pushed manifest for pubproj/coredns:v1.13.1")?;
    ensure(
        docker_manifest.content_type == "application/vnd.docker.distribution.manifest.v2+json"
            && docker_manifest.media_type_field.as_deref()
                == Some("application/vnd.docker.distribution.manifest.v2+json"),
        "Docker v2 image must be pushed with Docker manifest v2 mediaType",
    )?;
    Ok(())
}
