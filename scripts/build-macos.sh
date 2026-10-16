#!/usr/bin/env bash
set -euo pipefail

workspace="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
skip_install=0
skip_python=0
for arg in "$@"; do
  case "$arg" in
    --skip-install) skip_install=1 ;;
    --skip-python) skip_python=1 ;;
    -h|--help)
      cat <<'EOF'
Build native macOS Studio and Runner app bundles for the current architecture.

Usage: scripts/build-macos.sh [--skip-install] [--skip-python]

Run once natively on Apple Silicon for arm64 and on an Intel Mac for x86_64.
This script does not create universal binaries or cross-architecture bundles.
EOF
      exit 0
      ;;
    *) echo "Unknown argument: $arg" >&2; exit 2 ;;
  esac
done

host_arch="$(uname -m)"
case "$host_arch" in
  arm64) architecture=arm64; target=aarch64-apple-darwin ;;
  x86_64) architecture=x64; target=x86_64-apple-darwin ;;
  *) echo "Unsupported macOS architecture: $host_arch (expected arm64 or x86_64)." >&2; exit 1 ;;
esac

version="$(sw_vers -productVersion)"
major="${version%%.*}"
if (( major < 14 )); then
  echo "macOS 14 or newer is required; this host is macOS $version." >&2
  exit 1
fi

if ! rustup target list --installed | rg -qx "$target"; then
  echo "Rust target $target is not installed. Run: rustup target add $target" >&2
  exit 1
fi

cd "$workspace"
export MACOSX_DEPLOYMENT_TARGET=14.0
if (( skip_install == 0 )); then
  npm ci --no-audit --no-fund
fi
npm run build

runtime_id="3.14.8-$target"
python_source="$workspace/.cache/python/$runtime_id"
if (( skip_python == 0 )); then
  prep="$workspace/scripts/prepare-python-macos.py"
  if [[ ! -f "$prep" ]]; then
    echo "Missing $prep; use --skip-python to build without optional Python resources." >&2
    exit 1
  fi
  python3 "$prep" --architecture "$host_arch"
elif [[ -d "$python_source" ]]; then
  echo "Including cached Python runtime: $python_source"
else
  echo "Skipping Python runtime; projects with Python nodes will need it imported before preview/export."
fi

cargo build --release --target "$target" -p hmi-desktop --bin hmi-cli

tauri="$workspace/node_modules/.bin/tauri"
profile=release
(
  cd "$workspace/apps/studio/src-tauri"
  "$tauri" build --target "$target" --bundles app --no-sign --ci --features custom-protocol
)
(
  cd "$workspace/apps/runner/src-tauri"
  "$tauri" build --target "$target" --bundles app --no-sign --ci --features custom-protocol
)

bundle_root="$workspace/target/$target/$profile/bundle/macos"
studio_bundle="$bundle_root/HMI Design Studio.app"
runner_bundle="$bundle_root/HMI Runtime.app"
if [[ ! -d "$studio_bundle" || ! -d "$runner_bundle" ]]; then
  echo "Expected Tauri app bundles under $bundle_root; found:" >&2
  find "$bundle_root" -maxdepth 1 -print 2>/dev/null >&2 || true
  exit 1
fi

template="$workspace/.cache/templates/macos-$architecture/HmiRunner.app"
mkdir -p "$(dirname "$template")"
rm -rf "$template"
ditto "$runner_bundle" "$template"

stage_studio_resources() {
  local app="$1"
  local resources="$app/Contents/Resources"
  if [[ -d "$workspace/.cache/plugins" ]]; then
    mkdir -p "$resources/plugins"
    for plugin in "$workspace"/.cache/plugins/*; do
      [[ -e "$plugin" ]] || continue
      [[ "$(basename "$plugin")" == "demo-temperature" ]] && continue
      cp -R "$plugin" "$resources/plugins/"
    done
  fi
  if [[ -d "$python_source" ]]; then
    mkdir -p "$resources/python"
    rm -rf "$resources/python/$runtime_id"
    ditto "$python_source" "$resources/python/$runtime_id"
  fi
}

stage_studio_resources "$studio_bundle"
mkdir -p "$studio_bundle/Contents/Resources/templates/macos-$architecture"
rm -rf "$studio_bundle/Contents/Resources/templates/macos-$architecture/HmiRunner.app"
ditto "$template" "$studio_bundle/Contents/Resources/templates/macos-$architecture/HmiRunner.app"

distribution="$workspace/artifacts/HMIStudio-macos-$architecture-$(date '+%Y%m%d-%H%M%S')"
ditto "$studio_bundle" "$distribution/HMI Design Studio.app"
ditto "$template" "$distribution/HmiRunner.app"
if [[ -d "$workspace/.cache/plugins" ]]; then
  mkdir -p "$distribution/plugins"
  for plugin in "$workspace"/.cache/plugins/*; do
    [[ -e "$plugin" ]] || continue
    [[ "$(basename "$plugin")" == "demo-temperature" ]] && continue
    cp -R "$plugin" "$distribution/plugins/"
  done
fi
if [[ -d "$workspace/.cache/plugins" ]]; then
  mkdir -p "$distribution/HmiRunner.app/Contents/Resources/plugins"
  for plugin in "$workspace"/.cache/plugins/*; do
    [[ -e "$plugin" ]] || continue
    [[ "$(basename "$plugin")" == "demo-temperature" ]] && continue
    cp -R "$plugin" "$distribution/HmiRunner.app/Contents/Resources/plugins/"
  done
fi
cp "$workspace/target/$target/$profile/hmi-cli" "$distribution/hmi-cli"
cp "$workspace/README.md" "$distribution/README.md"

for app in "$distribution/HMI Design Studio.app" "$distribution/HmiRunner.app"; do
  codesign --force --deep --sign - "$app"
  codesign --verify --deep --strict "$app"
done

node "$workspace/scripts/test-native-macos.mjs" \
  --studio "$distribution/HMI Design Studio.app" \
  --runner "$distribution/HmiRunner.app"

echo "Studio app: $distribution/HMI Design Studio.app"
echo "Runner template: $distribution/HmiRunner.app"
echo "CLI: $distribution/hmi-cli"
