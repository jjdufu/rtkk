#!/usr/bin/env bash
set -euo pipefail

REPO="${RTKK_REPO:-jjdufu/rtkk}"
BINDIR="${RTKK_BINDIR:-$HOME/.local/bin}"
DEST="${BINDIR}/rtkk"
RTKK_TMP=""

cleanup() {
  if [[ -n "${RTKK_TMP:-}" ]]; then
    rm -rf "$RTKK_TMP"
    RTKK_TMP=""
  fi
}
trap cleanup EXIT

usage() {
  cat <<EOF
usage: $(basename "$0") [version] [--uninstall]

  (default)  download the latest rtkk release for this OS/arch into:
             \$HOME/.local/bin/rtkk
  version    install that release (with or without a leading v)
             example: $(basename "$0") 0.0.1
             piped:   curl .../install.sh | bash -s -- 0.0.1
  --uninstall
             delete the installed rtkk binary
             does not delete configs, logs, downloads, or pki

Environment:
  RTKK_REPO     GitHub repo (default: jjdufu/rtkk)
  RTKK_VERSION  release version without v (default: latest);
                overridden by the version argument
  RTKK_BINDIR   install directory (default: \$HOME/.local/bin)

Only Linux x86_64 is published. The binary is built in Debian 13.
EOF
}

die() { echo "$*" >&2; exit 1; }

need_home() {
  [[ -n "${HOME:-}" ]] || die "HOME is unset"
}

os_arch() {
  local sys mach
  sys="$(uname -s 2>/dev/null || true)"
  mach="$(uname -m 2>/dev/null || true)"
  case "$sys" in
    Linux) OS="linux" ;;
    *) die "unsupported OS: ${sys:-unknown} (rtkk releases only support Linux x86_64, tested on Debian 13)" ;;
  esac
  case "$mach" in
    x86_64|amd64) ARCH="amd64" ;;
    *) die "unsupported arch: ${mach:-unknown} (rtkk releases only support Linux x86_64)" ;;
  esac
}

verify_sha256() {
  local sums="$1" file="$2"
  if command -v sha256sum >/dev/null 2>&1; then
    grep -E "([ *])${file}\$" "$sums" | sha256sum -c -
  elif command -v shasum >/dev/null 2>&1; then
    grep -E "([ *])${file}\$" "$sums" | shasum -a 256 -c -
  else
    die "need sha256sum or shasum to verify the download"
  fi
}

resolve_version() {
  local raw="${CLI_VERSION:-${RTKK_VERSION:-}}"
  if [[ -n "$raw" ]]; then
    VERSION="${raw#v}"
    [[ -n "$VERSION" && "$VERSION" != "latest" ]] || die "invalid version: $raw"
    return 0
  fi
  local url
  url="$(curl -fsSL --retry 3 -o /dev/null -w '%{url_effective}' "https://github.com/${REPO}/releases/latest")"
  VERSION="${url##*/}"
  VERSION="${VERSION#v}"
  [[ -n "$VERSION" && "$VERSION" != "latest" ]] || die "cannot resolve latest release from $url"
}

do_uninstall() {
  need_home
  if [[ -e "$DEST" || -L "$DEST" ]]; then
    rm -f "$DEST"
    echo "removed $DEST"
  else
    echo "not installed: $DEST"
  fi
}

do_install() {
  need_home
  os_arch
  command -v curl >/dev/null 2>&1 || die "curl not found"
  command -v tar >/dev/null 2>&1 || die "tar not found"

  local VERSION asset base
  resolve_version
  asset="rtkk_${VERSION}_${OS}_${ARCH}.tar.gz"
  base="https://github.com/${REPO}/releases/download/v${VERSION}"
  cleanup
  RTKK_TMP="$(mktemp -d)"

  echo "installing rtkk ${VERSION} (${OS}/${ARCH})"
  echo "downloading ${base}/${asset}"
  if [[ -t 2 ]]; then
    curl -fL --retry 3 --progress-bar -o "${RTKK_TMP}/${asset}" "${base}/${asset}"
    echo >&2
  else
    curl -fsSL --retry 3 -o "${RTKK_TMP}/${asset}" "${base}/${asset}"
  fi
  curl -fsSL --retry 3 -o "${RTKK_TMP}/SHA256SUMS" "${base}/SHA256SUMS"
  (
    cd "$RTKK_TMP"
    verify_sha256 SHA256SUMS "$asset"
  )
  tar -xzf "${RTKK_TMP}/${asset}" -C "$RTKK_TMP"
  [[ -f "${RTKK_TMP}/rtkk" ]] || die "archive missing rtkk binary"

  mkdir -p "$BINDIR"
  if command -v install >/dev/null 2>&1; then
    install -m 755 "${RTKK_TMP}/rtkk" "$DEST"
  else
    cp "${RTKK_TMP}/rtkk" "$DEST"
    chmod 755 "$DEST"
  fi

  echo "installed $DEST"
  echo "example configs are in the release tarball (configs/); this installer only copies the binary"
  case ":$PATH:" in
    *":$BINDIR:"*) ;;
    *) echo "note: $BINDIR is not on PATH; add it or run $DEST" ;;
  esac
  echo "run: rtkk --version"
}

UNINSTALL=0
CLI_VERSION=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --uninstall|-u) UNINSTALL=1; shift ;;
    -h|--help) usage; exit 0 ;;
    --version)
      [[ $# -ge 2 ]] || die "--version needs an argument"
      [[ -z "$CLI_VERSION" ]] || die "version specified twice"
      CLI_VERSION="$2"
      shift 2
      ;;
    --version=*)
      [[ -z "$CLI_VERSION" ]] || die "version specified twice"
      CLI_VERSION="${1#--version=}"
      shift
      ;;
    -*)
      usage >&2
      die "unknown arg: $1"
      ;;
    *)
      [[ -z "$CLI_VERSION" ]] || die "version specified twice"
      CLI_VERSION="$1"
      shift
      ;;
  esac
done

if [[ "$UNINSTALL" == 1 ]]; then
  do_uninstall
else
  do_install
fi
