#!/usr/bin/env bash
set -euo pipefail

workspace="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
host_arch="$(uname -m)"
case "$host_arch" in
  arm64) target=aarch64-apple-darwin ;;
  x86_64) target=x86_64-apple-darwin ;;
  *) echo "Unsupported macOS architecture: $host_arch (expected arm64 or x86_64)." >&2; exit 1 ;;
esac
if ! rustup target list --installed | rg -qx "$target"; then
  echo "Rust target $target is not installed. Run: rustup target add $target" >&2
  exit 1
fi

cd "$workspace"
export MACOSX_DEPLOYMENT_TARGET=14.0
cargo build --target "$target" -p demo-protocol --bin demo-protocol
cargo build --target "$target" -p hmi-desktop --bin hmi-cli
plugin_directory="$workspace/.cache/plugins/demo-temperature"
mkdir -p "$plugin_directory"
cp "$workspace/target/$target/debug/demo-protocol" "$plugin_directory/demo-protocol"
node --input-type=module - "$workspace/crates/demo-protocol/package/manifest.json" "$plugin_directory/manifest.json" "$target" <<'NODE'
import fs from 'node:fs'
const [source, destination, target] = process.argv.slice(2)
const manifest = JSON.parse(fs.readFileSync(source, 'utf8'))
manifest.target = target
manifest.executable = 'demo-protocol'
fs.writeFileSync(destination, `${JSON.stringify(manifest, null, 2)}\n`)
NODE
# The live preview is a separate Runner process, so development must stage one.
npm exec vite -- build --config vite.runner.config.ts
(
  cd "$workspace/apps/runner/src-tauri"
  "$workspace/node_modules/.bin/tauri" build --debug --target "$target" --bundles app --no-sign --ci --features custom-protocol
)
architecture=arm64
if [[ "$target" == x86_64-apple-darwin ]]; then architecture=x64; fi
runner_template="$workspace/.cache/templates/macos-$architecture/HmiRunner.app"
mkdir -p "$(dirname "$runner_template")"
rm -rf "$runner_template"
ditto "$workspace/target/$target/debug/bundle/macos/HMI Runtime.app" "$runner_template"
echo "Demo plugin staged at $plugin_directory for $target."
echo "Use HMI_RESOURCE_ROOT=.cache when launching the Studio development app."
