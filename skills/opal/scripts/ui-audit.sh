#!/usr/bin/env bash
set -euo pipefail

self=$(readlink -f "$0")
skill_dir=$(cd "$(dirname "$self")/.." && pwd)
pack="$skill_dir/references/impeccable-rules.json"
config_dir=".ui-audit"
design="$config_dir/design.json"
rules="$config_dir/rules.json"

usage() {
  cat <<'EOF'
usage: ui-audit <command> [options]

  init [dir]            create .ui-audit/design.json and instructions
  audit <url> [flags]   run the rendered audit
  source [dir] [flags]  run the source scan
  pack                  print the bundled rules pack path

Flags after the target are passed to ui-probe. Use -- to separate them.
EOF
}

supports_rules() {
  command -v ui-probe >/dev/null 2>&1 && ui-probe --help 2>&1 | grep -q -- '--rules'
}

require_binary() {
  if ! command -v ui-probe >/dev/null 2>&1; then
    echo "ui-audit: ui-probe is not on PATH" >&2
    exit 1
  fi
}

command=${1:-}
[ $# -gt 0 ] && shift

case "$command" in
  init)
    target=${1:-.}
    shift || true
    mkdir -p "$target/$config_dir"

    if [ ! -f "$target/$design" ]; then
      printf '{}\n' > "$target/$design"
    fi

    cat > "$target/$config_dir/README.md" <<'EOF'
# ui-audit

- `design.json` holds the project's design parameters: colours, font families,
  spacing, radii, surfaces, and motion ranges. Every key is optional.
- `rules.json` is optional. Copy the bundled pack only when the project needs
  different selectors, thresholds, severities, or disabled rules.
- The bundled pack lives with the review skill and is used when no project
  override exists.

Run:

    scripts/ui-audit.sh audit http://localhost:3000 -- --theme both --viewport sweep

Read findings, fix the code, run again. A finding carries `layer`; `layout` and
`sheet` are structural, `content` is copy, `source` is file text.
EOF

    echo "created $target/$design"
    echo "next: fill in design parameters, then run 'ui-audit audit <url>'"
    ;;

  audit)
    require_binary
    url=${1:-}
    if [ -z "$url" ]; then
      echo "ui-audit: audit needs a URL" >&2
      exit 1
    fi
    shift || true
    [ "${1:-}" = "--" ] && shift

    args=()
    if [ -f "$design" ]; then
      args+=(--design "$design" --infer-missing)
    fi

    if [ -f "$rules" ]; then
      if supports_rules; then
        args+=(--rules "$rules")
      else
        echo "ui-audit: project rules ignored; ui-probe has no --rules support" >&2
      fi
    elif [ -f "$pack" ] && supports_rules; then
      args+=(--rules "$pack")
    fi

    exec ui-probe --url "$url" "${args[@]}" "$@"
    ;;

  source)
    require_binary
    target=${1:-.}
    if [ $# -gt 0 ]; then shift; fi
    [ "${1:-}" = "--" ] && shift
    exec ui-probe --source "$target" "$@"
    ;;

  pack)
    if [ ! -f "$pack" ]; then
      echo "ui-audit: bundled pack not found at $pack" >&2
      exit 1
    fi
    printf '%s\n' "$pack"
    ;;

  ""|-h|--help)
    usage
    ;;

  *)
    echo "ui-audit: unknown command: $command" >&2
    usage >&2
    exit 1
    ;;
esac
