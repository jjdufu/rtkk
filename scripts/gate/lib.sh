#!/usr/bin/env bash
# Shared assertion, port-slot isolation, and sandbox lifecycle library for G3 gate scripts.
set -euo pipefail

if [[ -f "$HOME/.cargo/env" ]]; then
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
WT_SLOT="${WT_SLOT:-0}"

if ! [[ "$WT_SLOT" =~ ^[0-4]$ ]]; then
  echo "FAIL gate/lib.sh: WT_SLOT must be an integer in 0..4, got '$WT_SLOT'" >&2
  exit 2
fi

GATE_PORT=$((19912 + WT_SLOT * 10000))
GATE_BIND_ADDR="127.0.0.1:${GATE_PORT}"
GATE_BASE_URL="http://${GATE_BIND_ADDR}"
GATE_WORK_DIR="${REPO_ROOT}/.gate/work-slot${WT_SLOT}"
RTKK_BIN="${RTKK_BIN:-${REPO_ROOT}/target/debug/rtkk}"

GATE_STEP="unset"
GATE_TMP_DIR=""
GATE_HTTP_BODY=""
GATE_HTTP_HEADERS=""
GATE_COOKIE_JAR=""
GATE_REGISTERED_PIDS=()

export REPO_ROOT WT_SLOT GATE_PORT GATE_BIND_ADDR GATE_BASE_URL GATE_WORK_DIR RTKK_BIN

gate_cleanup() {
  local pid
  for pid in "${GATE_REGISTERED_PIDS[@]+"${GATE_REGISTERED_PIDS[@]}"}"; do
    if kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
      wait "$pid" 2>/dev/null || true
    fi
  done
  if [[ -n "$GATE_TMP_DIR" && -d "$GATE_TMP_DIR" ]]; then
    rm -rf "$GATE_TMP_DIR"
  fi
}

gate_init() {
  GATE_STEP="$1"
  GATE_TMP_DIR="${REPO_ROOT}/.gate/tmp-${GATE_STEP}-slot${WT_SLOT}"
  rm -rf "$GATE_TMP_DIR" "$GATE_WORK_DIR"
  mkdir -p "$GATE_TMP_DIR" "$GATE_WORK_DIR"
  GATE_HTTP_BODY="${GATE_TMP_DIR}/http-body.out"
  GATE_HTTP_HEADERS="${GATE_TMP_DIR}/http-headers.out"
  GATE_COOKIE_JAR="${GATE_TMP_DIR}/cookies.txt"
  trap gate_cleanup EXIT INT TERM
}

gate_register_pid() {
  GATE_REGISTERED_PIDS+=("$1")
}

ensure_rtkk_bin() {
  (cd "$REPO_ROOT" && cargo build --locked)
}

run_assertion() {
  local assertion_name="$1"
  shift
  local rc=0
  set +e
  (
    set -euo pipefail
    "$@"
  )
  rc=$?
  set -e
  if [[ "$rc" -eq 0 ]]; then
    echo "PASS [${GATE_STEP}] ${assertion_name}"
  else
    echo "FAIL [${GATE_STEP}] ${assertion_name}" >&2
    exit 1
  fi
}

assert_eq() {
  local actual="$1"
  local expected="$2"
  local msg="${3:-values did not match}"
  if [[ "$actual" != "$expected" ]]; then
    echo "Assertion failed (${msg}): expected '${expected}', got '${actual}'" >&2
    return 1
  fi
}

assert_contains() {
  local haystack="$1"
  local needle="$2"
  local msg="${3:-missing expected substring}"
  if [[ "$haystack" != *"$needle"* ]]; then
    echo "Assertion failed (${msg}): expected to find '${needle}'" >&2
    return 1
  fi
}

assert_not_contains() {
  local haystack="$1"
  local needle="$2"
  local msg="${3:-unexpected forbidden substring present}"
  if [[ "$haystack" == *"$needle"* ]]; then
    echo "Assertion failed (${msg}): forbidden substring '${needle}' was present" >&2
    return 1
  fi
}

assert_file_exists() {
  local path="$1"
  if [[ ! -f "$path" ]]; then
    echo "Assertion failed: expected file '${path}' to exist" >&2
    return 1
  fi
}

file_mode_octal() {
  local path="$1"
  if stat -c '%a' "$path" >/dev/null 2>&1; then
    stat -c '%a' "$path"
  else
    stat -f '%Lp' "$path"
  fi
}

assert_file_mode() {
  local path="$1"
  local expected_octal="$2"
  if [[ ! -e "$path" ]]; then
    echo "Assertion failed: expected path '${path}' to exist" >&2
    return 1
  fi
  local actual_octal
  actual_octal="$(file_mode_octal "$path")"
  if [[ "$actual_octal" != "$expected_octal" ]]; then
    echo "Assertion failed: expected mode ${expected_octal} on '${path}', got ${actual_octal}" >&2
    return 1
  fi
}

sha256_hex() {
  local path="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$path" | awk '{print $1}'
  else
    shasum -a 256 "$path" | awk '{print $1}'
  fi
}

http_status() {
  local method="$1"
  local url="$2"
  shift 2
  : > "$GATE_HTTP_BODY"
  : > "$GATE_HTTP_HEADERS"
  curl -sS \
    -X "$method" \
    -D "$GATE_HTTP_HEADERS" \
    -o "$GATE_HTTP_BODY" \
    -w "%{http_code}" \
    "$@" \
    "$url"
}

json_field() {
  local source_file="$1"
  local jq_expr="$2"
  jq -er "$jq_expr" "$source_file"
}

wait_daemon_ready() {
  local health_url="${1:-${GATE_BASE_URL}/api/v1/health}"
  local timeout_secs="${2:-20}"
  local elapsed=0
  while (( elapsed < timeout_secs * 10 )); do
    if curl -fsS "$health_url" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
    elapsed=$((elapsed + 1))
  done
  echo "Timed out waiting for daemon health endpoint: ${health_url}" >&2
  return 1
}

wait_job_terminal() {
  local job_id="$1"
  local auth_token="$2"
  local timeout_secs="${3:-30}"
  local elapsed=0
  local status=""
  while (( elapsed < timeout_secs * 5 )); do
    local code
    code="$(http_status GET "${GATE_BASE_URL}/api/v1/jobs/${job_id}" -H "Authorization: Bearer ${auth_token}")"
    if [[ "$code" == "200" ]]; then
      status="$(jq -r '.status // empty' "$GATE_HTTP_BODY")"
      case "$status" in
        succeeded|failed|cancelled)
          printf '%s' "$status"
          return 0
          ;;
      esac
    fi
    sleep 0.2
    elapsed=$((elapsed + 1))
  done
  echo "Timed out waiting for job ${job_id} to reach terminal state (last=${status})" >&2
  return 1
}

gate_pass() {
  local step_name="${1:-$GATE_STEP}"
  echo "GATE ${step_name} PASS"
}
