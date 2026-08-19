#!/usr/bin/env bash
# Install a prebuilt tailfin binary. Never compiles.
#   curl -fsSL https://raw.githubusercontent.com/${TAILFIN_REPO}/main/install.sh | bash
# Local gate / override:
#   TAILFIN_TARBALL=./dist/tailfin-<triple>.tar.gz PREFIX=$HOME/.local ./install.sh
set -euo pipefail

REPO="${TAILFIN_REPO:-codeitlikemiley/tailfin}"
PREFIX="${PREFIX:-$HOME/.local}"
BIN_DIR="${PREFIX}/bin"
VERSION="${TAILFIN_VERSION:-latest}"

die() { echo "tailfin-install: $*" >&2; exit 1; }

triple() {
  os="$(uname -s)"
  arch="$(uname -m)"
  case "${os}-${arch}" in
    Darwin-arm64)   echo aarch64-apple-darwin ;;
    Darwin-x86_64)  echo x86_64-apple-darwin ;;
    Linux-x86_64)   echo x86_64-unknown-linux-gnu ;;
    Linux-amd64)    echo x86_64-unknown-linux-gnu ;;
    Linux-aarch64)  echo aarch64-unknown-linux-gnu ;;
    *) die "no prebuilt binary for ${os} ${arch} (need macOS arm64/x64 or Linux x64)" ;;
  esac
}

TARGET="$(triple)"
ASSET="tailfin-${TARGET}.tar.gz"

if [ -n "${TAILFIN_TARBALL:-}" ]; then
  tarball="$TAILFIN_TARBALL"
  [ -f "$tarball" ] || die "TAILFIN_TARBALL not a file: $tarball"
else
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  if [ "$VERSION" = latest ]; then
    url="https://github.com/${REPO}/releases/latest/download/${ASSET}"
  else
    url="https://github.com/${REPO}/releases/download/${VERSION}/${ASSET}"
  fi
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$tmp/$ASSET" || die "download failed: $url"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$tmp/$ASSET" "$url" || die "download failed: $url"
  else
    die "need curl or wget"
  fi
  tarball="$tmp/$ASSET"
fi

extract="$(mktemp -d)"
trap 'rm -rf "$extract" ${tmp:+"$tmp"}' EXIT
tar -xzf "$tarball" -C "$extract"
[ -f "$extract/tailfin" ] || die "archive missing tailfin binary"

mkdir -p "$BIN_DIR"
install -m 0755 "$extract/tailfin" "$BIN_DIR/tailfin"
# Ad-hoc sign so macOS Gatekeeper does not stall unsigned downloads.
if [ "$(uname -s)" = Darwin ] && command -v codesign >/dev/null 2>&1; then
  codesign --force --sign - "$BIN_DIR/tailfin" >/dev/null 2>&1 || true
fi

echo "installed $BIN_DIR/tailfin"
"$BIN_DIR/tailfin" report --help >/dev/null
echo "tailfin ok"
echo "put $BIN_DIR on PATH if it is not already:"
echo "  export PATH=\"$BIN_DIR:\$PATH\""
echo
echo "  tailfin run --upstream https://api.anthropic.com"
echo "  ANTHROPIC_BASE_URL=http://localhost:7171 claude"
