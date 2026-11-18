#!/usr/bin/env bash
set -u

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
bin=${UI_PROBE_BIN:-$root/opal/target/debug/opal}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export HOME="$tmp/home"
mkdir -p "$HOME"

if [ ! -x "$bin" ]; then
  echo "missing opal binary: $bin" >&2
  echo "build it with: cargo build --manifest-path opal/Cargo.toml" >&2
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
  inject=0
  case "${1:-}" in
    --url|--source|source) inject=1 ;;
  esac
  for arg in "$@"; do
    if [ "$arg" = "--rules" ]; then inject=0; fi
  done
  if [ "$inject" -eq 1 ]; then set -- "$@" --rules "$pack"; fi
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

run_in() {
  name=$1
  dir=$2
  expected=$3
  out=$4
  shift 4
  inject=0
  case "${1:-}" in
    audit|source) inject=1 ;;
  esac
  if [ -f "$dir/.opal/rules.json" ]; then inject=0; fi
  for arg in "$@"; do
    if [ "$arg" = "--rules" ]; then inject=0; fi
  done
  if [ "$inject" -eq 1 ]; then set -- "$@" --rules "$pack"; fi
  (cd "$dir" && "$bin" "$@") >"$out" 2>"$tmp/$name.err"
  code=$?
  if [ "$code" -eq "$expected" ]; then
    ok
  else
    bad "$name exited $code, expected $expected"
    sed -n '1,5p' "$tmp/$name.err" >&2
  fi
}

exists() {
  name=$1
  path=$2
  if [ -f "$path" ]; then
    ok
  else
    bad "$name missing file: $path"
  fi
}

file_has() {
  name=$1
  path=$2
  text=$3
  if grep -q -- "$text" "$path"; then
    ok
  else
    bad "$name output lacks: $text"
  fi
}

file_url() {
  printf 'file://%s/%s' "$here" "$1"
}

clean_url=$(file_url clean.html)
generic_url=$(file_url generic.html)
design_url=$(file_url design-drift.html)
dtcg="$here/design-dtcg.json"
pack="$root/skills/opal/references/default-rules.json"
derived() {
  jq "$2" "$pack" >"$tmp/$1.json"
}
derived contrast-off '.rules |= map(if .id == "low-contrast" then .enabled = false else . end)'
derived contrast-error '.rules |= map(if .id == "low-contrast" then .severity = "error" | .layer = "source" else . end)'
derived nested-error '.rules |= map(if .id == "nested-cards" then .severity = "error" else . end)'
derived state-pack '.rules |= map(if .id == "line-length" then .severity = "error" | .params.maxCh = 20 else . end)'
derived params-missing '.rules |= map(if .id == "aphoristic-cadence" then .params.minSections = "param:type.missing.minSections" else . end)'
derived rules-base '.structural += [{"name":"base-border-width","selector":".surface","properties":{"border-width":{"deny":["1px"]}}}]'
derived rules-extends '.extends = "rules-base.json" | .structural += [{"name":"child-border-style","selector":".surface","properties":{"border-style":{"deny":["solid"]}}}] | .rules |= map(if .id == "nested-cards" then .severity = "error" else . end)'
derived source-disable '.sourceRules |= map(if .id == "transition-all" then .enabled = false else . end)'
derived project-pack '(.structural += [{"name":"project-border-width","selector":".surface","properties":{"border-width":{"deny":["1px"]}}}]) | .rules |= map(if .id == "nested-cards" then .severity = "error" else . end)'
derived state-pack '.rules += [{"name":"line-length","selector":"*","operator":"all","when":{"all":[{"equals":[{"field":"visible"},true]},{"equals":[{"field":"hasText"},true]},{"gte":[{"field":"textLength"},120]},{"gt":[{"field":"chWidth"},0]},{"gt":[{"div":[{"sub":[{"field":"rect.w"},{"add":[{"field":"padding.1"},{"field":"padding.3"}]}]},{"field":"chWidth"}]},{"param":"maxCh"}]}]},"message":"line length above the pack limit","id":"line-length","params":{"maxCh":20},"severity":"error","layer":"layout"}]'
jqr default-pack-scrub "$pack" '[.rules[].id] | index("gpt-thin-border-wide-shadow") | not'
jqr default-pack-count "$pack" '(.rules | length == 66) and (.sourceRules | length == 10)'
jqr default-pack-fixed "$pack" '[.rules[].id] | (index("low-contrast") != null) and (index("heading-spacing-relation") != null)'

