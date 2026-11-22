//! Embedded Park UI static asset serving, `Gzip` content negotiation, `SHA256` `ETag` `304`, and SPA fallback.

use crate::daemon::state::ApiError;
use axum::body::Body;
use axum::http::header::{
    ACCEPT_ENCODING, CACHE_CONTROL, CONTENT_ENCODING, CONTENT_TYPE, ETAG, IF_NONE_MATCH, VARY,
};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

/// Single static web asset compiled into the `rtkk` binary by `build.rs`.
#[derive(Debug, Clone, Copy)]
pub struct EmbeddedAsset {
    /// Leading-slash URL path (e.g. `"/index.html"`, `"/assets/index-abc.js"`).
    pub path: &'static str,
    /// `Content-Type` header value.
    pub mime: &'static str,
    /// Quoted `SHA256` `ETag` value.
    pub etag: &'static str,
    /// Whether this asset has a content-hashed filename eligible for immutable caching.
    pub immutable: bool,
    /// Uncompressed asset bytes.
    pub raw: &'static [u8],
    /// Pre-compressed `Gzip` asset bytes.
    pub gzip: &'static [u8],
}

mod generated {
    use super::EmbeddedAsset;
    include!(concat!(env!("OUT_DIR"), "/embedded_web_assets.rs"));
}

pub use generated::{EMBEDDED_WEB_ASSETS, WEB_DIST_BUILT};

fn find_asset(url_path: &str) -> Option<&'static EmbeddedAsset> {
    EMBEDDED_WEB_ASSETS.iter().find(|a| a.path == url_path)
}

fn client_accepts_gzip(headers: &HeaderMap) -> bool {
    headers
        .get(ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|enc| {
            enc.split(',')
                .any(|part| part.trim().split(';').next().unwrap_or("") == "gzip")
        })
}

fn etag_matches(headers: &HeaderMap, asset_etag: &str) -> bool {
    let Some(raw_inm) = headers.get(IF_NONE_MATCH).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    raw_inm.split(',').any(|candidate| {
        let trimmed = candidate.trim();
        trimmed == "*" || trimmed == asset_etag || trimmed.strip_prefix("W/") == Some(asset_etag)
    })
}

fn build_asset_response(
    method: &Method,
    headers: &HeaderMap,
    asset: &'static EmbeddedAsset,
    is_spa_fallback: bool,
) -> Response {
    let cache_value = if asset.immutable && !is_spa_fallback {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };

    if etag_matches(headers, asset.etag) {
        let mut not_mod = Response::new(Body::empty());
        *not_mod.status_mut() = StatusCode::NOT_MODIFIED;
        if let Ok(etag_hdr) = HeaderValue::from_str(asset.etag) {
            not_mod.headers_mut().insert(ETAG, etag_hdr);
        }
        not_mod
            .headers_mut()
            .insert(CACHE_CONTROL, HeaderValue::from_static(cache_value));
        return not_mod;
    }

    let use_gzip = client_accepts_gzip(headers);
    let payload: &'static [u8] = if *method == Method::HEAD {
        &[]
    } else if use_gzip {
        asset.gzip
    } else {
        asset.raw
    };

    let mut res = Response::new(Body::from(payload));
    *res.status_mut() = StatusCode::OK;
    if let Ok(ct) = HeaderValue::from_str(asset.mime) {
        res.headers_mut().insert(CONTENT_TYPE, ct);
    }
    if let Ok(etag_hdr) = HeaderValue::from_str(asset.etag) {
        res.headers_mut().insert(ETAG, etag_hdr);
    }
    res.headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static(cache_value));
    res.headers_mut()
        .insert(VARY, HeaderValue::from_static("Accept-Encoding"));
    if use_gzip {
        res.headers_mut()
            .insert(CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    }
    res
}

/// Compresses `raw` bytes using `Gzip` (used for dynamic archive and asset compression).
///
/// # Errors
///
/// Returns an error if the `Gzip` encoder fails to finish.
pub fn gzip_compress_bytes(raw: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Write as _;
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(raw)
        .map_err(|e| format!("gzip write failed: {e}"))?;
    enc.finish().map_err(|e| format!("gzip finish failed: {e}"))
}

/// Fallback handler serving embedded frontend assets or SPA `index.html` while isolating `/api/*` `404`s.
pub async fn serve_embedded_or_api_404(method: Method, uri: Uri, headers: HeaderMap) -> Response {
    let req_path = uri.path();
    if req_path == "/api" || req_path.starts_with("/api/") {
        return ApiError::not_found(format!("Unknown API route: {method} {req_path}"))
            .into_response();
    }

    if method != Method::GET && method != Method::HEAD {
        return ApiError::not_found(format!("Unsupported method for static route: {method}"))
            .into_response();
    }

    let normalized = if req_path == "/" {
        "/index.html"
    } else {
        req_path
    };
    if let Some(exact) = find_asset(normalized) {
        return build_asset_response(&method, &headers, exact, false);
    }

    // Asset paths with explicit file extensions under /assets/ return 404 when missing
    if normalized.starts_with("/assets/") {
        return ApiError::not_found(format!("Static asset not found: {normalized}"))
            .into_response();
    }

    if let Some(index_asset) = find_asset("/index.html") {
        return build_asset_response(&method, &headers, index_asset, true);
    }

    ApiError::internal("Embedded index.html missing from binary").into_response()
}
