//! X.509 PKI certificate inspection, generation, chain verification, and `rtkk_bak/` archive export (Step 06).

use crate::cert::{
    CertPolicy, GenCertArgs, generate_certificate, parse_duration_hours, verify_certificate_files,
};
use crate::daemon::auth::RequireAuth;
use crate::daemon::state::{ApiError, DaemonState};
use axum::Json;
use axum::body::Body;
use axum::extract::State;
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use flate2::Compression;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Public metadata for a single `.crt` file inside `<work_dir>/pki/`.
#[derive(Debug, Serialize)]
pub struct CertificateMetadataItem {
    /// Path relative to `<work_dir>/pki/`.
    pub rel_path: String,
    /// Subject Common Name (`CN`).
    pub cn: String,
    /// Issuer Common Name (`CN`).
    pub issuer: String,
    /// Whether `BasicConstraints` marks `CA:TRUE`.
    pub is_ca: bool,
    /// Parsed DNS and IP Subject Alternative Names.
    pub sans: Vec<String>,
    /// RFC 2822 / ISO validity start string.
    pub not_before: String,
    /// RFC 2822 / ISO validity end string.
    pub not_after: String,
    /// Whole days remaining from current wall clock (negative if expired).
    pub days_remaining: i64,
    /// Whether a sibling `.key` file exists alongside the `.crt` file.
    pub has_matching_key: bool,
}

/// Response returned by `GET /api/v1/pki/certificates`.
#[derive(Debug, Serialize)]
pub struct ListCertificatesResponse {
    /// Discovered public certificates sorted by `rel_path`.
    pub items: Vec<CertificateMetadataItem>,
}

/// Request body for `POST /api/v1/pki/generate`.
#[derive(Debug, Deserialize)]
pub struct GenerateCertificateRequest {
    /// File stem written under `<work_dir>/pki/custom/<name>.{crt,key}`.
    pub name: String,
    /// Subject Common Name (`CN`).
    pub cn: String,
    /// Issue a `CA:TRUE` certificate when `true`.
    #[serde(default)]
    pub is_ca: bool,
    /// Optional issuer CA stem in `pki/custom/` or `pki/`.
    pub ca_name: Option<String>,
    /// Subject Alternative Names (DNS names or IP addresses).
    #[serde(default)]
    pub sans: Vec<String>,
    /// Duration string (`"867240h"`, `"99y"`, `"1y"`, `"10y"`).
    #[serde(default = "default_validity_99y")]
    pub validity: String,
}

fn default_validity_99y() -> String {
    "867240h".to_string()
}

/// Response returned by `POST /api/v1/pki/generate`.
#[derive(Debug, Serialize)]
pub struct GenerateCertificateResponse {
    /// Relative path to generated `.crt` under `pki/`.
    pub cert_rel_path: String,
    /// Relative path to generated `0600` `.key` under `pki/`.
    pub key_rel_path: String,
    /// Parsed validity duration in hours.
    pub validity_hours: u64,
}

/// Request body for `POST /api/v1/pki/verify`.
#[derive(Debug, Deserialize)]
pub struct VerifyCertificateRequest {
    /// Relative path to `.crt` inside `<work_dir>/pki/`.
    pub cert_rel_path: String,
    /// Optional relative path to `.key` inside `<work_dir>/pki/`.
    pub key_rel_path: Option<String>,
    /// Optional relative path to issuer CA `.crt` inside `<work_dir>/pki/`.
    pub ca_rel_path: Option<String>,
}

/// Response returned by `POST /api/v1/pki/verify`.
#[derive(Debug, Serialize)]
pub struct VerifyCertificateResponse {
    /// Whether certificate, keypair, and CA signature checks passed.
    pub valid: bool,
    /// Diagnostic verification summary.
    pub report: String,
}

/// Single backup file entry under `<work_dir>/rtkk_bak/`.
#[derive(Debug, Serialize)]
pub struct BackupFileItem {
    /// Path relative to `<work_dir>/rtkk_bak/`.
    pub rel_path: String,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Modification epoch timestamp in milliseconds.
    pub modified_ms: u64,
}