run clean 0 "$tmp/clean.json" --url "$clean_url" --no-sandbox --pretty
jqr clean-empty "$tmp/clean.json" '.findings == []'
jqr clean-colour-verified "$tmp/clean.json" '.unverified == []'

run generic 2 "$tmp/generic.json" --url "$generic_url" --no-sandbox --pretty
for check in page-overflow missing-lang skipped-heading missing-name target-size positive-tabindex low-contrast text-overflow; do
  jqr "generic-$check" "$tmp/generic.json" "[.findings[].check] | index(\"$check\") != null"
done
jqr generic-visible "$tmp/generic.json" '.summary.captured >= .summary.elements'

run generic-contrast-off 2 "$tmp/generic-contrast-off.json" --url "$generic_url" --no-sandbox --rules "$tmp/contrast-off.json" --pretty
jqr generic-contrast-off "$tmp/generic-contrast-off.json" '[.findings[].check] | index("low-contrast") == null'
jqr generic-contrast-off-summary "$tmp/generic-contrast-off.json" '.summary.byCheck.contrast == null'

run generic-contrast-error 2 "$tmp/generic-contrast-error.json" --url "$generic_url" --no-sandbox --rules "$tmp/contrast-error.json" --pretty
jqr generic-contrast-error "$tmp/generic-contrast-error.json" '[.findings[] | select(.check == "low-contrast")] | length > 0 and all(.severity == "error" and .layer == "source")'

run clean-contrast-off 0 "$tmp/clean-contrast-off.json" --url "$clean_url" --no-sandbox --rules "$tmp/contrast-off.json" --pretty

run diagnostics 2 "$tmp/diagnostics.json" --url "$(file_url diagnostics.html)" --no-sandbox --pretty
for check in console-error page-error network-error; do
  jqr "diagnostics-$check" "$tmp/diagnostics.json" "[.findings[].check] | index(\"$check\") != null"
done
jqr diagnostics-layer "$tmp/diagnostics.json" '[.findings[] | select(.check == "page-error") | .layer] | all(. == "runtime")'

run modern-colors 2 "$tmp/modern-colors.json" --url "$(file_url modern-colors.html)" --no-sandbox --pretty
jqr modern-colors-contrast "$tmp/modern-colors.json" '[.findings[] | select(.check == "low-contrast") | .evidence.color] | all(startswith("rgba("))'
jqr modern-colors-unverified "$tmp/modern-colors.json" '.unverified == []'

run frames 0 "$tmp/frames.json" --url "$(file_url frames.html)" --no-sandbox --pretty
jqr frames-unverified "$tmp/frames.json" '[.unverified[] | select(.check == "frames" and .frames == 1)] | length == 1'
jqr frames-shadow "$tmp/frames.json" '[.unverified[] | select(.check == "shadow-dom")] | length == 1'

run expanded 2 "$tmp/expanded.json" --url "$(file_url expanded.html)" --no-sandbox --rules "$pack" --pretty
run expanded-dark 2 "$tmp/expanded-dark.json" --url "$(file_url expanded-dark.html)" --no-sandbox --rules "$pack" --pretty
run hidden-share 2 "$tmp/hidden-share.json" --url "$(file_url hidden-share.html)" --no-sandbox --pretty
run uniform 2 "$tmp/uniform.json" --url "$(file_url uniform.html)" --no-sandbox --pretty
run short-tail 0 "$tmp/short-tail.json" --url "$(file_url short-tail.html)" --no-sandbox --pretty
jq -s '{checks: ([.[].findings[].check] | unique)}'   "$tmp/expanded.json" "$tmp/expanded-dark.json" "$tmp/hidden-share.json" "$tmp/uniform.json" "$tmp/short-tail.json"   >"$tmp/expanded-checks.json"
for check in   border-accent-on-rounded clipped-overflow-container cramped-padding   blinking-cursor theater-slop-phrase   first-viewport-column-overflow gray-on-color codex-grid-background   hard-offset-shadow heading-spacing-relation content-hidden-at-rest broken-image   organic-clip-path justified-text all-caps-body numbered-section-labels   italic-serif-display hero-eyebrow-chip pulsing-dot   radial-halo radial-spotlight-glow aphoristic-cadence overused-font   repeated-container-text repeating-stripes-gradient row-elevation-conflict edge-flush-cards   shape-assembled-illustration unbalanced-heading straggler sibling-text-collision   tiny-text text-occlusion tight-leading   undersized-ui-text monotonous-spacing body-text-viewport-edge buried-raster wide-tracking; do
  jqr "expanded-$check" "$tmp/expanded-checks.json" ".checks | index(\"$check\") != null"
