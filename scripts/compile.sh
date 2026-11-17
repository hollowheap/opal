#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cargo build --release --manifest-path "$root/opal/Cargo.toml"
install -m755 "$root/opal/target/release/opal" "$root/scripts/opal"
install -m644 "$root/skills/opal/references/default-rules.json" "$root/scripts/rules.json"
echo "installed $root/scripts/opal"