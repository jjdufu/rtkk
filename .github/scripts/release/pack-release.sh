#!/usr/bin/env bash
set -euo pipefail

export COPYFILE_DISABLE=1

NAME=""
VERSION=""
OS=""
ARCH=""
OUTDIR="dist/release"
BIN=""
CONFIGS=""
COMPOSE=""
COMPOSE_EXPLICIT=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --name) NAME="$2"; shift 2 ;;
    --version) VERSION="$2"; shift 2 ;;
    --os) OS="$2"; shift 2 ;;
    --arch) ARCH="$2"; shift 2 ;;
    --outdir) OUTDIR="$2"; shift 2 ;;
    --bin) BIN="$2"; shift 2 ;;
    --configs) CONFIGS="$2"; shift 2 ;;
    --compose) COMPOSE="$2"; COMPOSE_EXPLICIT=1; shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 1 ;;
  esac
done

VERSION="${VERSION#v}"

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
if [[ "$COMPOSE_EXPLICIT" -eq 1 && ! -d "$COMPOSE" ]]; then
  echo "compose dir not found: $COMPOSE" >&2
  exit 1
fi
if [[ "$COMPOSE_EXPLICIT" -eq 0 && -d "deploy/compose" ]]; then
  COMPOSE="deploy/compose"
fi

WORK_TMP="$(mktemp -d)"
trap 'rm -rf "$WORK_TMP"' EXIT

STAGE="${WORK_TMP}/bin"
mkdir -p "${STAGE}/configs"
cp "$BIN" "${STAGE}/${NAME}"
chmod 0755 "${STAGE}/${NAME}"
cp -a "${CONFIGS}/." "${STAGE}/configs/"

mkdir -p "$OUTDIR"
ARCHIVE="${OUTDIR}/${NAME}_${VERSION}_${OS}_${ARCH}.tar.gz"
tar -C "$STAGE" -czf "$ARCHIVE" .
echo "wrote ${ARCHIVE}"
ls -lh "$ARCHIVE"

if [[ -n "$COMPOSE" && -d "$COMPOSE" ]]; then
  COMPOSE_STAGE="${WORK_TMP}/compose"
  mkdir -p "$COMPOSE_STAGE"

  shopt -s dotglob nullglob
  for entry in "${COMPOSE}"/*; do
    base="$(basename "$entry")"
    case "$base" in
      .|..|data|logs|backups) continue ;;
    esac
    cp -a "$entry" "${COMPOSE_STAGE}/${base}"
  done
  shopt -u dotglob nullglob

  ENV_FILE="${COMPOSE_STAGE}/config/.env"
  if [[ ! -f "$ENV_FILE" ]]; then
    echo "compose .env not found: ${COMPOSE}/config/.env" >&2
    exit 1
  fi

  awk -v ver="$VERSION" '
    BEGIN { updated = 0 }
    /^RTKK_VERSION=/ {
      print "RTKK_VERSION=" ver
      updated = 1
      next
    }
    { print }
    END {
      if (!updated) {
        print "RTKK_VERSION=" ver
      }
    }
  ' "$ENV_FILE" > "${ENV_FILE}.tmp"
  mv "${ENV_FILE}.tmp" "$ENV_FILE"

  if [[ -d "${COMPOSE_STAGE}/scripts" ]]; then
    for sh_file in "${COMPOSE_STAGE}/scripts"/*.sh; do
      [[ -f "$sh_file" ]] && chmod 0755 "$sh_file"
    done
  fi

  COMPOSE_ARCHIVE="${OUTDIR}/${NAME}_${VERSION}_compose.tar.gz"
  tar -C "$COMPOSE_STAGE" -czf "$COMPOSE_ARCHIVE" .
  echo "wrote ${COMPOSE_ARCHIVE}"
  ls -lh "$COMPOSE_ARCHIVE"
fi
