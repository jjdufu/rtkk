//! High-entropy CSPRNG admin token bootstrap, constant-time verification, and `HttpOnly` session auth.

use crate::daemon::state::{ApiError, DaemonState};
use crate::error::RtkkError;
use axum::extract::FromRequestParts;
use axum::http::header::{AUTHORIZATION, COOKIE};
use axum::http::request::Parts;
use rand::RngCore as _;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use subtle::ConstantTimeEq as _;

/// Session lifetime in seconds (`12` hours).
pub const SESSION_TTL_SECS: u64 = 43_200;
/// Cookie name carrying the `HttpOnly` Daemon session identifier.
pub const SESSION_COOKIE_NAME: &str = "rtkk_session";

const MIN_TOKEN_LEN: usize = 16;
const MAX_FAILED_LOGINS_PER_MIN: usize = 20;
const B64URL_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn encode_base64url_nopad(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().saturating_mul(4).div_ceil(3));
    let mut idx = 0;
    while idx < bytes.len() {
        let b0 = bytes.get(idx).copied().unwrap_or(0);
        let b1 = bytes.get(idx + 1).copied().unwrap_or(0);
        let b2 = bytes.get(idx + 2).copied().unwrap_or(0);
        let rem = bytes.len() - idx;

        let i0 = usize::from(b0 >> 2);
        let i1 = usize::from(((b0 & 0x03) << 4) | (b1 >> 4));
        let i2 = usize::from(((b1 & 0x0f) << 2) | (b2 >> 6));
        let i3 = usize::from(b2 & 0x3f);

        if let Some(&c0) = B64URL_ALPHABET.get(i0) {
            out.push(char::from(c0));
        }
        if let Some(&c1) = B64URL_ALPHABET.get(i1) {
            out.push(char::from(c1));
        }
        if rem > 1
            && let Some(&c2) = B64URL_ALPHABET.get(i2)
        {
            out.push(char::from(c2));
        }
        if rem > 2
            && let Some(&c3) = B64URL_ALPHABET.get(i3)
        {
            out.push(char::from(c3));
        }
        idx += 3;
    }
    out
}

fn sha256_fixed(input: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(input);
    let digest = hasher.finalize();
    let mut arr = [0u8; 32];
    for (dst, src) in arr.iter_mut().zip(digest.iter()) {
        *dst = *src;
    }
    arr
}

fn generate_random_token_43() -> String {
    let mut raw = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut raw);
    encode_base64url_nopad(&raw)
}

fn write_token_file_0600(path: &Path, token: &str) -> Result<(), RtkkError> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| {
            RtkkError::Daemon(format!(
                "failed to create token directory {}: {e}",
                parent.display()
            ))
        })?;
        #[cfg(unix)]
        {
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
    }
    let pid = std::process::id();
    let nonce: u64 = rand::random();
    let tmp = path.with_file_name(format!(".admin.token.tmp.{pid}.{nonce:016x}"));
    std::fs::write(&tmp, format!("{token}\n")).map_err(|e| {
        RtkkError::Daemon(format!("failed to write token file {}: {e}", tmp.display()))
    })?;
    #[cfg(unix)]
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        RtkkError::Daemon(format!(
            "failed to set 0600 permissions on {}: {e}",
            tmp.display()
        ))
    })?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        RtkkError::Daemon(format!(
            "failed to rename {} to {}: {e}",
            tmp.display(),
            path.display()
        ))
    })
}

/// Manages constant-time admin token verification, `HttpOnly` sessions, and login rate limiting.
#[derive(Debug)]
pub struct AuthManager {
    token_digest: [u8; 32],
    sessions: Mutex<HashMap<[u8; 32], Instant>>,
    failed_attempts: Mutex<VecDeque<Instant>>,
}

impl AuthManager {
    /// Initializes an [`AuthManager`] from an explicit token, an existing `token_file`,
    /// or a newly generated 43-character CSPRNG token persisted with `0600` mode.
    ///
    /// Returns `(AuthManager, bootstrapped_new_token)`.
    ///
    /// # Errors
    ///
    /// Returns [`RtkkError::Daemon`] if an explicit/existing token is shorter than 16 bytes
    /// or writing a new token file fails.
    pub fn initialize(
        explicit_token: Option<&str>,
        token_file: &Path,
    ) -> Result<(Self, bool), RtkkError> {
        let (resolved_token, generated) = if let Some(raw) = explicit_token {
            let trimmed = raw.trim();
            if trimmed.len() < MIN_TOKEN_LEN {
                return Err(RtkkError::Daemon(format!(
                    "RTKK_AUTH_TOKEN must be at least {MIN_TOKEN_LEN} characters"
                )));
            }
            (trimmed.to_string(), false)
        } else if token_file.is_file() {
            let content = std::fs::read_to_string(token_file).map_err(|e| {
                RtkkError::Daemon(format!(
                    "failed to read token file {}: {e}",
                    token_file.display()
                ))
            })?;
            let trimmed = content.trim();
            if trimmed.len() < MIN_TOKEN_LEN {
                return Err(RtkkError::Daemon(format!(
                    "token file {} must contain at least {MIN_TOKEN_LEN} characters",
                    token_file.display()
                )));
            }
            #[cfg(unix)]
            std::fs::set_permissions(token_file, std::fs::Permissions::from_mode(0o600)).map_err(
                |e| {
                    RtkkError::Daemon(format!(
                        "failed to enforce 0600 mode on {}: {e}",
                        token_file.display()
                    ))
                },
            )?;
            (trimmed.to_string(), false)
        } else {
            let fresh = generate_random_token_43();
            write_token_file_0600(token_file, &fresh)?;
            (fresh, true)
        };

        Ok((
            Self {
                token_digest: sha256_fixed(resolved_token.as_bytes()),
                sessions: Mutex::new(HashMap::new()),
                failed_attempts: Mutex::new(VecDeque::new()),
            },
            generated,
        ))
    }

