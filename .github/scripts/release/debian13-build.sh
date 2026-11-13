#!/usr/bin/env bash
# Build or check rtkk inside the latest Debian 13 x86_64 image.
# The workflow pulls debian:13 immediately before invoking this script.
set -euo pipefail

MODE="${1:-release}"
RUST_VERSION="${RUST_VERSION:-1.100.0}"

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
  pkg-config \
  libssl-dev \
  zlib1g-dev \
  build-essential

curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs \
  | sh -s -- -y --default-toolchain "$RUST_VERSION" --profile minimal --component clippy --component rustfmt
# shellcheck disable=SC1091
. "$HOME/.cargo/env"
rustc --version

case "$MODE" in
  ci)
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo test --workspace --locked
    ;;
  release)
    cargo build --release --locked
    ./target/release/rtkk --version
    ./target/release/rtkk --help
    ;;
  *)
    echo "unknown mode: $MODE" >&2
    exit 1
    ;;
esac
