#!/usr/bin/env bash
# Stage B · Step 01 Gate: Scoped EventSink concurrency isolation & Workspace bootstrap (C15, C16).
set -euo pipefail

# shellcheck source=scripts/gate/lib.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib.sh"

gate_init "step01"

test_concurrent_sink_isolated() {
  (
    cd "$REPO_ROOT"
    cargo test --test workspace_and_events_tests concurrent_sink_isolated --locked -- --exact
  )
}

test_workspace_bootstrap_idempotent() {
  (
    cd "$REPO_ROOT"
    cargo test --test workspace_and_events_tests workspace_bootstrap_idempotent --locked -- --exact
  )

  # Also verify CLI execution against a freshly initialized isolated workspace directory
  ensure_rtkk_bin
  local ws="${GATE_WORK_DIR}/ws-init-check"
  mkdir -p "$ws"

  (
    cd "$REPO_ROOT"
    RTKK_WORK_DIR="$ws" cargo test --test workspace_and_events_tests --locked
  )

  # Confirm existing Phase 0 53-test suite still passes without global static regressions
  (
    cd "$REPO_ROOT"
    cargo test --workspace --locked
  )
}

run_assertion "concurrent-sink-isolated" test_concurrent_sink_isolated
run_assertion "workspace-bootstrap-idempotent" test_workspace_bootstrap_idempotent

gate_pass "step01"
