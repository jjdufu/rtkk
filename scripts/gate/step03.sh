#!/usr/bin/env bash
# Stage B · Step 03 Gate: Park UI design system, offline bundle, embedded SPA & Gzip/ETag 304 (C20).
set -euo pipefail

# shellcheck source=scripts/gate/lib.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

gate_init "step03"

if [[ ! -d "${REPO_ROOT}/web/src" ]]; then
  echo "FAIL [step03] web/src directory does not exist yet" >&2
  exit 1
fi

python3 "${REPO_ROOT}/scripts/web-guard.py"

(
  cd "${REPO_ROOT}/web"
  npm run typecheck
  npm run build
)

assert_file_exists "${REPO_ROOT}/web/dist/index.html"

# Rebuild rtkk so build.rs embeds the freshly compiled web/dist/ assets
(
  cd "$REPO_ROOT"
  cargo build --locked
)

TOKEN_FILE="${GATE_WORK_DIR}/.rtkk-daemon/admin.token"
"$RTKK_BIN" -l "${GATE_TMP_DIR}/daemon.log" serve \
  --listen "$GATE_BIND_ADDR" \
  --work-dir "$GATE_WORK_DIR" \
  --token-file "$TOKEN_FILE" &
DAEMON_PID=$!
gate_register_pid "$DAEMON_PID"
wait_daemon_ready "${GATE_BASE_URL}/api/v1/health" 20

ASSET_PATH_FILE="${GATE_TMP_DIR}/embedded-asset.path"

test_spa_embedded_index() {
  local root_code
  root_code="$(http_status GET "${GATE_BASE_URL}/")"
  assert_eq "$root_code" "200" "GET / must serve embedded index.html"
  local html
  html="$(cat "$GATE_HTTP_BODY")"
  assert_contains "$html" "<div id=\"root\">" "embedded index.html must contain React root mount"
  assert_not_contains "$html" "https://fonts.googleapis.com" "embedded HTML must be 100% offline"

  local asset_path
  asset_path="$(grep -Eo '/assets/[^"'\'' >]+\.(js|css)' "$GATE_HTTP_BODY" | head -n 1)"
  if [[ -z "$asset_path" ]]; then
    echo "Failed to locate hashed /assets/*.js or *.css reference in embedded index.html" >&2
    return 1
  fi
  printf '%s' "$asset_path" > "$ASSET_PATH_FILE"

  # Deep SPA client route must also return index.html (200)
  local deep_code
  deep_code="$(http_status GET "${GATE_BASE_URL}/inventory")"
  assert_eq "$deep_code" "200" "SPA client route /inventory must fallback to index.html"
  assert_contains "$(cat "$GATE_HTTP_BODY")" "<div id=\"root\">" "SPA fallback must return index.html"
}

test_spa_gzip_etag() {
  local asset_path
  asset_path="$(cat "$ASSET_PATH_FILE")"
  local asset_url="${GATE_BASE_URL}${asset_path}"
  local code
  code="$(http_status GET "$asset_url" -H "Accept-Encoding: gzip")"
  assert_eq "$code" "200" "embedded static asset must return 200"

  local hdrs_lower
  hdrs_lower="$(tr '[:upper:]' '[:lower:]' < "$GATE_HTTP_HEADERS")"
  assert_contains "$hdrs_lower" "content-encoding: gzip" "asset response must negotiate gzip encoding"
  assert_contains "$hdrs_lower" "etag:" "asset response must include SHA256 ETag header"

  local etag_val
  etag_val="$(awk 'tolower($1) == "etag:" { sub(/\r$/, "", $2); print $2; exit }' "$GATE_HTTP_HEADERS")"
  if [[ -z "$etag_val" ]]; then
    echo "ETag value was empty on ${asset_path}" >&2
    return 1
  fi

  local not_mod_code
  not_mod_code="$(http_status GET "$asset_url" -H "If-None-Match: ${etag_val}")"
  assert_eq "$not_mod_code" "304" "matching If-None-Match ETag must return 304 Not Modified"
}

run_assertion "spa-embedded-index" test_spa_embedded_index
run_assertion "spa-gzip-etag" test_spa_gzip_etag

gate_pass "step03"