/// Response returned by `GET /api/v1/backups`.
#[derive(Debug, Serialize)]
pub struct ListBackupsResponse {
    /// Sorted list of collected backup files.
    pub items: Vec<BackupFileItem>,
}

fn safe_join_within(base: &Path, rel: &str) -> Result<PathBuf, ApiError> {
    let trimmed = rel.trim();
    if trimmed.is_empty() {
        return Err(ApiError::invalid_argument("Relative path cannot be empty"));
    }
    let rel_path = Path::new(trimmed);
    if rel_path.is_absolute() {
        return Err(ApiError::invalid_argument(
            "Absolute paths are not permitted",
        ));
    }
    for comp in rel_path.components() {
        if !matches!(comp, Component::Normal(_)) {
            return Err(ApiError::invalid_argument(format!(
                "Unsafe path traversal segment in '{trimmed}'"
            )));
        }
    }
    Ok(base.join(rel_path))
}

fn collect_files_recursive(base: &Path, cur: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(cur) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            collect_files_recursive(base, &p, out);
        } else if p.is_file() && p.strip_prefix(base).is_ok() {
            out.push(p);
        }
    }
}

fn parse_cert_metadata(pki_root: &Path, cert_path: &Path) -> Option<CertificateMetadataItem> {
    let raw = std::fs::read(cert_path).ok()?;
    let (_, pem_obj) = x509_parser::pem::parse_x509_pem(&raw).ok()?;
    let x509 = pem_obj.parse_x509().ok()?;

    let cn = x509
        .subject()
        .iter_common_name()
        .next()
        .and_then(|a| a.as_str().ok())
        .unwrap_or("")
        .to_string();
    let issuer = x509
        .issuer()
        .iter_common_name()
        .next()
        .and_then(|a| a.as_str().ok())
        .unwrap_or("")
        .to_string();
    let is_ca = x509.is_ca();

    let mut sans = Vec::new();
    if let Ok(Some(san_ext)) = x509.subject_alternative_name() {
        for gn in &san_ext.value.general_names {
            match gn {
                x509_parser::extensions::GeneralName::DNSName(dns) => sans.push((*dns).to_string()),
                x509_parser::extensions::GeneralName::IPAddress(ip_bytes) => {
                    if let Ok(octets) = <[u8; 4]>::try_from(*ip_bytes) {
                        sans.push(std::net::Ipv4Addr::from(octets).to_string());
                    } else if let Ok(octets16) = <[u8; 16]>::try_from(*ip_bytes) {
                        sans.push(std::net::Ipv6Addr::from(octets16).to_string());
                    }
                }
                _ => {}
            }
        }
    }

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    let not_after_ts = x509.validity().not_after.timestamp();
    let days_remaining = (not_after_ts - now_secs) / 86_400;
    let rel_path = cert_path
        .strip_prefix(pki_root)
        .ok()?
        .to_string_lossy()
        .replace('\\', "/");
    let has_matching_key = cert_path.with_extension("key").is_file();

    Some(CertificateMetadataItem {
        rel_path,
        cn,
        issuer,
        is_ca,
        sans,
        not_before: x509.validity().not_before.to_string(),
        not_after: x509.validity().not_after.to_string(),
        days_remaining,
        has_matching_key,
    })
}

/// Lists public X.509 certificate metadata inside `<work_dir>/pki/` without exposing private keys.
///
/// # Errors
///
/// Returns [`ApiError::internal`] if joining the background directory scan task fails.
pub async fn list_certificates(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
) -> Result<Json<ListCertificatesResponse>, ApiError> {
    let pki_dir = state.workspace().pki_dir();
    let items = tokio::task::spawn_blocking(move || {
        let mut files = Vec::new();
        collect_files_recursive(&pki_dir, &pki_dir, &mut files);
        let mut certs = Vec::new();
        for f in files {
            if f.extension().and_then(|e| e.to_str()) == Some("crt")
                && let Some(meta) = parse_cert_metadata(&pki_dir, &f)
            {
                certs.push(meta);
            }
        }
        certs
    })
    .await
    .map_err(|e| ApiError::internal(format!("PKI scan join error: {e}")))?;

    Ok(Json(ListCertificatesResponse { items }))
}

