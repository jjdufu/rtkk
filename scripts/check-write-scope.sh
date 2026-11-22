#!/usr/bin/env bash
# G1 Static Guard: Enforce multi-agent role exclusive write-scope boundaries.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

is_path_allowed_for_role() {
  local role="$1"
  local rel_path="$2"

  case "$role" in
    orchestrator)
      case "$rel_path" in
        IRON_RULES.md|AGENTS.md|GEMINI.md|CLAUDE.md|.gitignore|README.md|OPERATION_GUIDE.md|\
        Cargo.toml|Cargo.lock|rust-toolchain.toml|build.rs|\
        src/lib.rs|src/main.rs|src/cli.rs|src/error.rs|\
        src/daemon/mod.rs|src/daemon/state.rs|src/daemon/routes/mod.rs|\
        web/package.json|web/package-lock.json|web/tsconfig.json|web/vite.config.ts|\
        web/src/App.tsx|web/src/lib/api/enums.ts|web/src/modules/registry.ts|\
        .github/*|docs/*|plans/*|\
        scripts/*.py|scripts/check-write-scope.sh|scripts/gate/lib.sh|scripts/gate/all.sh)
          return 0
          ;;
        *)
          return 1
          ;;
      esac
      ;;
    squad-engine)
      case "$rel_path" in
        src/cert.rs|src/prep.rs|src/utils.rs|src/workspace.rs|\
        src/connector/*|src/inventory/*|src/logging/*|src/playbook/*|\
        src/registry/*|src/tasks/*|src/template/*|src/workspace/*)
          return 0
          ;;
        *)
          return 1
          ;;
      esac
      ;;
    squad-daemon)
      case "$rel_path" in
        src/daemon/routes/mod.rs)
          return 1
          ;;
        src/daemon/auth.rs|src/daemon/jobs.rs|src/daemon/mask.rs|\
        src/daemon/sse.rs|src/daemon/embed.rs|src/daemon/routes/*)
          return 0
          ;;
        *)
          return 1
          ;;
      esac
      ;;
    squad-web)
      case "$rel_path" in
        web/src/App.tsx|web/src/lib/api/enums.ts|web/src/modules/registry.ts)
          return 1
          ;;
        web/AGENTS.md|web/index.html|\
        web/src/components/*|web/src/pages/*|web/src/styles/*|\
        web/src/hooks/*|web/src/lib/*)
          return 0
          ;;
        *)
          return 1
          ;;
      esac
      ;;
    squad-ops-qa)
      case "$rel_path" in
        configs/*|deploy/*|test/*|web/e2e/*|web/playwright.config.ts|\
        scripts/gate/step*.sh|scripts/gate/vps-cluster-e2e.sh)
          return 0
          ;;
        *)
          return 1
          ;;
      esac
      ;;
    *)
      echo "FAIL check-write-scope: unknown role '$role'" >&2
      return 2
      ;;
  esac
}

run_self_test() {
  local cases_ok=(
    "orchestrator:Cargo.toml"
    "orchestrator:src/daemon/routes/mod.rs"
    "orchestrator:docs/API_CONTRACT.md"
    "orchestrator:web/src/App.tsx"
    "squad-engine:src/workspace.rs"
    "squad-engine:src/logging/events.rs"
    "squad-engine:src/template/mod.rs"
    "squad-daemon:src/daemon/auth.rs"
    "squad-daemon:src/daemon/routes/inventory.rs"
    "squad-web:web/AGENTS.md"
    "squad-web:web/src/pages/OverviewPage.tsx"
    "squad-ops-qa:configs/deploy.yaml"
    "squad-ops-qa:deploy/compose/config/compose.yml"
    "squad-ops-qa:scripts/gate/step04.sh"
  )

  local cases_deny=(
    "squad-engine:Cargo.toml"
    "squad-engine:src/cli.rs"
    "squad-daemon:src/daemon/routes/mod.rs"
    "squad-daemon:src/daemon/mod.rs"
    "squad-web:web/src/App.tsx"
    "squad-web:web/src/lib/api/enums.ts"
    "squad-web:web/package.json"
    "squad-ops-qa:scripts/gate/all.sh"
    "squad-ops-qa:scripts/lint-guard.py"
    "orchestrator:src/playbook/engine.rs"
  )

  local entry role path
  for entry in "${cases_ok[@]}"; do
    role="${entry%%:*}"
    path="${entry#*:}"
    if ! is_path_allowed_for_role "$role" "$path"; then
      echo "FAIL check-write-scope self-test: expected ALLOW for role=${role} path=${path}" >&2
      exit 1
    fi
  done

  for entry in "${cases_deny[@]}"; do
    role="${entry%%:*}"
    path="${entry#*:}"
    if is_path_allowed_for_role "$role" "$path"; then
      echo "FAIL check-write-scope self-test: expected DENY for role=${role} path=${path}" >&2
      exit 1
    fi
  done

  echo "ok  check-write-scope (--self-test passed: ${#cases_ok[@]} allow + ${#cases_deny[@]} deny rules verified)"
}

ROLE=""
BASE_REF="HEAD"
SELF_TEST=0
EXPLICIT_FILES=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --self-test)
      SELF_TEST=1
      shift
      ;;
    --role)
      ROLE="${2:-}"
      shift 2
      ;;
    --base)
      BASE_REF="${2:-HEAD}"
      shift 2
      ;;
    --files)
      shift
      while [[ $# -gt 0 ]]; do
        EXPLICIT_FILES+=("$1")
        shift
      done
      ;;
    *)
      echo "Usage: $0 [--self-test] | [--role <role> [--base <git-ref>] [--files <path>...]]" >&2
      exit 2
      ;;
  esac
done

if [[ "$SELF_TEST" -eq 1 ]]; then
  run_self_test
  exit 0
fi

if [[ -z "$ROLE" ]]; then
  echo "Usage: $0 --self-test | --role <orchestrator|squad-engine|squad-daemon|squad-web|squad-ops-qa>" >&2
  exit 2
fi

CHANGED_FILES=()
if [[ "${#EXPLICIT_FILES[@]}" -gt 0 ]]; then
  CHANGED_FILES=("${EXPLICIT_FILES[@]}")
else
  while IFS= read -r line; do
    [[ -n "$line" ]] && CHANGED_FILES+=("$line")
  done < <(
    cd "$REPO_ROOT"
    {
      git diff --name-only "$BASE_REF" 2>/dev/null || true
      git diff --name-only --cached 2>/dev/null || true
      git ls-files --others --exclude-standard 2>/dev/null || true
    } | sort -u
  )
fi

VIOLATIONS=0
for f in "${CHANGED_FILES[@]+"${CHANGED_FILES[@]}"}"; do
  if ! is_path_allowed_for_role "$ROLE" "$f"; then
    echo "FAIL check-write-scope: role '$ROLE' is not allowed to modify '$f'" >&2
    VIOLATIONS=$((VIOLATIONS + 1))
  fi
done

if [[ "$VIOLATIONS" -gt 0 ]]; then
  exit 1
fi

echo "ok  check-write-scope (role=${ROLE}, ${#CHANGED_FILES[@]} changed files within scope)"
