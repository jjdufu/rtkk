#!/usr/bin/env bash
# Stage B · Step 07 Gate: Standalone Docker Compose (:19912) lifecycle & offline Park UI Playwright E2E (C28, C29).
set -euo pipefail

# shellcheck source=scripts/gate/lib.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

gate_init "step07"

assert_file_exists "${REPO_ROOT}/deploy/docker/Dockerfile"
assert_file_exists "${REPO_ROOT}/deploy/compose/config/.env"
assert_file_exists "${REPO_ROOT}/deploy/compose/config/compose.yml"
assert_file_exists "${REPO_ROOT}/deploy/compose/lib/common.sh"
for s in env start stop down status remove; do
  assert_file_exists "${REPO_ROOT}/deploy/compose/scripts/${s}.sh"
  bash -n "${REPO_ROOT}/deploy/compose/scripts/${s}.sh"
done

python3 "${REPO_ROOT}/scripts/configs-guard.py"
python3 "${REPO_ROOT}/scripts/web-guard.py"

(
  cd "${REPO_ROOT}/web"
  npm run typecheck
  npm run build
)

(
  cd "$REPO_ROOT"
  cargo build --locked
)

test_playwright_parkui_offline_e2e() {
  local token_file="${GATE_WORK_DIR}/.rtkk-daemon/admin.token"
  "$RTKK_BIN" -l "${GATE_TMP_DIR}/daemon-e2e.log" serve \
    --listen "$GATE_BIND_ADDR" \
    --work-dir "$GATE_WORK_DIR" \
    --token-file "$token_file" &
  local pid=$!
  gate_register_pid "$pid"
  wait_daemon_ready "${GATE_BASE_URL}/api/v1/health" 20

  local admin_token
  admin_token="$(tr -d '\r\n' < "$token_file")"

  (
    cd "${REPO_ROOT}/web"
    RTKK_E2E_BASE_URL="$GATE_BASE_URL" \
    RTKK_E2E_TOKEN="$admin_token" \
    npx playwright test e2e/parkui-and-journey.spec.ts
  )

  kill -TERM "$pid"
  wait "$pid" || true
}

test_compose_lifecycle_19912() {
  if ! command -v docker >/dev/null 2>&1; then
    echo "Docker daemon CLI is required for compose-lifecycle-19912 gate" >&2
    return 1
  fi

  # Copy compose stack into sandbox so runtime data/logs/backups never dirty working tree
  local compose_sb="${GATE_TMP_DIR}/compose-stack"
  cp -R "${REPO_ROOT}/deploy/compose" "$compose_sb"

  (
    cd "$REPO_ROOT"
    cargo build --release --locked
    docker build -f deploy/docker/Dockerfile -t dex2all/rtkk:gate-local .
  )

  cat > "${compose_sb}/config/.env" <<EOF
RTKK_VERSION=gate-local
RTKK_PORT=${GATE_PORT}
TZ=Asia/Shanghai
EOF

  bash "${compose_sb}/scripts/start.sh"
  wait_daemon_ready "http://127.0.0.1:${GATE_PORT}/api/v1/health" 30

  local status_out
  status_out="$(bash "${compose_sb}/scripts/status.sh")"
  assert_contains "$status_out" "${GATE_PORT}" "status.sh must report mapped host port"

  assert_file_exists "${compose_sb}/data/configs/inventory.yaml"
  assert_file_exists "${compose_sb}/data/.rtkk-daemon/admin.token"

  bash "${compose_sb}/scripts/down.sh"
  assert_file_exists "${compose_sb}/data/configs/inventory.yaml"
  bash "${compose_sb}/scripts/remove.sh"
}

run_assertion "playwright-parkui-offline-e2e" test_playwright_parkui_offline_e2e
run_assertion "compose-lifecycle-19912" test_compose_lifecycle_19912

gate_pass "step07"