fn resolve_ca_pair(pki_dir: &Path, ca_name: &str) -> Result<(PathBuf, PathBuf), ApiError> {
    if !crate::utils::is_safe_module_name(ca_name) {
        return Err(ApiError::invalid_argument(format!(
            "Unsafe ca_name '{ca_name}'"
        )));
    }
    let custom_crt = pki_dir.join("custom").join(format!("{ca_name}.crt"));
    let custom_key = pki_dir.join("custom").join(format!("{ca_name}.key"));
    if custom_crt.is_file() && custom_key.is_file() {
        return Ok((custom_key, custom_crt));
    }
    let root_crt = pki_dir.join(format!("{ca_name}.crt"));
    let root_key = pki_dir.join(format!("{ca_name}.key"));
    if root_crt.is_file() && root_key.is_file() {
        return Ok((root_key, root_crt));
    }
    Err(ApiError::not_found(format!(
        "Issuer CA '{ca_name}' (.crt + .key) not found in workspace pki/"
    )))
}

/// Generates a new ECDSA P-256 CA or leaf certificate under `<work_dir>/pki/custom/` (`POST /api/v1/pki/generate`).
///
/// # Errors
///
/// Returns [`ApiError`] (`400`, `404`, `409`, or `500`) on invalid inputs, missing CA, or active job conflict.
pub async fn post_generate_certificate(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
    Json(req): Json<GenerateCertificateRequest>,
) -> Result<Json<GenerateCertificateResponse>, ApiError> {
    if let Some(active_id) = state.jobs().active_mutating_job_id() {
        return Err(ApiError::conflict_active_job(&active_id));
    }
    if !crate::utils::is_safe_module_name(&req.name) {
        return Err(ApiError::invalid_argument(format!(
            "Invalid certificate name '{}'",
            req.name
        )));
    }
    if req.cn.trim().is_empty() {
        return Err(ApiError::invalid_argument("Common Name (cn) is required"));
    }
    let validity_hours = parse_duration_hours(&req.validity).map_err(ApiError::invalid_argument)?;

    let pki_dir = state.workspace().pki_dir();
    let (root_key, root_cert) = if let Some(ref ca_name) = req.ca_name
        && !ca_name.trim().is_empty()
    {
        let (rk, rc) = resolve_ca_pair(&pki_dir, ca_name.trim())?;
        (Some(rk), Some(rc))
    } else {
        (None, None)
    };

    let cert_rel = format!("custom/{}.crt", req.name);
    let key_rel = format!("custom/{}.key", req.name);
    let args = GenCertArgs {
        cn: req.cn.trim().to_string(),
        out_key: pki_dir.join(&key_rel),
        out_cert: pki_dir.join(&cert_rel),
        root_key,
        root_cert,
        sans: req.sans,
        is_ca: req.is_ca,
        date_hours: validity_hours,
        policy: CertPolicy::Always,
    };

    tokio::task::spawn_blocking(move || generate_certificate(&args))
        .await
        .map_err(|e| ApiError::internal(format!("Certificate generation join error: {e}")))?
        .map_err(|e| ApiError::invalid_argument(e.to_string()))?;

    Ok(Json(GenerateCertificateResponse {
        cert_rel_path: cert_rel,
        key_rel_path: key_rel,
        validity_hours,
    }))
}