done

run expanded-contrast-off 2 "$tmp/expanded-contrast-off.json" --url "$(file_url expanded.html)" --no-sandbox --rules "$tmp/contrast-off.json" --pretty
jqr expanded-contrast-off "$tmp/expanded-contrast-off.json" '[.findings[].check] | index("low-contrast") == null'
jqr expanded-contrast-off-unverified "$tmp/expanded-contrast-off.json" '[.unverified[] | select(.check == "low-contrast")] | length == 0'

run determinism-first 2 "$tmp/determinism-first.json" --url "$(file_url expanded.html)" --no-sandbox --rules "$pack" --theme both --viewport sweep --pretty
run determinism-second 2 "$tmp/determinism-second.json" --url "$(file_url expanded.html)" --no-sandbox --rules "$pack" --theme both --viewport sweep --pretty
if cmp -s "$tmp/determinism-first.json" "$tmp/determinism-second.json"; then
  ok
else
  bad "capture is not deterministic across two runs"
fi

run disclosure 2 "$tmp/disclosure.json" --url "$(file_url disclosure.html)" --no-sandbox --expand --pretty
jqr disclosure-expanded "$tmp/disclosure.json" '.states.expanded | length == 1'
jqr disclosure-skip "$tmp/disclosure.json" '[.states.skipped[] | select(.reason == "postcondition-not-met")] | length == 1'

run nested 2 "$tmp/nested.json" --url "$(file_url structure.html)" --no-sandbox --rules "$tmp/nested-error.json" --pretty
jqr nested-error "$tmp/nested.json" '[.findings[] | select(.check == "nested-cards" and .severity == "error" and .state == "default" and .layer == "layout")] | length == 1'

run cards 2 "$tmp/cards.json" --url "$(file_url cards.html)" --no-sandbox --rules "$tmp/nested-error.json" --pretty
jqr cards-surface "$tmp/cards.json" '[.findings[] | select(.check == "nested-cards")] | length == 2'

run state 2 "$tmp/state.json" --url "$(file_url state.html)" --no-sandbox --expand --rules "$tmp/state-pack.json" --pretty
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
run auto 0 "$tmp/auto.json" --url "$design_url" --no-sandbox --design auto --allow uniform-spacing,tight-container-inset,washed-grey-on-tint --pretty
jqr auto-report "$tmp/auto.json" '.designFile == ".ui-witness/design.json" and .inferredDesign.palette[0].count != null and .inferredDesign.palette[0].confidence != null'
jqr auto-file "$tmp/.ui-witness/design.json" '.color["color-1"] and .font["font-1"] and .typography["type-1"]'
jqr auto-file-descriptions "$tmp/.ui-witness/design.json" '[.. | objects | select(has("$description")) | .["$description"]] | length > 0 and all(.[]; test("^Inferred .+, [0-9]+ observations$"))'
jqr auto-file-statistics "$tmp/.ui-witness/design.json" '[paths | select(.[-1] == "count" or .[-1] == "confidence")] | length == 0'

run auto-existing 1 "$tmp/auto-existing.json" --url "$design_url" --no-sandbox --design auto
err_has auto-existing "design file already exists"

run auto-force 0 "$tmp/auto-force.json" --url "$design_url" --no-sandbox --design auto --allow uniform-spacing,tight-container-inset,washed-grey-on-tint --force

jq '.color = {"color-1": {"$value": "#000000", "$type": "color"}}' "$tmp/.ui-witness/design.json" >"$tmp/design-merge.json"
mv "$tmp/design-merge.json" "$tmp/.ui-witness/design.json"
run infer-missing 0 "$tmp/infer-missing.json" --url "$design_url" --no-sandbox --design auto --allow uniform-spacing,tight-container-inset,washed-grey-on-tint --infer-missing
jqr infer-missing-preserve "$tmp/.ui-witness/design.json" '.color["color-1"]["$value"] == "#000000"'
jqr infer-missing-fill "$tmp/.ui-witness/design.json" '.font["font-1"] != null and .radius["radius-1"] != null'
cd "$root"

jq 'del(.radius)' "$dtcg" >"$tmp/partial.json"
run partial 2 "$tmp/partial.json.report" --url "$design_url" --no-sandbox --design "$tmp/partial.json" --infer-missing --pretty
jqr partial-preserve "$tmp/partial.json" '.color.ink["$value"] == "#1a1a1a"'
jqr partial-fill "$tmp/partial.json" '.radius["radius-1"] != null'

