#!/usr/bin/env bash
# Live Virtual VPS Cluster Acceptance Gate: Debian 13 multi-node SSH/sudo/su, Harbor sync, and HA K8s v1.37.2 E2E.
set -euo pipefail

# shellcheck source=scripts/gate/lib.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

if [[ -z "${RTKK_VPS_INVENTORY:-}" || ! -f "${RTKK_VPS_INVENTORY:-}" ]]; then
  echo "Usage: RTKK_VPS_INVENTORY=/path/to/vps-inventory.yaml bash scripts/gate/vps-cluster-e2e.sh" >&2
  exit 2
fi

gate_init "vps-e2e"
ensure_rtkk_bin

mkdir -p "${GATE_WORK_DIR}/configs"
cp "$RTKK_VPS_INVENTORY" "${GATE_WORK_DIR}/configs/inventory.yaml"

VPS_DOWNLOADS_SRC="${RTKK_VPS_DOWNLOADS_DIR:-${REPO_ROOT}/downloads}"
if [[ -d "$VPS_DOWNLOADS_SRC" && ! -e "${GATE_WORK_DIR}/downloads" ]]; then
  ln -s "$(cd "$VPS_DOWNLOADS_SRC" && pwd -P)" "${GATE_WORK_DIR}/downloads"
fi

TOKEN_FILE="${GATE_WORK_DIR}/.rtkk-daemon/admin.token"
(
  cd "$GATE_WORK_DIR"
  exec "$RTKK_BIN" -l "${GATE_TMP_DIR}/vps-daemon.log" serve \
    --listen "$GATE_BIND_ADDR" \
    --work-dir "$GATE_WORK_DIR" \
    --token-file "$TOKEN_FILE"
) &
DAEMON_PID=$!
gate_register_pid "$DAEMON_PID"

wait_daemon_ready "${GATE_BASE_URL}/api/v1/health" 20
ADMIN_TOKEN="$(tr -d '\r\n' < "$TOKEN_FILE")"
AUTH_HDR="Authorization: Bearer ${ADMIN_TOKEN}"

test_vps_compose_daemon_up() {
  "$RTKK_BIN" healthcheck --url "${GATE_BASE_URL}/api/v1/health"
  assert_file_mode "$TOKEN_FILE" "600"
}

test_vps_ssh_and_become_probe() {
  local code
  code="$(http_status POST "${GATE_BASE_URL}/api/v1/inventory/probe" \
    -H "$AUTH_HDR" \
    -H "Content-Type: application/json" \
    -d '{"hosts":[]}')"
  assert_eq "$code" "200" "POST /api/v1/inventory/probe must return 200"

  local failed_nodes
  failed_nodes="$(jq '[.results[] | select(.reachable != true or .uid != 0)] | length' "$GATE_HTTP_BODY")"
  assert_eq "$failed_nodes" "0" "all virtual VPS nodes must be reachable with root/become uid=0"
}

test_vps_registry_ping_and_push() {
  local code
  code="$(http_status POST "${GATE_BASE_URL}/api/v1/registry/ping" \
    -H "$AUTH_HDR" \
    -H "Content-Type: application/json" \
    -d '{}')"
  assert_eq "$code" "200" "POST /api/v1/registry/ping must return 200"
  assert_eq "$(json_field "$GATE_HTTP_BODY" '.reachable')" "true" "VPS private Harbor/OCI registry must be reachable"

  local push_code job_id state
  push_code="$(http_status POST "${GATE_BASE_URL}/api/v1/registry/push" \
    -H "$AUTH_HDR" \
    -H "Content-Type: application/json" \
    -d '{"image_names":[]}')"
  assert_eq "$push_code" "200" "POST /api/v1/registry/push must return 200"
  job_id="$(json_field "$GATE_HTTP_BODY" '.job_id')"
  state="$(wait_job_terminal "$job_id" "$ADMIN_TOKEN" 600)"
  assert_eq "$state" "succeeded" "Harbor batch image push must succeed"
}

