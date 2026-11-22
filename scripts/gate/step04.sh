#!/usr/bin/env bash
# Stage B · Step 04 Gate: Inventory secret masking/preservation, DNS hijack hosts.toml, and live template preview (C21, C22, C23).
set -euo pipefail

# shellcheck source=scripts/gate/lib.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

gate_init "step04"
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

SEED_SSH_SECRET="gate-ssh-secret-do-not-leak-9182"
SEED_REG_SECRET="gate-reg-secret-do-not-leak-7364"
INV_FILE="${GATE_WORK_DIR}/configs/inventory.yaml"

sed -i.orig \
  -e "s/ansible_ssh_pass: \"change-me\"/ansible_ssh_pass: \"${SEED_SSH_SECRET}\"/" \
  -e "s/password: \"change-me\"/password: \"${SEED_REG_SECRET}\"/" \
  "$INV_FILE"
rm -f "${INV_FILE}.orig"

test_inventory_secret_masked() {
  local code
  code="$(http_status GET "${GATE_BASE_URL}/api/v1/inventory" -H "$AUTH_HDR")"
  assert_eq "$code" "200" "GET /api/v1/inventory must return 200"

  local raw_body
  raw_body="$(cat "$GATE_HTTP_BODY")"
  assert_not_contains "$raw_body" "$SEED_SSH_SECRET" "API response must never leak ansible_ssh_pass"
  assert_not_contains "$raw_body" "$SEED_REG_SECRET" "API response must never leak registry.password"
  assert_eq "$(json_field "$GATE_HTTP_BODY" '.inventory.vars.ansible_ssh_pass')" "********" "ansible_ssh_pass must be masked"
  assert_eq "$(json_field "$GATE_HTTP_BODY" '.inventory.vars.registry.password')" "********" "registry.password must be masked"
  assert_eq "$(json_field "$GATE_HTTP_BODY" '.inventory.vars.registry.auth_basic')" "********" "registry.auth_basic must be masked"
}

test_inventory_preserve_unmodified_secret() {
  # Fetch masked YAML, update dns_upstream while leaving ******** untouched, and PUT back
  http_status GET "${GATE_BASE_URL}/api/v1/inventory" -H "$AUTH_HDR" >/dev/null
  local masked_yaml updated_yaml
  masked_yaml="$(json_field "$GATE_HTTP_BODY" '.masked_yaml')"
  updated_yaml="${masked_yaml/10.0.0.111/10.0.0.222}"

  local put_code
  put_code="$(http_status PUT "${GATE_BASE_URL}/api/v1/inventory" \
    -H "$AUTH_HDR" \
    -H "Content-Type: application/json" \
    -d "$(jq -nc --arg y "$updated_yaml" '{mode: "yaml", raw_yaml: $y}')")"
  assert_eq "$put_code" "200" "PUT /api/v1/inventory must return 200"
  assert_file_exists "${GATE_WORK_DIR}/configs/inventory.yaml.bak"

  local disk_text
  disk_text="$(cat "$INV_FILE")"
  assert_contains "$disk_text" "$SEED_SSH_SECRET" "unmodified ssh secret must be preserved on disk"
  assert_contains "$disk_text" "$SEED_REG_SECRET" "unmodified registry secret must be preserved on disk"
  assert_contains "$disk_text" "10.0.0.222" "updated dns_upstream must be written to disk"
  assert_not_contains "$disk_text" "********" "mask placeholder must never be written to disk"

  # Verify read-only validate endpoint
  local val_code
  val_code="$(http_status POST "${GATE_BASE_URL}/api/v1/inventory/validate" \
    -H "$AUTH_HDR" \
    -H "Content-Type: application/json" \
    -d "$(jq -nc --arg y "$updated_yaml" '{mode: "yaml", raw_yaml: $y}')")"
  assert_eq "$val_code" "200" "POST /api/v1/inventory/validate must return 200"
  assert_eq "$(json_field "$GATE_HTTP_BODY" '.valid')" "true" "candidate inventory must validate cleanly"
}

test_public_registries_hijack_hosts_toml() {
  (
    cd "$REPO_ROOT"
    cargo test --test configs_render_tests public_registries_hijack_hosts_toml --locked -- --exact
  )
}

test_template_preview_render_ok() {
  local code
  code="$(http_status POST "${GATE_BASE_URL}/api/v1/templates/render" \
    -H "$AUTH_HDR" \
    -H "Content-Type: application/json" \
    -d '{"host":"c23-p61","templates":["all"]}')"
  assert_eq "$code" "200" "POST /api/v1/templates/render must return 200"

  local count invalid_count
  count="$(json_field "$GATE_HTTP_BODY" '.files | length')"
  if (( count < 7 )); then
    echo "Expected at least 7 rendered templates (6 .gtmpl + images.yaml), got ${count}" >&2
    return 1
  fi
  invalid_count="$(jq '[.files[] | select(.syntax_valid != true)] | length' "$GATE_HTTP_BODY")"
  assert_eq "$invalid_count" "0" "all rendered preview files must pass syntax validation"
}

run_assertion "inventory-secret-masked" test_inventory_secret_masked
run_assertion "inventory-preserve-unmodified-secret" test_inventory_preserve_unmodified_secret
run_assertion "public-registries-hijack-hosts-toml" test_public_registries_hijack_hosts_toml
run_assertion "template-preview-render-ok" test_template_preview_render_ok

gate_pass "step04"