run params 2 "$tmp/params.json" --url "$design_url" --no-sandbox --design "$dtcg" --rules "$tmp/params-missing.json" --pretty

run extends 2 "$tmp/extends.json" --url "$(file_url cards.html)" --no-sandbox --rules "$tmp/rules-extends.json" --pretty
jqr extends-named "$tmp/extends.json" '[.findings[] | select(.check == "nested-cards" and .severity == "error")] | length == 2'
jqr extends-base "$tmp/extends.json" '[.findings[] | select(.check == "base-border-width")] | length == 1'
jqr extends-child "$tmp/extends.json" '[.findings[] | select(.check == "child-border-style")] | length == 1'

run extends-pack 0 "$tmp/extends-pack.json" pack --rules "$tmp/rules-extends.json" --json
jqr extends-pack-sources "$tmp/extends-pack.json" '.sources[0].origin == "explicit" and .sources[1].origin == "extended" and (.sources[1].path | endswith("rules-base.json"))'

run extends-cycle 1 "$tmp/extends-cycle.json" --url "$clean_url" --no-sandbox --rules "$here/rules-cycle-a.json"
err_has extends-cycle "extends cycle"
run extends-missing 1 "$tmp/extends-missing.json" --url "$clean_url" --no-sandbox --rules "$here/rules-extends-missing.json"
err_has extends-missing "read rules file"
jqr params-missing "$tmp/params.json" '[.unverified[] | select(.check == "aphoristic-cadence" and (.reason | startswith("missing param:")) and .rule == "aphoristic-cadence")] | length == 1'

run dark 2 "$tmp/dark.json" --url "$(file_url dark-only.html)" --no-sandbox --theme both --pretty
jqr dark-themes "$tmp/dark.json" '.themes == ["light", "dark"]'
jqr dark-finding-theme "$tmp/dark.json" '[.findings[] | select(.check == "low-contrast") | .themes] | any(. == ["light"])'

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

for name in invalid-key invalid-property invalid-selector; do
  run "$name" 1 "$tmp/$name.json" --url "$clean_url" --no-sandbox --rules "$here/$name.json"
  err_has "$name" "rules:"
done
for name in invalid-rule-id invalid-param; do
  run "$name" 0 "$tmp/$name.json" --url "$clean_url" --no-sandbox --rules "$here/$name.json"
  jqr "$name-gap" "$tmp/$name.json" '.coverageGaps | length > 0'
done

run init 0 "$tmp/init.out" init "$tmp/project"
exists init-design "$tmp/project/.opal/design.json"
exists init-readme "$tmp/project/.opal/README.md"

mkdir -p "$tmp/home/.opal" "$tmp/project/.opal" "$tmp/neutral"
cp "$tmp/project-pack.json" "$tmp/project/.opal/rules.json"

run_in pack "$tmp/project" 0 "$tmp/pack.out" pack
if [ "$(sed -n '1p' "$tmp/pack.out")" = "$tmp/project/.opal/rules.json" ]; then
  ok
else
  bad "pack first source is not the project rules file"
fi
if [ -n "$(sed -n '2p' "$tmp/pack.out")" ]; then
  bad "pack reported a second rules file"
else
  ok
fi

run_in pack-explicit "$tmp/project" 0 "$tmp/pack-explicit.out" pack --rules "$tmp/nested-error.json"
if [ "$(sed -n '1p' "$tmp/pack-explicit.out")" = "$tmp/nested-error.json" ]; then
  ok
else
  bad "pack --rules does not take precedence"
fi

run_in overlay "$tmp/project" 2 "$tmp/overlay.json" audit "$(file_url cards.html)" --no-sandbox --pretty
jqr overlay-named-error "$tmp/overlay.json" '[.findings[] | select(.check == "nested-cards" and .severity == "error")] | length == 2'
jqr overlay-named-override "$tmp/overlay.json" '[.findings[] | select(.check == "nested-cards" and .severity == "advisory")] | length == 0'
jqr overlay-project-structural "$tmp/overlay.json" '[.findings[] | select(.check == "project-border-width")] | length == 1'
jqr overlay-sources "$tmp/overlay.json" '.sources.rules | length == 1 and (.[0] | endswith("/.opal/rules.json"))'
jqr overlay-rule-origins "$tmp/overlay.json" '.sources.rulesOrigin == ["project"]'
jqr overlay-schema "$tmp/overlay.json" '.schemaVersion == 1'
jqr overlay-design-source "$tmp/overlay.json" '(.sources.design | length == 1) and (.sources.design[0] | endswith("/.opal/design.json")) and (.sources.designOrigin == ["project"])'
jqr overlay-rule-source-project "$tmp/overlay.json" "[.findings[] | select(.check == \"project-border-width\") | .ruleSource] | all(contains(\"$tmp/project\"))"
jqr overlay-named-source "$tmp/overlay.json" "[.findings[] | select(.check == \"nested-cards\") | .ruleSource] | all(contains(\"$tmp/project\"))"

