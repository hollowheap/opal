#!/usr/bin/env bash
set -u

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
bin=${UI_PROBE_BIN:-$root/pkgs/ui-probe/target/debug/ui-probe}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

if [ ! -x "$bin" ]; then
  echo "missing ui-probe binary: $bin" >&2
  echo "build it with: nix develop .#rust --command bash -lc 'cd pkgs/ui-probe && cargo build'" >&2
  exit 1
fi

pass=0
fail=0

ok() {
  pass=$((pass + 1))
}

bad() {
  fail=$((fail + 1))
  echo "FAIL: $*" >&2
}

run() {
  name=$1
  expected=$2
  out=$3
  shift 3
  "$bin" "$@" >"$out" 2>"$tmp/$name.err"
  code=$?
  if [ "$code" -eq "$expected" ]; then
    ok
  else
    bad "$name exited $code, expected $expected"
    sed -n '1,5p' "$tmp/$name.err" >&2
  fi
}

jqr() {
  name=$1
  file=$2
  filter=$3
  if jq -e "$filter" "$file" >/dev/null 2>&1; then
    ok
  else
    bad "$name assertion failed: $filter"
  fi
}

err_has() {
  name=$1
  text=$2
  if grep -q -- "$text" "$tmp/$name.err"; then
    ok
  else
    bad "$name stderr lacks: $text"
  fi
}

file_url() {
  printf 'file://%s/%s' "$here" "$1"
}

clean_url=$(file_url clean.html)
generic_url=$(file_url generic.html)
design_url=$(file_url design-drift.html)
dtcg="$here/design-dtcg.json"
pack="$root/assets/skills/review/references/impeccable-rules.json"

run clean 0 "$tmp/clean.json" --url "$clean_url" --no-sandbox --pretty
jqr clean-empty "$tmp/clean.json" '.findings == []'
jqr clean-colour-verified "$tmp/clean.json" '.unverified == []'

run generic 2 "$tmp/generic.json" --url "$generic_url" --no-sandbox --pretty
for check in page-overflow missing-lang heading-skip missing-name target-size positive-tabindex contrast clipped-text; do
  jqr "generic-$check" "$tmp/generic.json" "[.findings[].check] | index(\"$check\") != null"
done
jqr generic-visible "$tmp/generic.json" '.summary.captured >= .summary.elements'

run disclosure 2 "$tmp/disclosure.json" --url "$(file_url disclosure.html)" --no-sandbox --expand --pretty
jqr disclosure-expanded "$tmp/disclosure.json" '.states.expanded | length == 1'
jqr disclosure-skip "$tmp/disclosure.json" '[.states.skipped[] | select(.reason == "postcondition-not-met")] | length == 1'

run nested 2 "$tmp/nested.json" --url "$(file_url structure.html)" --no-sandbox --rules "$here/rules-error.json" --pretty
jqr nested-error "$tmp/nested.json" '[.findings[] | select(.check == "nested-cards" and .severity == "error" and .state == "default" and .layer == "layout")] | length == 1'

run cards 2 "$tmp/cards.json" --url "$(file_url cards.html)" --no-sandbox --rules "$here/rules-error.json" --pretty
jqr cards-surface "$tmp/cards.json" '[.findings[] | select(.check == "nested-cards")] | length == 2'

run state 2 "$tmp/state.json" --url "$(file_url state.html)" --no-sandbox --expand --rules "$here/rules-state.json" --pretty
jqr state-open "$tmp/state.json" '[.findings[] | select(.check == "line-length")] | length > 0 and all(.state == "open" and (.states == ["open"]))'
jqr state-postcondition "$tmp/state.json" '.states.expanded[0].postcondition == "aria-expanded=true or controlled region visible"'
jqr state-hidden "$tmp/state.json" '.summary.captured > .summary.elements'

run dtcg 2 "$tmp/dtcg.json" --url "$design_url" --no-sandbox --design "$dtcg" --pretty
jqr dtcg-color "$tmp/dtcg.json" '[.findings[] | select(.check == "design-color") | .evidence.value] | index("rgba(242, 242, 242, 1)") != null'
jqr dtcg-font "$tmp/dtcg.json" '[.findings[] | select(.check == "design-font") | .evidence.family] | index("comic sans ms") != null'
jqr dtcg-radius-px "$tmp/dtcg.json" '[.findings[] | select(.check == "design-radius") | .evidence.value] | index("50px") != null'
jqr dtcg-radius-percent "$tmp/dtcg.json" '[.findings[] | select(.check == "design-radius") | .evidence.value] | index("50%") == null'
jqr dtcg-spacing "$tmp/dtcg.json" '[.findings[] | select(.check == "design-spacing") | .evidence.value] | index(13) != null'
jqr dtcg-typography "$tmp/dtcg.json" '[.findings[].check] | index("design-typography") != null'
jqr dtcg-gradient-skipped "$tmp/dtcg.json" '[.findings[] | select(.check == "design-color") | .evidence.value] | index("rgba(18, 52, 86, 1)") == null'

