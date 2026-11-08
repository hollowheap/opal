#!/usr/bin/env bash
set -u

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
bin=${UI_PROBE_BIN:-$root/ui-probe/target/debug/ui-probe}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export HOME="$tmp/home"
mkdir -p "$HOME"

if [ ! -x "$bin" ]; then
  echo "missing ui-probe binary: $bin" >&2
  echo "build it with: cargo build --manifest-path ui-probe/Cargo.toml" >&2
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

run_in() {
  name=$1
  dir=$2
  expected=$3
  out=$4
  shift 4
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

run clean 0 "$tmp/clean.json" --url "$clean_url" --no-sandbox --pretty
jqr clean-empty "$tmp/clean.json" '.findings == []'
jqr clean-colour-verified "$tmp/clean.json" '.unverified == []'

run generic 2 "$tmp/generic.json" --url "$generic_url" --no-sandbox --pretty
for check in page-overflow missing-lang heading-skip missing-name target-size positive-tabindex contrast clipped-text; do
  jqr "generic-$check" "$tmp/generic.json" "[.findings[].check] | index(\"$check\") != null"
done
jqr generic-visible "$tmp/generic.json" '.summary.captured >= .summary.elements'

run diagnostics 2 "$tmp/diagnostics.json" --url "$(file_url diagnostics.html)" --no-sandbox --pretty
for check in console-error page-error network-error; do
  jqr "diagnostics-$check" "$tmp/diagnostics.json" "[.findings[].check] | index(\"$check\") != null"
done
jqr diagnostics-layer "$tmp/diagnostics.json" '[.findings[] | select(.check == "page-error") | .layer] | all(. == "runtime")'

run modern-colors 2 "$tmp/modern-colors.json" --url "$(file_url modern-colors.html)" --no-sandbox --pretty
jqr modern-colors-contrast "$tmp/modern-colors.json" '[.findings[] | select(.check == "contrast") | .evidence.color] | all(startswith("rgba("))'
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
for check in   accent-border-rounded continuous-scroller clipped-escape-child colored-halo-shadow   tight-container-inset cream-canvas decorative-blinking-caret dismissive-copy   opening-column-overrun compressed-type-scale washed-grey-on-tint grid-line-background   unblurred-offset-shadow heading-spacing-relation hidden-text-share image-source-missing   irregular-clip-path justified-narrow-text literal-uppercase-passage numeric-section-index   oversized-headline oversized-italic-serif pill-eyebrow-label pulsing-status-dot   radial-background-wash radial-spotlight rebuttal-cadence reflex-font-choice   repeated-container-copy repeating-stripe-surface row-elevation-conflict scroller-edge-flush   primitive-svg-scene short-heading-tail short-last-line sibling-text-collision   sub-12px-body synthetic-purple-palette overlapping-copy tight-line-height   sub-11px-control-text uniform-spacing viewport-edge-text washed-out-image excess-letter-spacing; do
  jqr "expanded-$check" "$tmp/expanded-checks.json" ".checks | index(\"$check\") != null"
done

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

run params 2 "$tmp/params.json" --url "$design_url" --no-sandbox --design "$dtcg" --rules "$here/rules-params.json" --pretty

run extends 2 "$tmp/extends.json" --url "$(file_url cards.html)" --no-sandbox --rules "$here/rules-extends.json" --pretty
jqr extends-named "$tmp/extends.json" '[.findings[] | select(.check == "nested-cards" and .severity == "error")] | length == 2'
jqr extends-base "$tmp/extends.json" '[.findings[] | select(.check == "base-border-width")] | length == 1'
jqr extends-child "$tmp/extends.json" '[.findings[] | select(.check == "child-border-style")] | length == 1'

run extends-pack 0 "$tmp/extends-pack.json" pack --rules "$here/rules-extends.json" --json
jqr extends-pack-sources "$tmp/extends-pack.json" '.sources[0].origin == "explicit" and .sources[1].origin == "extended" and (.sources[1].path | endswith("rules-base.json"))'

run extends-cycle 1 "$tmp/extends-cycle.json" --url "$clean_url" --no-sandbox --rules "$here/rules-cycle-a.json"
err_has extends-cycle "extends cycle"
run extends-missing 1 "$tmp/extends-missing.json" --url "$clean_url" --no-sandbox --rules "$here/rules-extends-missing.json"
err_has extends-missing "read rules file"
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

run init 0 "$tmp/init.out" init "$tmp/project"
exists init-design "$tmp/project/.opal/design.json"
exists init-readme "$tmp/project/.opal/README.md"

mkdir -p "$tmp/home/.opal" "$tmp/project/.opal" "$tmp/neutral"
cat >"$tmp/home/.opal/rules.json" <<'EOF'
{
  "version": 1,
  "pack": "user",
  "structural": [
    {
      "name": "user-border-style",
      "selector": ".surface",
      "properties": {
        "border-style": { "deny": ["solid"] }
      }
    }
  ],
  "rules": [
    { "id": "nested-cards", "enabled": true, "severity": "advisory", "layer": "layout" },
    { "id": "line-length", "enabled": true, "severity": "advisory", "layer": "layout", "params": { "maxCh": 40 } }
  ]
}
EOF
cat >"$tmp/project/.opal/rules.json" <<'EOF'
{
  "version": 1,
  "pack": "project",
  "structural": [
    {
      "name": "project-border-width",
      "selector": ".surface",
      "properties": {
        "border-width": { "deny": ["1px"] }
      }
    }
  ],
  "rules": [
    { "id": "nested-cards", "enabled": true, "severity": "error", "layer": "layout" }
  ]
}
EOF

run_in pack "$tmp/project" 0 "$tmp/pack.out" pack
if [ "$(sed -n '1p' "$tmp/pack.out")" = "$tmp/project/.opal/rules.json" ]; then
  ok
else
  bad "pack first source is not the project rules file"
fi
if [ "$(sed -n '2p' "$tmp/pack.out")" = "$tmp/home/.opal/rules.json" ]; then
  ok
else
  bad "pack second source is not the user rules file"
fi

run_in pack-explicit "$tmp/project" 0 "$tmp/pack-explicit.out" pack --rules "$here/rules-error.json"
if [ "$(sed -n '1p' "$tmp/pack-explicit.out")" = "$here/rules-error.json" ]; then
  ok
else
  bad "pack --rules does not take precedence"
fi

run_in overlay "$tmp/project" 2 "$tmp/overlay.json" audit "$(file_url cards.html)" --no-sandbox --pretty
jqr overlay-named-error "$tmp/overlay.json" '[.findings[] | select(.check == "nested-cards" and .severity == "error")] | length == 2'
jqr overlay-named-override "$tmp/overlay.json" '[.findings[] | select(.check == "nested-cards" and .severity == "advisory")] | length == 0'
jqr overlay-project-structural "$tmp/overlay.json" '[.findings[] | select(.check == "project-border-width")] | length == 1'
jqr overlay-user-structural "$tmp/overlay.json" '[.findings[] | select(.check == "user-border-style")] | length == 1'
jqr overlay-sources "$tmp/overlay.json" '.sources.rules | length == 2 and (.[0] | endswith("/.opal/rules.json")) and (.[1] | endswith("/.opal/rules.json"))'
jqr overlay-rule-origins "$tmp/overlay.json" '.sources.rulesOrigin == ["project", "user"]'
jqr overlay-schema "$tmp/overlay.json" '.schemaVersion == 1'
jqr overlay-design-source "$tmp/overlay.json" '(.sources.design | length == 1) and (.sources.design[0] | endswith("/.opal/design.json")) and (.sources.designOrigin == ["project"])'
jqr overlay-rule-source-project "$tmp/overlay.json" "[.findings[] | select(.check == \"project-border-width\") | .ruleSource] | all(contains(\"$tmp/project\"))"
jqr overlay-rule-source-user "$tmp/overlay.json" "[.findings[] | select(.check == \"user-border-style\") | .ruleSource] | all(contains(\"$tmp/home\"))"
jqr overlay-named-source "$tmp/overlay.json" "[.findings[] | select(.check == \"nested-cards\") | .ruleSource] | all(contains(\"$tmp/project\"))"

run_in audit-separator "$tmp/project" 2 "$tmp/audit-separator.json" audit -- "$(file_url cards.html)" --no-sandbox --pretty
jqr audit-separator-sources "$tmp/audit-separator.json" '.sources.rules | length == 2'

run_in pack-json "$tmp/project" 0 "$tmp/pack.json" pack --json
jqr pack-json-sources "$tmp/pack.json" '.sources | length == 2 and .[0].origin == "project" and (.[0].path | endswith("/.opal/rules.json")) and .[1].origin == "user"'

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

run format-sarif 2 "$tmp/sarif.json" --url "$(file_url cards.html)" --no-sandbox --rules "$here/rules-error.json" --format sarif
jqr format-sarif "$tmp/sarif.json" '.version == "2.1.0" and (.runs[0].results | length > 0) and (.runs[0].tool.driver.rules | length > 0) and ([.runs[0].results[].ruleId] | index("nested-cards") != null)'

run format-markdown 2 "$tmp/format.md" --url "$(file_url cards.html)" --no-sandbox --rules "$here/rules-error.json" --format markdown
file_has format-markdown "$tmp/format.md" "# ui-probe:"
file_has format-markdown "$tmp/format.md" "nested-cards"

run format-github 2 "$tmp/format.github" --url "$(file_url cards.html)" --no-sandbox --rules "$here/rules-error.json" --format github
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
cp "$here/rules-error.json" "$tmp/baseline-project/.opal/rules.json"
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

echo "ui-probe tests: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
