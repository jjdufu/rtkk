#!/usr/bin/env bash
# Stage B · Step 02 Gate: Daemon (:19912), token/session auth, mutating job mutex, and SSE replay (C17, C18, C19).
set -euo pipefail

# shellcheck source=scripts/gate/lib.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

gate_init "step02"
ensure_rtkk_bin

DAEMON_LOG="${GATE_TMP_DIR}/daemon.log"
TOKEN_FILE="${GATE_WORK_DIR}/.rtkk-daemon/admin.token"

"$RTKK_BIN" -l "$DAEMON_LOG" serve \
  --listen "$GATE_BIND_ADDR" \
  --work-dir "$GATE_WORK_DIR" \
  --token-file "$TOKEN_FILE" &
DAEMON_PID=$!
gate_register_pid "$DAEMON_PID"

ADMIN_TOKEN_PATH="$TOKEN_FILE"

test_daemon_healthcheck_ok() {
  wait_daemon_ready "${GATE_BASE_URL}/api/v1/health" 20

  "$RTKK_BIN" healthcheck --url "${GATE_BASE_URL}/api/v1/health"

  local status_code
  status_code="$(http_status GET "${GATE_BASE_URL}/api/v1/health")"
  assert_eq "$status_code" "200" "GET /api/v1/health must return 200"
  assert_eq "$(json_field "$GATE_HTTP_BODY" '.status')" "ok" "health status field must equal ok"

  assert_file_mode "${GATE_WORK_DIR}/.rtkk-daemon" "700"
  assert_file_mode "$ADMIN_TOKEN_PATH" "600"
  local admin_token
  admin_token="$(tr -d '\r\n' < "$ADMIN_TOKEN_PATH")"
  if [[ "${#admin_token}" -lt 32 ]]; then
    echo "Generated admin token is unexpectedly short (${#admin_token} chars)" >&2
    return 1
  fi
}

test_unauth_401_blocked() {
  local ep code
  for ep in "/api/v1/auth/me" "/api/v1/system/overview" "/api/v1/jobs"; do
    code="$(http_status GET "${GATE_BASE_URL}${ep}")"
    assert_eq "$code" "401" "unauthenticated ${ep} must return 401"
    assert_eq "$(json_field "$GATE_HTTP_BODY" '.code')" "UNAUTHORIZED" "401 body must carry code=UNAUTHORIZED"
    json_field "$GATE_HTTP_BODY" '.request_id' >/dev/null
  done

  # Unknown /api/* routes must return 404 JSON rather than SPA HTML fallback
  code="$(http_status GET "${GATE_BASE_URL}/api/v1/unknown-endpoint")"
  assert_eq "$code" "404" "unknown API route must return 404"
  assert_eq "$(json_field "$GATE_HTTP_BODY" '.code')" "NOT_FOUND" "404 API body must carry code=NOT_FOUND"
}

test_login_session_ok() {
  local admin_token
  admin_token="$(tr -d '\r\n' < "$ADMIN_TOKEN_PATH")"

  local bad_code
  bad_code="$(http_status POST "${GATE_BASE_URL}/api/v1/auth/login" \
    -H "Content-Type: application/json" \
    -d '{"token":"wrong-token-value-for-gate-test"}')"
  assert_eq "$bad_code" "401" "invalid login token must return 401"

  local ok_code
  ok_code="$(http_status POST "${GATE_BASE_URL}/api/v1/auth/login" \
    -c "$GATE_COOKIE_JAR" \
    -H "Content-Type: application/json" \
    -d "$(jq -nc --arg t "$admin_token" '{token: $t}')")"
  assert_eq "$ok_code" "200" "valid token login must return 200"
  assert_eq "$(json_field "$GATE_HTTP_BODY" '.authenticated')" "true" "login must return authenticated=true"
  assert_contains "$(tr '[:upper:]' '[:lower:]' < "$GATE_HTTP_HEADERS")" "httponly" "session cookie must be HttpOnly"

  local me_cookie_code
  me_cookie_code="$(http_status GET "${GATE_BASE_URL}/api/v1/auth/me" -b "$GATE_COOKIE_JAR")"
  assert_eq "$me_cookie_code" "200" "session cookie must authenticate /api/v1/auth/me"

  local ov_bearer_code
  ov_bearer_code="$(http_status GET "${GATE_BASE_URL}/api/v1/system/overview" \
    -H "Authorization: Bearer ${admin_token}")"
  assert_eq "$ov_bearer_code" "200" "Bearer header must authenticate /api/v1/system/overview"
}

test_mutating_job_mutex() {
  (
    cd "$REPO_ROOT"
    cargo test --test daemon_core_tests mutating_job_mutex_blocks_concurrent_writes --locked -- --exact
  )
}

test_sse_replay_complete() {
  (
    cd "$REPO_ROOT"
    cargo test --test daemon_core_tests sse_replays_persisted_jsonl_and_tails_live_events --locked -- --exact
  )

  # Verify clean SIGTERM shutdown of live daemon
  kill -TERM "$DAEMON_PID"
  local waited=0
  while kill -0 "$DAEMON_PID" 2>/dev/null && (( waited < 50 )); do
    sleep 0.1
    waited=$((waited + 1))
  done
  if kill -0 "$DAEMON_PID" 2>/dev/null; then
    echo "Daemon did not exit within 5s after SIGTERM" >&2
    return 1
  fi
}

run_assertion "daemon-healthcheck-ok" test_daemon_healthcheck_ok
run_assertion "unauth-401-blocked" test_unauth_401_blocked
run_assertion "login-session-ok" test_login_session_ok
run_assertion "mutating-job-mutex" test_mutating_job_mutex
run_assertion "sse-replay-complete" test_sse_replay_complete

gate_pass "step02"
