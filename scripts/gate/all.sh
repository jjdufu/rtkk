#!/usr/bin/env bash
# Master 3-tier quality gate runner (G1 Static Guards -> G2 Compile/Lint/Doc/Unit -> G3 E2E Gates).
set -euo pipefail

if [[ -f "$HOME/.cargo/env" ]]; then
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

echo "=== [G1] Running Static Guards ==="
python3 scripts/lint-guard.py
python3 scripts/web-guard.py
python3 scripts/configs-guard.py
python3 scripts/docs-and-plans-guard.py
python3 scripts/check_routes.py
python3 scripts/check_traceability.py
bash scripts/check-write-scope.sh --self-test

echo "=== [G2] Running Rust Format, Clippy, Rustdoc, and Workspace Tests ==="
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked
cargo test --workspace --locked

if [[ -f "${REPO_ROOT}/web/package.json" ]]; then
  echo "=== [G2-Web] Running Frontend TypeScript Check and Production Build ==="
  (
    cd "${REPO_ROOT}/web"
    npm run typecheck
    npm run build
  )
fi

echo "=== [G3] Running Active End-to-End Gates ==="
cargo build --locked

# Stage A activates step00 by default; orchestrator appends step01..step07 as Stage B steps land.
ACTIVE_GATES="${RTKK_ACTIVE_GATES:-step00}"
for gate_step in $ACTIVE_GATES; do
  bash "scripts/gate/${gate_step}.sh"
done

echo "ALL GATES PASS"