run_in audit-separator "$tmp/project" 2 "$tmp/audit-separator.json" audit -- "$(file_url cards.html)" --no-sandbox --pretty
jqr audit-separator-sources "$tmp/audit-separator.json" '.sources.rules | length == 1'

run_in pack-json "$tmp/project" 0 "$tmp/pack.json" pack --json
jqr pack-json-sources "$tmp/pack.json" '.sources | length == 1 and .[0].origin == "project" and (.[0].path | endswith("/.opal/rules.json"))'

run_in suppress "$tmp/project" 2 "$tmp/suppress.json" audit "$(file_url suppress.html)" --no-sandbox --pretty
jqr suppress-nested "$tmp/suppress.json" '[.findings[] | select(.check == "nested-cards")] | length == 1'
jqr suppress-selector "$tmp/suppress.json" '[.findings[] | select(.check == "nested-cards") | .selector] | all(contains(":nth-of-type(2)"))'

mkdir -p "$tmp/project/app/src"
run_in pack-walk "$tmp/project/app/src" 0 "$tmp/pack-walk.out" pack
if [ "$(sed -n '1p' "$tmp/pack-walk.out")" = "$tmp/project/.opal/rules.json" ]; then
  ok
else
  bad "pack from a nested directory did not find the project rules file"
fi

cat >"$tmp/project/.opal/design.json" <<'EOF'
{
  "color": {
    "ink": { "$value": "#1a1a1a", "$type": "color" }
  }
}
EOF
cat >"$tmp/home/.opal/design.json" <<'EOF'
{
  "color": {
    "paper": { "$value": "#ffffff", "$type": "color" }
  }
}
EOF
run_in design-resolve "$tmp/project" 2 "$tmp/design-resolve.json" audit "$(file_url design.html)" --no-sandbox --pretty
jqr design-source "$tmp/design-resolve.json" '(.sources.design | length == 2) and (.sources.design[0] | endswith("/.opal/design.json")) and (.sources.designOrigin == ["project", "user"])'
jqr design-declared "$tmp/design-resolve.json" '.parameters.declared == ["design"]'
jqr design-merge-ink "$tmp/design-resolve.json" '[.findings[] | select(.check == "design-color" and .evidence.value == "rgba(26, 26, 26, 1)")] | length == 0'
jqr design-merge-paper "$tmp/design-resolve.json" '[.findings[] | select(.check == "design-color" and .evidence.value == "rgba(255, 255, 255, 1)")] | length == 0'
jqr design-merge-drift "$tmp/design-resolve.json" '[.findings[] | select(.check == "design-color" and .evidence.value == "rgba(204, 0, 0, 1)")] | length == 1'

run_in design-walk "$tmp/project/app/src" 2 "$tmp/design-walk.json" audit "$(file_url design.html)" --no-sandbox --pretty
jqr design-walk-source "$tmp/design-walk.json" '(.sources.design | length == 2) and (.sources.design[0] | endswith("/.opal/design.json")) and (.sources.designOrigin == ["project", "user"])'

run_in design-user "$tmp/neutral" 2 "$tmp/design-user.json" audit "$(file_url design.html)" --no-sandbox --pretty
jqr design-user-source "$tmp/design-user.json" '(.sources.designOrigin == ["user"]) and (.sources.design | length == 1) and (.sources.design[0] | endswith("/.opal/design.json"))'

run_in design-explicit "$tmp/neutral" 2 "$tmp/design-explicit.json" audit "$(file_url design.html)" --no-sandbox --design "$dtcg" --pretty
jqr design-explicit-source "$tmp/design-explicit.json" '.sources.designOrigin == ["explicit", "user"] and (.sources.design | length == 2)'

run source-command 2 "$tmp/source-command.json" source "$here/source" --source-all --pretty
jqr source-command-token "$tmp/source-command.json" '[.findings[] | select(.check == "raw-color") | .file] | unique == ["component.tsx"]'
jqr source-command-schema "$tmp/source-command.json" '.schemaVersion == 1'

