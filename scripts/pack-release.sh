#!/usr/bin/env bash
# Pack a tailfin binary into dist/tailfin-<target>.tar.gz with the binary at archive root.
set -euo pipefail
# macOS tar otherwise stuffs Apple xattrs that GNU tar warns about.
export COPYFILE_DISABLE=1

bin="${1:?usage: pack-release.sh <binary> <target-triple>}"
target="${2:?}"
test -f "$bin" || { echo "missing binary: $bin" >&2; exit 1; }

root="$(cd "$(dirname "$0")/.." && pwd)"
dist="$root/dist"
stage="$(mktemp -d)"
trap 'rm -rf "$stage"' EXIT

cp "$bin" "$stage/tailfin"
chmod +x "$stage/tailfin"
mkdir -p "$dist"
archive="$dist/tailfin-${target}.tar.gz"
tar -C "$stage" -czf "$archive" tailfin
(
  cd "$dist"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "tailfin-${target}.tar.gz"
  else
    shasum -a 256 "tailfin-${target}.tar.gz"
  fi
) | tee "$archive.sha256"
echo "wrote $archive"
