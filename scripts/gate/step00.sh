#!/usr/bin/env bash
# Stage A · Step 00 Gate: Phase 0 CLI baseline end-to-end verification (C01, C07, C14).
set -euo pipefail

# shellcheck source=scripts/gate/lib.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

gate_init "step00"
ensure_rtkk_bin

test_cli_version() {
  local cargo_ver
  cargo_ver="$(sed -n 's/^version = "\([^"]*\)".*/\1/p' "${REPO_ROOT}/Cargo.toml" | head -n 1)"
  local out
  out="$("$RTKK_BIN" --version)"
  assert_contains "$out" "rtkk ${cargo_ver}" "CLI --version must match Cargo.toml version"
}

test_cert_99y_and_verify() {
  local pki_dir="${GATE_TMP_DIR}/pki"
  mkdir -p -m 0700 "$pki_dir"

  local ca_key="${pki_dir}/ca.key"
  local ca_crt="${pki_dir}/ca.crt"
  local leaf_key="${pki_dir}/registry.key"
  local leaf_crt="${pki_dir}/registry.crt"

  "$RTKK_BIN" -l "${GATE_TMP_DIR}/cert-ca.log" cert gen \
    --cn "rtkk-gate-root-ca" \
    --out-key "$ca_key" \
    --out-cert "$ca_crt" \
    --ca

  assert_file_mode "$ca_key" "600"
  assert_file_mode "$ca_crt" "644"

  "$RTKK_BIN" -l "${GATE_TMP_DIR}/cert-leaf.log" cert gen \
    --cn "127.0.0.1" \
    --out-key "$leaf_key" \
    --out-cert "$leaf_crt" \
    --ca-key "$ca_key" \
    --ca-cert "$ca_crt" \
    --san "127.0.0.1" \
    --san "registry.internal" \
    --validity "1y"

  assert_file_mode "$leaf_key" "600"
  assert_file_mode "$leaf_crt" "644"

  local verify_out
  verify_out="$("$RTKK_BIN" -l "${GATE_TMP_DIR}/cert-verify.log" cert verify "$leaf_crt" --key "$leaf_key" --ca "$ca_crt")"
  assert_contains "$verify_out" "127.0.0.1" "cert verify output must include leaf CN/SAN"
}

test_cli_local_playbook() {
  local sb="${GATE_TMP_DIR}/playbook-sb"
  mkdir -p "$sb"

  cat > "${sb}/inventory.yaml" <<'EOF'
vars:
  service_name: "rtkk-node"
  listen_port: 19912
hosts:
  localhost:
    ansible_connection: local
EOF

  cat > "${sb}/node.conf.gtmpl" <<'EOF'
service={{ .service_name }}
port={{ .listen_port }}
encoded={{ b64enc .service_name }}
EOF

  cat > "${sb}/site.yaml" <<EOF
- name: Local baseline smoke play
  hosts: localhost
  tasks:
    - name: Write static secret token file
      copy:
        content: "token=placeholder\n"
        dest: "${sb}/secret.env"
        mode: "0600"
    - name: Render node configuration from template
      template:
        src: "${sb}/node.conf.gtmpl"
        dest: "${sb}/node.conf"
        mode: "0644"
    - name: Inspect rendered file via shell with changed_when false
      shell: "grep '^port=19912$' '${sb}/node.conf'"
      register: port_check
      changed_when: "false"
      failed_when: 'ne .rc 0'
EOF

  local log1="${sb}/run1.log"
  local log2="${sb}/run2.log"
  local out1 out2

  out1="$("$RTKK_BIN" -l "$log1" run "${sb}/site.yaml" -i "${sb}/inventory.yaml")"
  assert_file_mode "${sb}/secret.env" "600"
  assert_file_mode "${sb}/node.conf" "644"

  local rendered
  rendered="$(cat "${sb}/node.conf")"
  assert_contains "$rendered" "service=rtkk-node" "template must render service_name"
  assert_contains "$rendered" "port=19912" "template must render listen_port"
  assert_contains "$rendered" "encoded=cnRray1ub2Rl" "template must evaluate b64enc helper"
  assert_contains "$out1" "changed" "first run must report changed tasks"
  assert_contains "$out1" "Playbook executed successfully." "first run must succeed"

  out2="$("$RTKK_BIN" -l "$log2" run "${sb}/site.yaml" -i "${sb}/inventory.yaml")"
  assert_contains "$out2" "Playbook executed successfully." "second run must succeed"
  assert_not_contains "$out2" "fatal:" "second run must report zero failures"
  if printf '%s\n' "$out2" | grep -qx 'changed'; then
    echo "Expected idempotent second playbook run with zero changed tasks" >&2
    return 1
  fi
}

test_fail_fast_bad_inv() {
  local bad_dir="${GATE_TMP_DIR}/bad-inputs"
  mkdir -p "$bad_dir"

  cat > "${bad_dir}/cycle-inv.yaml" <<'EOF'
hosts:
  n1: 127.0.0.1
roles:
  group_a: [group_b]
  group_b: [group_a]
EOF

  cat > "${bad_dir}/noop.yaml" <<'EOF'
- name: Noop play
  hosts: localhost
  tasks:
    - name: Echo ok
      command: "true"
EOF

  local err_cycle=""
  if err_cycle="$("$RTKK_BIN" -l "${bad_dir}/cycle.log" run "${bad_dir}/noop.yaml" -i "${bad_dir}/cycle-inv.yaml" 2>&1)"; then
    echo "Expected cyclic inventory to fail with non-zero exit code" >&2
    return 1
  fi
  assert_contains "$err_cycle" "Error:" "cyclic inventory must emit structured Error diagnostic"

  cat > "${bad_dir}/good-inv.yaml" <<'EOF'
hosts:
  localhost:
    ansible_connection: local
EOF

  local err_forks=""
  if err_forks="$("$RTKK_BIN" -l "${bad_dir}/forks0.log" run "${bad_dir}/noop.yaml" -i "${bad_dir}/good-inv.yaml" --forks 0 2>&1)"; then
    echo "Expected --forks 0 to fail fast with non-zero exit code" >&2
    return 1
  fi
  assert_contains "$err_forks" "forks" "--forks 0 error message must mention forks limit"
}

run_assertion "cli-version" test_cli_version
run_assertion "cert-99y-and-verify" test_cert_99y_and_verify
run_assertion "cli-local-playbook" test_cli_local_playbook
run_assertion "fail-fast-bad-inv" test_fail_fast_bad_inv

gate_pass "step00"