run format-sarif 2 "$tmp/sarif.json" --url "$(file_url cards.html)" --no-sandbox --rules "$tmp/nested-error.json" --format sarif
jqr format-sarif "$tmp/sarif.json" '.version == "2.1.0" and (.runs[0].results | length > 0) and (.runs[0].tool.driver.rules | length > 0) and ([.runs[0].results[].ruleId] | index("nested-cards") != null)'

run format-markdown 2 "$tmp/format.md" --url "$(file_url cards.html)" --no-sandbox --rules "$tmp/nested-error.json" --format markdown
file_has format-markdown "$tmp/format.md" "# opal:"
file_has format-markdown "$tmp/format.md" "nested-cards"

run format-github 2 "$tmp/format.github" --url "$(file_url cards.html)" --no-sandbox --rules "$tmp/nested-error.json" --format github
file_has format-github "$tmp/format.github" "::error title=nested-cards"

run source-format-sarif 2 "$tmp/source-format.json" source "$here/source" --source-all --format sarif
jqr source-format-sarif "$tmp/source-format.json" '[.runs[0].results[] | select(.ruleId == "raw-color") | .locations[0].physicalLocation.artifactLocation.uri] | index("component.tsx") != null'

run doctor 0 "$tmp/doctor.json" doctor --json
jqr doctor "$tmp/doctor.json" '.ok == true and ([.checks[] | select(.name == "chromium" and .ok)] | length == 1)'
run doctor-text 0 "$tmp/doctor.txt" doctor
file_has doctor-text "$tmp/doctor.txt" "ok chromium:"
run doctor-fail 1 "$tmp/doctor-fail.json" doctor --json --chrome "$here/fake-chrome.sh"
jqr doctor-fail "$tmp/doctor-fail.json" '.ok == false and ([.checks[] | select(.name == "chromium" and (.ok | not))] | length == 1)'
run doctor-rules 1 "$tmp/doctor-rules.json" doctor --json --rules "$here/invalid-rule-id.json"
jqr doctor-rules "$tmp/doctor-rules.json" '.ok == false and ([.checks[] | select(.name == "rules" and (.ok | not))] | length == 1)'

mkdir -p "$tmp/source-suppress"
cat >"$tmp/source-suppress/component.tsx" <<'EOF'
const a = "#ff0000";
const b = "#00ff00"; // opal-ignore raw-color
// opal-ignore-next-line raw-color
const c = "#0000ff";
EOF
cat >"$tmp/source-suppress/ignored.tsx" <<'EOF'
// opal-ignore-file raw-color
const d = "#123456";
EOF
run source-suppress 2 "$tmp/source-suppress.json" source "$tmp/source-suppress" --source-all --pretty
jqr source-suppress-count "$tmp/source-suppress.json" '.summary.findings == 1 and .summary.suppressed == 3'
jqr source-suppress-line "$tmp/source-suppress.json" '.findings[0].line == 1 and .findings[0].file == "component.tsx"'

mkdir -p "$tmp/baseline-project/.opal"
cp "$tmp/nested-error.json" "$tmp/baseline-project/.opal/rules.json"
run_in baseline-write "$tmp/baseline-project" 0 "$tmp/baseline-write.json" audit "$(file_url cards.html)" --no-sandbox --write-baseline --pretty
exists baseline-file "$tmp/baseline-project/.opal/baseline.json"
jqr baseline-written "$tmp/baseline-write.json" '.baseline.written > 0'

run_in baseline-read "$tmp/baseline-project" 0 "$tmp/baseline-read.json" audit "$(file_url cards.html)" --no-sandbox --pretty
jqr baseline-suppressed "$tmp/baseline-read.json" '.baseline.suppressed > 0 and .summary.findings == 0'

run_in baseline-disabled "$tmp/baseline-project" 2 "$tmp/baseline-disabled.json" audit "$(file_url cards.html)" --no-sandbox --no-baseline --pretty
jqr baseline-disabled-findings "$tmp/baseline-disabled.json" '.summary.findings > 0 and .baseline == null'

run_in baseline-explicit-write "$tmp/baseline-project" 0 "$tmp/baseline-explicit-write.json" audit "$(file_url cards.html)" --no-sandbox --baseline "$tmp/custom-baseline.json" --write-baseline --pretty
exists baseline-custom "$tmp/custom-baseline.json"
run_in baseline-explicit-read "$tmp/baseline-project" 0 "$tmp/baseline-explicit-read.json" audit "$(file_url cards.html)" --no-sandbox --baseline "$tmp/custom-baseline.json" --pretty
jqr baseline-explicit-suppressed "$tmp/baseline-explicit-read.json" '.baseline.suppressed > 0 and .summary.findings == 0'

