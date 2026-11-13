#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
bin=${UI_PROBE_BIN:-$root/ui-probe/target/debug/ui-probe}
here="$root/ui-probe/tests"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export HOME="$tmp/home"
mkdir -p "$HOME"

"$bin" --url "file://$here/rules-v2.html" --no-sandbox \
  --rules "$here/rules-v2.json" --design "$here/rules-v2-design.json" \
  --pretty >"$tmp/v2.json" || [ $? -eq 2 ]
"$bin" --url "file://$here/rules-v2-lifecycle.html" --no-sandbox \
  --rules "$here/rules-v2-lifecycle.json" --pretty >"$tmp/lifecycle.json" || [ $? -eq 2 ]

jq -e '[.findings[] | select(.check == "bare-selector")] | length == 0' "$tmp/v2.json" >/dev/null
jq -e '[.findings[] | select(.check == "skeleton-card-composition")] | length == 1' "$tmp/v2.json" >/dev/null
jq -e '[.findings[] | select(.check == "fill-composed")] | length == 1' "$tmp/v2.json" >/dev/null
jq -e '[.findings[] | select(.check == "padding-components")] | length == 1' "$tmp/v2.json" >/dev/null
jq -e '[.findings[] | select(.check == "card-count" and .evidence.count == 4)] | length == 1' "$tmp/v2.json" >/dev/null
jq -e '[.findings[] | select(.check == "component-membership")] | length == 2' "$tmp/v2.json" >/dev/null
jq -e '[.findings[] | select(.check == "membership-shorthand")] | length == 2' "$tmp/v2.json" >/dev/null
jq -e '[.findings[] | select(.check == "card-exceptions" and (.selector | contains("exempt"))) ] | length == 0' "$tmp/v2.json" >/dev/null
jq -e '[.findings[] | select(.check == "kind-aliases")] | length == 4' "$tmp/v2.json" >/dev/null
jq -e '.coverageGaps | length == 1 and .[0].coverageGap == true' "$tmp/v2.json" >/dev/null
jq -e '[.findings[] | select(.check == "skeleton-at-first-paint" and .lifecycle == "first-paint")] | length == 1' "$tmp/lifecycle.json" >/dev/null
jq -e '[.findings[] | select(.check == "skeleton-at-settled")] | length == 0' "$tmp/lifecycle.json" >/dev/null
echo "rules format checks passed"
