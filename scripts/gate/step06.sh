#!/usr/bin/env bash
# Stage B · Step 06 Gate: Playbook outline/tagged async execution, PKI API cert lifecycle, and backup archive export (C26, C27).
set -euo pipefail

# shellcheck source=scripts/gate/lib.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

gate_init "step06"
ensure_rtkk_bin

python3 "${REPO_ROOT}/scripts/check_routes.py"

TOKEN_FILE="${GATE_WORK_DIR}/.rtkk-daemon/admin.token"
"$RTKK_BIN" -l "${GATE_TMP_DIR}/daemon.log" serve \
  --listen "$GATE_BIND_ADDR" \
  --work-dir "$GATE_WORK_DIR" \
  --token-file "$TOKEN_FILE" &
DAEMON_PID=$!
gate_register_pid "$DAEMON_PID"
wait_daemon_ready "${GATE_BASE_URL}/api/v1/health" 20

ADMIN_TOKEN="$(tr -d '\r\n' < "$TOKEN_FILE")"
AUTH_HDR="Authorization: Bearer ${ADMIN_TOKEN}"

test_playbook_outline_and_tagged_run() {
  local code
  code="$(http_status GET "${GATE_BASE_URL}/api/v1/playbooks/deploy/outline" -H "$AUTH_HDR")"
  assert_eq "$code" "200" "GET /api/v1/playbooks/deploy/outline must return 200"

  local plays_len has_check_tag
  plays_len="$(json_field "$GATE_HTTP_BODY" '.plays | length')"
  has_check_tag="$(jq '[.all_tags[] | select(. == "check")] | length' "$GATE_HTTP_BODY")"
  if (( plays_len < 5 || has_check_tag < 1 )); then
    echo "Unexpected deploy.yaml outline: plays=${plays_len}, check_tag=${has_check_tag}" >&2
    return 1
  fi

  # Create a local-only tagged playbook inside sandbox configs/download.yaml to verify async tagged run
  local marker_yes="${GATE_TMP_DIR}/tagged-ran.txt"
  local marker_no="${GATE_TMP_DIR}/untagged-skipped.txt"
  cat > "${GATE_WORK_DIR}/configs/download.yaml" <<EOF
- name: Sandbox tagged execution play
  hosts: localhost
  tasks:
    - name: Task matching selected tag
      copy:
        content: "matched\n"
        dest: "${marker_yes}"
        mode: "0644"
      tags: ["gate_selected"]
    - name: Task excluded by tag filter
      copy:
        content: "excluded\n"
        dest: "${marker_no}"
        mode: "0644"
      tags: ["gate_other"]
EOF

  local run_code job_id final_state
  run_code="$(http_status POST "${GATE_BASE_URL}/api/v1/operations/run" \
    -H "$AUTH_HDR" \
    -H "Content-Type: application/json" \
    -d '{"playbook":"download","tags":["gate_selected"],"forks":2}')"
  assert_eq "$run_code" "200" "POST /api/v1/operations/run must return 200"
  job_id="$(json_field "$GATE_HTTP_BODY" '.job_id')"
  final_state="$(wait_job_terminal "$job_id" "$ADMIN_TOKEN" 20)"
  assert_eq "$final_state" "succeeded" "tagged playbook job must reach succeeded"

  assert_file_exists "$marker_yes"
  if [[ -e "$marker_no" ]]; then
    echo "Excluded tag task unexpectedly executed (${marker_no} exists)" >&2
    return 1
  fi
}

test_pki_api_gen_inspect_verify() {
  local ca_code
  ca_code="$(http_status POST "${GATE_BASE_URL}/api/v1/pki/generate" \
    -H "$AUTH_HDR" \
    -H "Content-Type: application/json" \
    -d '{"name":"registry-ca","cn":"rtkk-api-ca","is_ca":true,"sans":[],"validity":"867240h"}')"
  assert_eq "$ca_code" "200" "POST /api/v1/pki/generate (CA) must return 200"

  local leaf_code
  leaf_code="$(http_status POST "${GATE_BASE_URL}/api/v1/pki/generate" \
    -H "$AUTH_HDR" \
    -H "Content-Type: application/json" \
    -d '{"name":"harbor-tls","cn":"10.0.0.10","is_ca":false,"ca_name":"registry-ca","sans":["10.0.0.10","harbor.internal"],"validity":"1y"}')"
  assert_eq "$leaf_code" "200" "POST /api/v1/pki/generate (leaf) must return 200"

  assert_file_mode "${GATE_WORK_DIR}/pki/custom/registry-ca.key" "600"
  assert_file_mode "${GATE_WORK_DIR}/pki/custom/harbor-tls.key" "600"

  local list_code
  list_code="$(http_status GET "${GATE_BASE_URL}/api/v1/pki/certificates" -H "$AUTH_HDR")"
  assert_eq "$list_code" "200" "GET /api/v1/pki/certificates must return 200"
  assert_not_contains "$(cat "$GATE_HTTP_BODY")" "PRIVATE KEY" "PKI list API must never leak private key PEM"
  assert_contains "$(cat "$GATE_HTTP_BODY")" "harbor.internal" "PKI list API must expose parsed SAN metadata"

  local ver_code
  ver_code="$(http_status POST "${GATE_BASE_URL}/api/v1/pki/verify" \
    -H "$AUTH_HDR" \
    -H "Content-Type: application/json" \
    -d '{"cert_rel_path":"custom/harbor-tls.crt","key_rel_path":"custom/harbor-tls.key","ca_rel_path":"custom/registry-ca.crt"}')"
  assert_eq "$ver_code" "200" "POST /api/v1/pki/verify must return 200"
  assert_eq "$(json_field "$GATE_HTTP_BODY" '.valid')" "true" "leaf cert + key + CA chain must verify valid=true"
}

test_backup_archive_stream() {
  mkdir -p -m 0700 "${GATE_WORK_DIR}/rtkk_bak/c23-p61/etc/kubernetes"
  printf 'apiVersion: v1\nkind: Config\nclusters: []\n' > "${GATE_WORK_DIR}/rtkk_bak/c23-p61/etc/kubernetes/admin.conf"
  chmod 0600 "${GATE_WORK_DIR}/rtkk_bak/c23-p61/etc/kubernetes/admin.conf"

  local list_code
  list_code="$(http_status GET "${GATE_BASE_URL}/api/v1/backups" -H "$AUTH_HDR")"
  assert_eq "$list_code" "200" "GET /api/v1/backups must return 200"
  assert_contains "$(cat "$GATE_HTTP_BODY")" "admin.conf" "backups listing must include admin.conf"

  local archive_tar="${GATE_TMP_DIR}/exported-backups.tar.gz"
  local extract_dir="${GATE_TMP_DIR}/extracted-backups"
  mkdir -p "$extract_dir"

  curl -fsS -H "$AUTH_HDR" -o "$archive_tar" "${GATE_BASE_URL}/api/v1/backups/archive"
  tar -xzf "$archive_tar" -C "$extract_dir"
  assert_file_exists "${extract_dir}/c23-p61/etc/kubernetes/admin.conf"
}

run_assertion "playbook-outline-and-tagged-run" test_playbook_outline_and_tagged_run
run_assertion "pki-api-gen-inspect-verify" test_pki_api_gen_inspect_verify
run_assertion "backup-archive-stream" test_backup_archive_stream

gate_pass "step06"
