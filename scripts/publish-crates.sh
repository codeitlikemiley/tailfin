#!/usr/bin/env bash
# Publish workspace crates to crates.io in dependency order.
# A crate whose current version is already on the registry is skipped, so
# re-running the job after a rate-limit or a retry is safe.
#
# Requires CARGO_REGISTRY_TOKEN. Optional: DRY_RUN=1 to package without upload.
set -euo pipefail

CRATES=(
  tailfin-ident
  tailfin-wire
  tailfin-tree
  tailfin-ledger
  tailfin-proxy
  tailfin
)

die() { echo "publish-crates: $*" >&2; exit 1; }

[ -n "${CARGO_REGISTRY_TOKEN:-}" ] || die "CARGO_REGISTRY_TOKEN is unset"

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

version_of() {
  python3 - "$1" <<'PY'
import json, subprocess, sys
name = sys.argv[1]
meta = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--format-version", "1", "--no-deps", "--offline"]
))
for p in meta["packages"]:
    if p["name"] == name:
        print(p["version"])
        raise SystemExit(0)
raise SystemExit(f"unknown crate {name}")
PY
}

already_published() {
  local name="$1" ver="$2" code
  code="$(curl -sS -o /dev/null -w '%{http_code}' \
    -A 'tailfin-publish/0.1 (https://github.com/codeitlikemiley/tailfin)' \
    "https://crates.io/api/v1/crates/${name}/${ver}")"
  [ "$code" = 200 ]
}

for crate in "${CRATES[@]}"; do
  ver="$(version_of "$crate")"
  if already_published "$crate" "$ver"; then
    echo "skip $crate $ver (already on crates.io)"
    continue
  fi
  echo "publish $crate $ver"
  if [ "${DRY_RUN:-}" = 1 ]; then
    cargo publish -p "$crate" --locked --dry-run
  else
    cargo publish -p "$crate" --locked
  fi
done
echo "publish-crates: done"
