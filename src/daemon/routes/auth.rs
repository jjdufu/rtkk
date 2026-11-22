//! Login (`POST /api/v1/auth/login`), logout (`POST /api/v1/auth/logout`), and session status (`GET /api/v1/auth/me`).

use crate::daemon::auth::{RequireAuth, SESSION_COOKIE_NAME, SESSION_TTL_SECS};
use crate::daemon::state::{ApiError, DaemonState};
use axum::Json;
use axum::extract::State;
use axum::http::header::SET_COOKIE;
use axum::http::{HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

/// Login request payload submitted to `POST /api/v1/auth/login`.
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    /// High-entropy admin access token.
    pub token: String,
}

/// Response returned by `POST /api/v1/auth/login`.
#[derive(Debug, Serialize)]
pub struct LoginResponse {
    /// Always `true` when `200 OK`.
    pub authenticated: bool,
    /// Cookie validity window in seconds.
    pub expires_in_secs: u64,
}

/// Response returned by `POST /api/v1/auth/logout`.
#[derive(Debug, Serialize)]
pub struct LogoutResponse {
    /// Always `false` after session revocation.
    pub authenticated: bool,
}

/// Authenticated session inspection payload returned by `GET /api/v1/auth/me`.
#[derive(Debug, Serialize)]
pub struct MeResponse {
    /// Always `true` for authenticated callers.
    pub authenticated: bool,
    /// Running `rtkk` binary version.
    pub version: &'static str,
    /// Canonicalized or configured workspace root path.
    pub work_dir: String,
    /// Configured daemon listen address.
    pub bind_addr: String,
}

fn is_https_forwarded(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|p| p.eq_ignore_ascii_case("https"))
}

/// Authenticates an admin token and sets an `HttpOnly; SameSite=Strict` session cookie.
///
/// # Errors
///
/// Returns [`ApiError`] (`401` or `429`) if token validation fails or rate limits trigger.
pub async fn post_login(
    State(state): State<DaemonState>,
    headers: HeaderMap,
    Json(payload): Json<LoginRequest>,
) -> Result<Response, ApiError> {
    let sid = state
        .auth()
        .authenticate_and_create_session(&payload.token)?;
    let secure_attr = if is_https_forwarded(&headers) {
        "; Secure"
    } else {
        ""
    };
    let cookie_str = format!(
        "{SESSION_COOKIE_NAME}={sid}; HttpOnly; SameSite=Strict; Path=/; Max-Age={SESSION_TTL_SECS}{secure_attr}"
    );
    let mut res = Json(LoginResponse {
        authenticated: true,
        expires_in_secs: SESSION_TTL_SECS,
    })
    .into_response();
    if let Ok(val) = HeaderValue::from_str(&cookie_str) {
        res.headers_mut().insert(SET_COOKIE, val);
    }
    Ok(res)
}

/// Revokes the caller's current session cookie and clears it in the browser.
pub async fn post_logout(auth: RequireAuth, State(state): State<DaemonState>) -> Response {
    if let Some(ref sid) = auth.session_id {
        state.auth().revoke_session(sid);
    }
    let clear_cookie =
        format!("{SESSION_COOKIE_NAME}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0");
    let mut res = Json(LogoutResponse {
        authenticated: false,
    })
    .into_response();
    if let Ok(val) = HeaderValue::from_str(&clear_cookie) {
        res.headers_mut().insert(SET_COOKIE, val);
    }
    res
}

/// Returns current session status and workspace metadata (`GET /api/v1/auth/me`).
pub async fn get_me(_auth: RequireAuth, State(state): State<DaemonState>) -> Json<MeResponse> {
    let work_dir = std::fs::canonicalize(state.workspace().root())
        .unwrap_or_else(|_| state.workspace().root().to_path_buf())
        .to_string_lossy()
        .into_owned();
    Json(MeResponse {
        authenticated: true,
        version: env!("CARGO_PKG_VERSION"),
        work_dir,
        bind_addr: state.bind_addr().to_string(),
    })
}