run_in baseline-missing "$tmp/baseline-project" 1 "$tmp/baseline-missing.json" audit "$(file_url clean.html)" --no-sandbox --baseline "$tmp/missing-baseline.json"
err_has baseline-missing "baseline file not found"

before=$(jq '.fingerprints | length' "$tmp/baseline-project/.opal/baseline.json")
run_in baseline-update "$tmp/baseline-project" 0 "$tmp/baseline-update.json" audit "$(file_url generic.html)" --no-sandbox --update-baseline --pretty
after=$(jq '.fingerprints | length' "$tmp/baseline-project/.opal/baseline.json")
if [ "$after" -gt "$before" ]; then
  ok
else
  bad "baseline update did not add fingerprints"
fi

run_in source-baseline-write "$tmp/source-suppress" 0 "$tmp/source-baseline-write.json" source . --source-all --write-baseline --pretty
run_in source-baseline-read "$tmp/source-suppress" 0 "$tmp/source-baseline-read.json" source . --source-all --pretty
jqr source-baseline-suppressed "$tmp/source-baseline-read.json" '.baseline.suppressed == 1 and .summary.findings == 0'

cat >"$tmp/source-custom.json" <<'EOF'
{
  "version": 2,
  "sourceRules": [
    { "id": "raw-color", "enabled": false },
    { "id": "custom-todo", "severity": "P3", "layer": "source", "message": "todo marker", "pattern": "TODO" }
  ]
}
EOF
mkdir -p "$tmp/source-custom"
printf 'const color = "#ff0000";\n// TODO fixme\n' >"$tmp/source-custom/app.tsx"
run source-custom-rules 0 "$tmp/source-custom.json.out" source "$tmp/source-custom" --source-all --rules "$tmp/source-custom.json" --pretty
jqr source-custom-rules "$tmp/source-custom.json.out" '([.findings[].check] | index("raw-color") == null) and ([.findings[].check] | index("custom-todo") != null)'