run generic-rules 0 "$tmp/generic-rules.json" --url "$(file_url gradient-text.html)" --no-sandbox --rules "$pack" --pretty
jqr generic-rules-oracle "$tmp/generic-rules.json" '[.findings[] | select(.check == "no-gradient-text" and .severity == "advisory" and .property == "background-clip" and .evidence.actual == "text")] | length == 1'

run allowed 0 "$tmp/allowed.json" --url "$(file_url gradient-text.html)" --no-sandbox --rules "$pack" --allow no-gradient-text --pretty
jqr allowed-filter "$tmp/allowed.json" '[.findings[].check] | index("no-gradient-text") == null'

cd "$tmp"
run auto 0 "$tmp/auto.json" --url "$design_url" --no-sandbox --design auto --pretty
jqr auto-report "$tmp/auto.json" '.designFile == ".ui-witness/design.json" and .inferredDesign.palette[0].count != null and .inferredDesign.palette[0].confidence != null'
jqr auto-file "$tmp/.ui-witness/design.json" '.color["color-1"] and .font["font-1"] and .typography["type-1"]'
jqr auto-file-descriptions "$tmp/.ui-witness/design.json" '[.. | objects | select(has("$description")) | .["$description"]] | length > 0 and all(.[]; test("^Inferred .+, [0-9]+ observations$"))'
jqr auto-file-statistics "$tmp/.ui-witness/design.json" '[paths | select(.[-1] == "count" or .[-1] == "confidence")] | length == 0'

run auto-existing 1 "$tmp/auto-existing.json" --url "$design_url" --no-sandbox --design auto
err_has auto-existing "design file already exists"

run auto-force 0 "$tmp/auto-force.json" --url "$design_url" --no-sandbox --design auto --force

jq '.color = {"color-1": {"$value": "#000000", "$type": "color"}}' "$tmp/.ui-witness/design.json" >"$tmp/design-merge.json"
mv "$tmp/design-merge.json" "$tmp/.ui-witness/design.json"
run infer-missing 0 "$tmp/infer-missing.json" --url "$design_url" --no-sandbox --design auto --infer-missing
jqr infer-missing-preserve "$tmp/.ui-witness/design.json" '.color["color-1"]["$value"] == "#000000"'
jqr infer-missing-fill "$tmp/.ui-witness/design.json" '.font["font-1"] != null and .radius["radius-1"] != null'
cd "$root"

jq 'del(.radius)' "$dtcg" >"$tmp/partial.json"
run partial 2 "$tmp/partial.json.report" --url "$design_url" --no-sandbox --design "$tmp/partial.json" --infer-missing --pretty
jqr partial-preserve "$tmp/partial.json" '.color.ink["$value"] == "#1a1a1a"'
jqr partial-fill "$tmp/partial.json" '.radius["radius-1"] != null'

run params 2 "$tmp/params.json" --url "$design_url" --no-sandbox --design "$dtcg" --rules "$here/rules-params.json" --pretty
jqr params-missing "$tmp/params.json" '[.unverified[] | select(.check == "line-length" and (.reason | startswith("missing param:")) and .rule == "line-length")] | length == 1'

run dark 2 "$tmp/dark.json" --url "$(file_url dark-only.html)" --no-sandbox --theme both --pretty
jqr dark-themes "$tmp/dark.json" '.themes == ["light", "dark"]'
jqr dark-finding-theme "$tmp/dark.json" '[.findings[] | select(.check == "contrast") | .themes] | any(. == ["light"])'

run source 2 "$tmp/source.json" --source "$here/source" --source-all --pretty
jqr source-token-filter "$tmp/source.json" '[.findings[] | select(.check == "raw-color") | .file] | unique == ["component.tsx"]'

run missing-source 1 "$tmp/missing-source.json" --source "$tmp/does-not-exist"
err_has missing-source "source directory does not exist"

run empty 1 "$tmp/empty.json" --url "$(file_url empty.html)" --no-sandbox
err_has empty "empty capture"

run launch-fail 1 "$tmp/launch-fail.json" --url "$clean_url" --chrome "$here/fake-chrome.sh" --no-sandbox
err_has launch-fail "fake chrome failed: boom"
err_has launch-fail "status: exit status: 3"

run launch-timeout 1 "$tmp/launch-timeout.json" --url "$clean_url" --chrome "$here/sleep-chrome.sh" --no-sandbox --launch-timeout 100
err_has launch-timeout "did not report a DevTools endpoint within 100ms"

for name in invalid-key invalid-rule-id invalid-property invalid-selector invalid-param; do
  run "$name" 1 "$tmp/$name.json" --url "$clean_url" --no-sandbox --rules "$here/$name.json"
  err_has "$name" "rules:"
done

echo "ui-probe tests: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
