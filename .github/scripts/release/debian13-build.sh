#!/usr/bin/env bash
# Build or check rtkk inside the latest Debian 13 x86_64 image.
# The workflow pulls debian:13 immediately before invoking this script.
set -euo pipefail

MODE="${1:-release}"
RUST_VERSION="${RUST_VERSION:-1.100.0}"
NODE_DIST_URL="${NODE_DIST_URL:-https://nodejs.org/dist/latest-v24.x}"

. /etc/os-release
if [ "${ID:-}" != "debian" ] || [ "${VERSION_ID:-}" != "13" ]; then
  echo "rtkk is only tested on Debian 13, got ${PRETTY_NAME:-${ID:-unknown} ${VERSION_ID:-unknown}}" >&2
  exit 1
fi
if [ "$(uname -m)" != "x86_64" ]; then
  echo "rtkk releases only support Linux x86_64, got $(uname -m)" >&2
  exit 1
fi

echo "packaging image: ${PRETTY_NAME}"
# Do not pipe ldd into head: pipefail turns the resulting SIGPIPE into exit 141.
ldd --version

export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y --no-install-recommends \
  ca-certificates \
  curl \
  jq \
  python3 \
  pkg-config \
  libssl-dev \
  zlib1g-dev \
  xz-utils \
  build-essential

install_node24() {
  local node_tmp node_tarball node_ver
  node_tmp="$(mktemp -d)"
  curl --proto '=https' --tlsv1.2 -fsSL "${NODE_DIST_URL}/SHASUMS256.txt" -o "${node_tmp}/SHASUMS256.txt"
  node_tarball="$(awk '/node-v24\.[0-9]+\.[0-9]+-linux-x64\.tar\.xz$/ { print $2; exit }' "${node_tmp}/SHASUMS256.txt")"
  node_tarball="${node_tarball#\*}"
  if [ -z "$node_tarball" ]; then
    echo "failed to resolve Node.js v24 linux-x64 archive from ${NODE_DIST_URL}/SHASUMS256.txt" >&2
    rm -rf "$node_tmp"
    exit 1
  fi
  curl --proto '=https' --tlsv1.2 -fsSL "${NODE_DIST_URL}/${node_tarball}" -o "${node_tmp}/${node_tarball}"
  (
    cd "$node_tmp"
    grep -E "[ *]${node_tarball}\$" SHASUMS256.txt | sha256sum -c -
  )
  tar -xJf "${node_tmp}/${node_tarball}" -C /usr/local --strip-components=1
  rm -rf "$node_tmp"
  export PATH="/usr/local/bin:$PATH"

  node_ver="$(node --version)"
  case "$node_ver" in
    v24.*) ;;
    *)
      echo "expected Node.js v24.*, got ${node_ver}" >&2
      exit 1
      ;;
  esac
  echo "Node.js: ${node_ver}, npm: $(npm --version)"
}

build_web_dist() {
  if [ -f web/package.json ]; then
    echo "building frontend assets (web/dist/)..."
    (
      cd web
      npm ci
      npm run typecheck
      npm run build
    )
    if [ ! -f web/dist/index.html ]; then
      echo "web/dist/index.html missing after frontend build" >&2
      exit 1
    fi
  fi
}

run_containerless_step07_checks() {
  if [ -d deploy/compose/scripts ]; then
    local script_file
    for script_file in deploy/compose/scripts/*.sh; do
      [ -f "$script_file" ] && bash -n "$script_file"
    done
  fi

  if [ ! -f web/playwright.config.ts ] || [ ! -f web/e2e/parkui-and-journey.spec.ts ]; then
    return 0
  fi

  echo "=== [G3-Containerless] Running Offline Park UI Playwright E2E ==="
  local e2e_tmp e2e_work token_file daemon_pid elapsed ready admin_token
  e2e_tmp="$(mktemp -d)"
  e2e_work="${e2e_tmp}/work"
  mkdir -p "$e2e_work"
  token_file="${e2e_work}/.rtkk-daemon/admin.token"
  daemon_pid=""

  cleanup_e2e() {
    if [ -n "${daemon_pid:-}" ] && kill -0 "$daemon_pid" 2>/dev/null; then
      kill -TERM "$daemon_pid" 2>/dev/null || true
      wait "$daemon_pid" 2>/dev/null || true
    fi
    rm -rf "$e2e_tmp"
  }

  ./target/debug/rtkk -l "${e2e_tmp}/daemon-e2e.log" serve \
    --listen "127.0.0.1:19912" \
    --work-dir "$e2e_work" \
    --token-file "$token_file" &
  daemon_pid=$!
  trap cleanup_e2e EXIT INT TERM

  ready=0
  elapsed=0
  while [ "$elapsed" -lt 200 ]; do
    if curl -fsS "http://127.0.0.1:19912/api/v1/health" >/dev/null 2>&1; then
      ready=1
      break
    fi
    sleep 0.1
    elapsed=$((elapsed + 1))
  done
  if [ "$ready" -ne 1 ]; then
    echo "daemon did not become healthy on 127.0.0.1:19912 within 20s" >&2
    exit 1
  fi

  admin_token="$(tr -d '\r\n' < "$token_file")"
  (
    cd web
    RTKK_E2E_BASE_URL="http://127.0.0.1:19912" \
    RTKK_E2E_TOKEN="$admin_token" \
    npx playwright test e2e/parkui-and-journey.spec.ts
  )

  cleanup_e2e
  trap - EXIT INT TERM
  echo "PASS [step07-containerless] playwright-parkui-offline-e2e"
}

install_node24

curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs \
  | sh -s -- -y --default-toolchain "$RUST_VERSION" --profile minimal --component clippy --component rustfmt
# shellcheck disable=SC1091
. "$HOME/.cargo/env"
rustc --version

case "$MODE" in
  ci)
    build_web_dist
    if [ -f web/package.json ] && [ -f web/playwright.config.ts ]; then
      (
        cd web
        npx playwright install --with-deps chromium
      )
    fi
    if [ -n "${RTKK_ACTIVE_GATES:-}" ] || (command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1); then
      bash scripts/gate/all.sh
    else
      echo "note: Docker daemon unavailable inside Debian 13 container; running step00..step06 + offline Playwright E2E"
      RTKK_ACTIVE_GATES="step00 step01 step02 step03 step04 step05 step06" bash scripts/gate/all.sh
      run_containerless_step07_checks
    fi
    ;;
  release)
    build_web_dist
    cargo build --release --locked
    ./target/release/rtkk --version
    ./target/release/rtkk --help
    ;;
  *)
    echo "unknown mode: $MODE" >&2
    exit 1
    ;;
esac
