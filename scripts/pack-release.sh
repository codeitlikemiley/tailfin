#!/usr/bin/env bash
# Pack a raz binary into dist/raz-<target>.tar.gz with the binary at archive root.
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

cp "$bin" "$stage/raz"
chmod +x "$stage/raz"
mkdir -p "$dist"
tar -C "$stage" -czf "$dist/raz-${target}.tar.gz" raz
echo "wrote $dist/raz-${target}.tar.gz"
