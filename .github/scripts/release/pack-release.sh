#!/usr/bin/env bash
set -euo pipefail

NAME=""
VERSION=""
OS=""
ARCH=""
OUTDIR="dist/release"
BIN=""
CONFIGS=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --name) NAME="$2"; shift 2 ;;
    --version) VERSION="$2"; shift 2 ;;
    --os) OS="$2"; shift 2 ;;
    --arch) ARCH="$2"; shift 2 ;;
    --outdir) OUTDIR="$2"; shift 2 ;;
    --bin) BIN="$2"; shift 2 ;;
    --configs) CONFIGS="$2"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 1 ;;
  esac
done

if [[ -z "$NAME" || -z "$VERSION" || -z "$OS" || -z "$ARCH" || -z "$BIN" || -z "$CONFIGS" ]]; then
  echo "missing required args" >&2
  exit 1
fi
if [[ ! -f "$BIN" ]]; then
  echo "binary not found: $BIN" >&2
  exit 1
fi
if [[ ! -d "$CONFIGS" ]]; then
  echo "configs dir not found: $CONFIGS" >&2
  exit 1
fi

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

cp "$BIN" "${STAGE}/${NAME}"
chmod +x "${STAGE}/${NAME}"
mkdir -p "${STAGE}/configs"
cp -a "${CONFIGS}/." "${STAGE}/configs/"

mkdir -p "$OUTDIR"
ARCHIVE="${OUTDIR}/${NAME}_${VERSION}_${OS}_${ARCH}.tar.gz"
tar -C "$STAGE" -czf "$ARCHIVE" .
echo "wrote ${ARCHIVE}"
ls -lh "$ARCHIVE"