    /// Performs constant-time verification of a candidate bearer/login token.
    #[must_use]
    pub fn verify_token(&self, candidate: &str) -> bool {
        let trimmed = candidate.trim();
        if trimmed.is_empty() {
            return false;
        }
        let candidate_digest = sha256_fixed(trimmed.as_bytes());
        bool::from(candidate_digest.ct_eq(&self.token_digest))
    }

    /// Checks login rate limits, verifies `candidate`, and issues a fresh session ID on success.
    ///
    /// # Errors
    ///
    /// Returns [`ApiError`] (`429` or `401`) if rate-limited or invalid.
    pub fn authenticate_and_create_session(&self, candidate: &str) -> Result<String, ApiError> {
        let now = Instant::now();
        {
            let mut failures = self
                .failed_attempts
                .lock()
                .map_err(|_| ApiError::internal("auth rate-limit lock poisoned"))?;
            while failures
                .front()
                .is_some_and(|ts| now.duration_since(*ts) > Duration::from_secs(60))
            {
                let _ = failures.pop_front();
            }
            if failures.len() >= MAX_FAILED_LOGINS_PER_MIN {
                return Err(ApiError::rate_limited(
                    "Too many failed authentication attempts; retry in 60 seconds",
                ));
            }
        }

        if !self.verify_token(candidate) {
            if let Ok(mut failures) = self.failed_attempts.lock() {
                failures.push_back(now);
            }
            return Err(ApiError::unauthorized("Invalid access token"));
        }

        let sid = generate_random_token_43();
        let sid_hash = sha256_fixed(sid.as_bytes());
        let expires_at = now + Duration::from_secs(SESSION_TTL_SECS);
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| ApiError::internal("session store lock poisoned"))?;
        sessions.retain(|_, exp| *exp > now);
        sessions.insert(sid_hash, expires_at);
        Ok(sid)
    }

    /// Validates an active session cookie token in constant time via SHA256 lookup.
    #[must_use]
    pub fn validate_session(&self, sid: &str) -> bool {
        let trimmed = sid.trim();
        if trimmed.is_empty() {
            return false;
        }
        let now = Instant::now();
        let target = sha256_fixed(trimmed.as_bytes());
        let Ok(mut sessions) = self.sessions.lock() else {
            return false;
        };
        sessions.retain(|_, exp| *exp > now);
        sessions.contains_key(&target)
    }

    /// Revokes a session cookie token if present.
    pub fn revoke_session(&self, sid: &str) {
        let target = sha256_fixed(sid.trim().as_bytes());
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(&target);
        }
    }
}

/// Extracts `rtkk_session` value from a raw `Cookie` header string.
#[must_use]
pub fn extract_session_cookie(cookie_header: &str) -> Option<&str> {
    for part in cookie_header.split(';') {
        let trimmed = part.trim();
        if let Some((k, v)) = trimmed.split_once('=')
            && k.trim() == SESSION_COOKIE_NAME
        {
            return Some(v.trim());
        }
    }
    None
}

/// Extractor enforcing valid Bearer token or `HttpOnly` session cookie authentication.
#[derive(Debug, Clone)]
pub struct RequireAuth {
    /// Session ID if authenticated via cookie (`None` when authenticated via Bearer token).
    pub session_id: Option<String>,
}

impl FromRequestParts<DaemonState> for RequireAuth {
    type Rejection = ApiError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &DaemonState,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        let res = if let Some(auth_val) = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            && let Some(bearer) = auth_val
                .strip_prefix("Bearer ")
                .or_else(|| auth_val.strip_prefix("bearer "))
            && state.auth().verify_token(bearer)
        {
            Ok(Self { session_id: None })
        } else if let Some(cookie_hdr) = parts.headers.get(COOKIE).and_then(|h| h.to_str().ok())
            && let Some(sid) = extract_session_cookie(cookie_hdr)
            && state.auth().validate_session(sid)
        {
            Ok(Self {
                session_id: Some(sid.to_string()),
            })
        } else {
            Err(ApiError::unauthorized(
                "Authentication required (provide session cookie or Authorization: Bearer token)",
            ))
        };
        std::future::ready(res)
    }
}