/// Verifies a certificate, optional private key match, and optional CA chain (`POST /api/v1/pki/verify`).
///
/// # Errors
///
/// Returns [`ApiError`] (`400` or `404`) if paths traverse outside `pki/` or files do not exist.
pub async fn post_verify_certificate(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
    Json(req): Json<VerifyCertificateRequest>,
) -> Result<Json<VerifyCertificateResponse>, ApiError> {
    let pki_dir = state.workspace().pki_dir();
    let cert_path = safe_join_within(&pki_dir, &req.cert_rel_path)?;
    if !cert_path.is_file() {
        return Err(ApiError::not_found(format!(
            "Certificate '{}' not found",
            req.cert_rel_path
        )));
    }
    let key_path = match req.key_rel_path.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(rel) => Some(safe_join_within(&pki_dir, rel)?),
        None => None,
    };
    let ca_path = match req.ca_rel_path.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(rel) => Some(safe_join_within(&pki_dir, rel)?),
        None => None,
    };

    let outcome = tokio::task::spawn_blocking(move || {
        verify_certificate_files(&cert_path, key_path.as_deref(), ca_path.as_deref())
    })
    .await
    .map_err(|e| ApiError::internal(format!("Verify join error: {e}")))?;

    match outcome {
        Ok(report) => Ok(Json(VerifyCertificateResponse {
            valid: true,
            report: report.to_string(),
        })),
        Err(err) => Ok(Json(VerifyCertificateResponse {
            valid: false,
            report: err.to_string(),
        })),
    }
}

/// Lists collected backup files under `<work_dir>/rtkk_bak/` (`GET /api/v1/backups`).
///
/// # Errors
///
/// Returns [`ApiError::internal`] if joining the background backup scan task fails.
pub async fn list_backups(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
) -> Result<Json<ListBackupsResponse>, ApiError> {
    let bak_dir = state.workspace().backups_dir();
    let items = tokio::task::spawn_blocking(move || {
        let mut files = Vec::new();
        collect_files_recursive(&bak_dir, &bak_dir, &mut files);
        let mut list = Vec::with_capacity(files.len());
        for f in files {
            let Ok(meta) = std::fs::metadata(&f) else {
                continue;
            };
            let Ok(rel) = f.strip_prefix(&bak_dir) else {
                continue;
            };
            let modified_ms = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .and_then(|d| u64::try_from(d.as_millis()).ok())
                .unwrap_or(0);
            list.push(BackupFileItem {
                rel_path: rel.to_string_lossy().replace('\\', "/"),
                size_bytes: meta.len(),
                modified_ms,
            });
        }
        list
    })
    .await
    .map_err(|e| ApiError::internal(format!("Backups scan join error: {e}")))?;

    Ok(Json(ListBackupsResponse { items }))
}

fn build_backups_tar_gz(bak_dir: &Path) -> Result<Vec<u8>, String> {
    let enc = GzEncoder::new(Vec::new(), Compression::default());
    let mut tar_builder = tar::Builder::new(enc);
    if bak_dir.is_dir() {
        let mut files = Vec::new();
        collect_files_recursive(bak_dir, bak_dir, &mut files);
        for f in files {
            let rel = f
                .strip_prefix(bak_dir)
                .map_err(|e| format!("strip_prefix error: {e}"))?;
            tar_builder
                .append_path_with_name(&f, rel)
                .map_err(|e| format!("failed to archive {}: {e}", f.display()))?;
        }
    }
    let gz_enc = tar_builder
        .into_inner()
        .map_err(|e| format!("failed to finalize tar archive: {e}"))?;
    gz_enc
        .finish()
        .map_err(|e| format!("failed to finalize gzip archive: {e}"))
}

/// Exports `<work_dir>/rtkk_bak/` as a `.tar.gz` download (`GET /api/v1/backups/archive`).
///
/// # Errors
///
/// Returns [`ApiError::internal`] if archive construction fails.
pub async fn download_backups_archive(
    _auth: RequireAuth,
    State(state): State<DaemonState>,
) -> Result<Response, ApiError> {
    let bak_dir = state.workspace().backups_dir();
    tracing::info!(dir = %bak_dir.display(), "exporting rtkk_bak archive");
    let archive_bytes = tokio::task::spawn_blocking(move || build_backups_tar_gz(&bak_dir))
        .await
        .map_err(|e| ApiError::internal(format!("Archive task join error: {e}")))?
        .map_err(ApiError::internal)?;

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let disp = format!("attachment; filename=\"rtkk-backups-{ts}.tar.gz\"");
    let mut res = Response::new(Body::from(archive_bytes));
    *res.status_mut() = StatusCode::OK;
    res.headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/gzip"));
    if let Ok(hv) = HeaderValue::from_str(&disp) {
        res.headers_mut().insert(CONTENT_DISPOSITION, hv);
    }
    Ok(res)
}
