#!/usr/bin/env bash
set -euo pipefail

url=${1:?usage: determinism-check.sh <url> [ui-probe flags...]}
shift

bin=${UI_PROBE_BIN:-$HOME/.opal/bin/ui-probe}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

set +e
"$bin" audit "$url" --no-sandbox --pretty "$@" >"$tmp/first.json" 2>"$tmp/first.err"
first=$?
"$bin" audit "$url" --no-sandbox --pretty "$@" >"$tmp/second.json" 2>"$tmp/second.err"
second=$?
set -e

if [ "$first" -ne 0 ] && [ "$first" -ne 2 ]; then
  echo "first run failed with $first" >&2
  sed -n '1,40p' "$tmp/first.err" >&2
  exit 1
fi
if [ "$second" -ne 0 ] && [ "$second" -ne 2 ]; then
  echo "second run failed with $second" >&2
  sed -n '1,40p' "$tmp/second.err" >&2
  exit 1
fi

if cmp -s "$tmp/first.json" "$tmp/second.json"; then
  echo "deterministic: $url"
else
  echo "nondeterministic: $url" >&2
  diff -u "$tmp/first.json" "$tmp/second.json" | sed -n '1,160p' >&2
  exit 1
fi