test_vps_prep_hosts_idempotent() {
  local code job_id state
  code="$(http_status POST "${GATE_BASE_URL}/api/v1/operations/prep-hosts" \
    -H "$AUTH_HDR" \
    -H "Content-Type: application/json" \
    -d '{"best_effort":false}')"
  assert_eq "$code" "200" "POST /api/v1/operations/prep-hosts must return 200"
  job_id="$(json_field "$GATE_HTTP_BODY" '.job_id')"
  state="$(wait_job_terminal "$job_id" "$ADMIN_TOKEN" 120)"
  assert_eq "$state" "succeeded" "prep hosts across VPS nodes must succeed"
}

test_vps_preflight_and_dns_hijack() {
  local tag code job_id state
  for tag in "check" "preflight"; do
    code="$(http_status POST "${GATE_BASE_URL}/api/v1/operations/run" \
      -H "$AUTH_HDR" \
      -H "Content-Type: application/json" \
      -d "$(jq -nc --arg t "$tag" '{playbook: "deploy", tags: [$t], forks: 10}')")"
    assert_eq "$code" "200" "POST /api/v1/operations/run (${tag}) must return 200"
    job_id="$(json_field "$GATE_HTTP_BODY" '.job_id')"
    state="$(wait_job_terminal "$job_id" "$ADMIN_TOKEN" 300)"
    assert_eq "$state" "succeeded" "VPS ${tag} play must succeed"
  done
}

test_vps_ha_k8s_deploy_ready() {
  local code job_id state
  code="$(http_status POST "${GATE_BASE_URL}/api/v1/operations/run" \
    -H "$AUTH_HDR" \
    -H "Content-Type: application/json" \
    -d '{"playbook":"deploy","tags":[],"forks":10}')"
  assert_eq "$code" "200" "POST /api/v1/operations/run (full deploy) must return 200"
  job_id="$(json_field "$GATE_HTTP_BODY" '.job_id')"
  state="$(wait_job_terminal "$job_id" "$ADMIN_TOKEN" 1800)"
  assert_eq "$state" "succeeded" "Full HA Kubernetes v1.37.2 deployment must succeed"
}

test_vps_pki_and_backup_export() {
  local archive_tar="${GATE_TMP_DIR}/vps-backups.tar.gz"
  curl -fsS -H "$AUTH_HDR" -o "$archive_tar" "${GATE_BASE_URL}/api/v1/backups/archive"
  assert_file_exists "$archive_tar"
  tar -tzf "$archive_tar" | grep -q "admin.conf"
}

test_vps_cleanup_and_redeploy_option() {
  if [[ "${RTKK_VPS_RUN_CLEANUP:-0}" != "1" ]]; then
    return 0
  fi
  local code job_id state
  code="$(http_status POST "${GATE_BASE_URL}/api/v1/operations/run" \
    -H "$AUTH_HDR" \
    -H "Content-Type: application/json" \
    -d '{"playbook":"cleanup","tags":[],"forks":10}')"
  assert_eq "$code" "200" "POST /api/v1/operations/run (cleanup) must return 200"
  job_id="$(json_field "$GATE_HTTP_BODY" '.job_id')"
  state="$(wait_job_terminal "$job_id" "$ADMIN_TOKEN" 600)"
  assert_eq "$state" "succeeded" "VPS cleanup.yaml reset must succeed"
}

run_assertion "vps-compose-daemon-up" test_vps_compose_daemon_up
run_assertion "vps-ssh-and-become-probe" test_vps_ssh_and_become_probe
run_assertion "vps-registry-ping-and-push" test_vps_registry_ping_and_push
run_assertion "vps-prep-hosts-idempotent" test_vps_prep_hosts_idempotent
run_assertion "vps-preflight-and-dns-hijack" test_vps_preflight_and_dns_hijack
run_assertion "vps-ha-k8s-deploy-ready" test_vps_ha_k8s_deploy_ready
run_assertion "vps-pki-and-backup-export" test_vps_pki_and_backup_export
run_assertion "vps-cleanup-and-redeploy-option" test_vps_cleanup_and_redeploy_option

gate_pass "vps-e2e"
