#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
mkdir -p "$HOME/.opal/bin"
cargo build --release --manifest-path "$root/ui-probe/Cargo.toml"
install -m755 "$root/ui-probe/target/release/ui-probe" "$HOME/.opal/bin/ui-probe"
install -m644 "$root/skills/opal/references/impeccable-rules.json" "$HOME/.opal/rules.json"
echo "installed $HOME/.opal/bin/ui-probe"
