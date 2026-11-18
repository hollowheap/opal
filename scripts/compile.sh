#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
skill_dir=${OPAL_SKILL_DIR:-}
if [ -z "$skill_dir" ]; then
  echo "OPAL_SKILL_DIR is required" >&2
  exit 1
fi
if [ ! -d "$skill_dir" ]; then
  echo "OPAL_SKILL_DIR is not a directory: $skill_dir" >&2
  exit 1
fi
cargo build --release --manifest-path "$root/opal/Cargo.toml"
install -d -m755 "$skill_dir/scripts"
install -m755 "$root/opal/target/release/opal" "$skill_dir/scripts/opal"
echo "installed $skill_dir/scripts/opal"