cat >"$tmp/impeccable.html" <<'EOF'
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <style>
    body { margin: 0; background: #000000; color: #ffffff; font-family: system-ui, sans-serif; }
    .glass { backdrop-filter: blur(8px); padding: 12px; transition: all 0.3s ease; }
    .glow { background: #101218; box-shadow: 0 0 28px rgba(59, 130, 246, 0.7); padding: 12px; }
    .fake-btn { cursor: pointer; display: inline-block; padding: 8px; }
    .grid { display: grid; grid-template-columns: repeat(3, 220px); gap: 16px; }
    .card { width: 220px; height: 140px; background: #141820; border: 1px solid #333; padding: 16px; box-shadow: 0 2px 8px rgba(0,0,0,0.2); border-radius: 8px; }
    .tile { width: 40px; height: 40px; border-radius: 8px; background: rgba(59, 130, 246, 0.2); display: flex; align-items: center; justify-content: center; }
    .kpi { font-size: 42px; font-weight: 700; }
  </style>
</head>
<body>
  <section class="glass glow">
    <div class="fake-btn">Action</div>
    <div class="tile"><svg width="20" height="20"><circle cx="10" cy="10" r="8" /></svg></div>
    <h3>Feature</h3>
    <div class="kpi">99.9%</div>
  </section>
  <section class="grid">
    <div class="card">Card 1 body text for layout</div>
    <div class="card">Card 2 body text for layout</div>
    <div class="card">Card 3 body text for layout</div>
  </section>
  <dialog open>Blocking dialog</dialog>
</body>
</html>
EOF
saved_home=$HOME
export HOME="$tmp/clean-home"
mkdir -p "$HOME"
run impeccable-rules 2 "$tmp/impeccable.json" --url "file://$tmp/impeccable.html" --no-sandbox --pretty
for check in sheet-selection sheet-tap-highlight icon-tile-stack hero-metric-template transition-all clickable-non-control identical-card-grid; do
  jqr "impeccable-$check" "$tmp/impeccable.json" "[.findings[].check] | index(\"$check\") != null"
done

cat >"$tmp/light-single-card.html" <<'EOF'
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <style>
    body { margin: 0; background: #ffffff; color: #000000; font-family: system-ui, sans-serif; }
    .card { width: 240px; height: 140px; border: 1px solid #d0d0d0; padding: 16px; box-shadow: 0 2px 8px rgba(0,0,0,0.08); border-radius: 8px; transition: all 0.2s ease; }
    @media (prefers-reduced-motion: reduce) {}
  </style>
</head>
<body>
  <main>
    <div class="card">Single card on white page</div>
  </main>
</body>
</html>
EOF
run light-single-card 0 "$tmp/light-single-card.json" --url "file://$tmp/light-single-card.html" --rules "$tmp/source-disable.json" --no-sandbox --pretty
jqr light-no-pure-black "$tmp/light-single-card.json" '[.findings[].check] | index("pure-black-white") == null'
jqr light-no-identical-grid "$tmp/light-single-card.json" '[.findings[].check] | index("identical-card-grid") == null'
jqr source-disable-keeps-rendered-transition-all "$tmp/light-single-card.json" '[.findings[].check] | index("transition-all") != null'
export HOME="$saved_home"

cat >"$tmp/invalid-source-regex.json" <<'EOF'
{
  "version": 2,
  "sourceRules": [
    { "id": "broken", "message": "broken", "pattern": "[unclosed" }
  ]
}
EOF
run invalid-source-regex 1 "$tmp/invalid-source-regex.out" source "$tmp/source-custom" --rules "$tmp/invalid-source-regex.json"
err_has invalid-source-regex "rules:"

mkdir -p "$tmp/det-home-a/.opal" "$tmp/det-home-b/.opal"
printf '{"version":2,"rules":[{"id":"nested-cards","enabled":false}]}\n' >"$tmp/det-home-a/.opal/rules.json"
printf '{"version":2,"rules":[]}\n' >"$tmp/det-home-b/.opal/rules.json"
HOME="$tmp/det-home-a" "$bin" audit "$clean_url" --no-sandbox --rules "$pack" --pretty >"$tmp/det-audit-a.json" 2>"$tmp/det-audit-a.err"
HOME="$tmp/det-home-b" "$bin" audit "$clean_url" --no-sandbox --rules "$pack" --pretty >"$tmp/det-audit-b.json" 2>"$tmp/det-audit-b.err"
if cmp -s "$tmp/det-audit-a.json" "$tmp/det-audit-b.json"; then ok; else bad "audit output changed with HOME"; fi
HOME="$tmp/det-home-a" "$bin" pack --json >"$tmp/det-pack-a.json" 2>"$tmp/det-pack-a.err"
HOME="$tmp/det-home-b" "$bin" pack --json >"$tmp/det-pack-b.json" 2>"$tmp/det-pack-b.err"
if cmp -s "$tmp/det-pack-a.json" "$tmp/det-pack-b.json"; then ok; else bad "pack output changed with HOME"; fi

cat >"$tmp/clean-cards.html" <<'EOF'
<!doctype html><html lang="en"><head><style>.card{width:220px;height:120px;background:#f2f2f2;padding:16px;border-radius:8px}</style></head><body><main><section class="card">Basic</section><section class="card">Pro</section><section class="card">Team</section></main></body></html>
EOF
run negative-cards 2 "$tmp/negative-cards.json" --url "file://$tmp/clean-cards.html" --no-sandbox --rules "$pack" --allow identical-card-grid --pretty
jqr negative-cards-clean "$tmp/negative-cards.json" '[.findings[].check] | index("identical-card-grid") == null'

cat >"$tmp/clean-kicker.html" <<'EOF'
<!doctype html><html lang="en"><head><style>.k{font-size:12px;text-transform:uppercase;letter-spacing:.08em}h1{font-size:32px}</style></head><body><span class="k" data-opal-ignore="kicker-above-heading">Docs</span><h1>Guide</h1><p>Body copy.</p></body></html>
EOF
run negative-kicker 2 "$tmp/negative-kicker.json" --url "file://$tmp/clean-kicker.html" --no-sandbox --rules "$pack" --pretty
jqr negative-kicker-clean "$tmp/negative-kicker.json" '[.findings[].check] | index("kicker-above-heading") == null'

cat >"$tmp/clean-chip.html" <<'EOF'
<!doctype html><html lang="en"><head><style>.chip{display:inline-block;padding:4px 10px;border-radius:999px;background:#eee;font-size:12px;letter-spacing:.08em;text-transform:uppercase}h1{font-size:48px}</style></head><body><span class="chip">Product</span><h1>Build faster</h1></body></html>
EOF
run negative-chip 2 "$tmp/negative-chip.json" --url "file://$tmp/clean-chip.html" --no-sandbox --rules "$pack" --allow hero-eyebrow-chip,kicker-above-heading --pretty
jqr negative-chip-clean "$tmp/negative-chip.json" '[.findings[].check] | index("hero-eyebrow-chip") == null and index("kicker-above-heading") == null'

echo "opal tests: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
