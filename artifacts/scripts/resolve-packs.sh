#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
bin=${UI_PROBE_BIN:-$HOME/.opal/bin/ui-probe}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

packs=(
  "$root/skills/opal/references/default-rules.json"
  "$root/ui-probe/tests/rules-base.json"
  "$root/ui-probe/tests/rules-error.json"
  "$root/ui-probe/tests/rules-extends.json"
  "$root/ui-probe/tests/rules-params.json"
  "$root/ui-probe/tests/rules-state.json"
)

for pack in "${packs[@]}"; do
  "$bin" pack --rules "$pack" --json >"$tmp/pack.json"
  sources=$(jq '.sources | length' "$tmp/pack.json")
  if [ "$sources" -lt 1 ]; then
    echo "pack did not resolve: $pack" >&2
    exit 1
  fi
  echo "loaded: $pack ($sources source(s))"
done
