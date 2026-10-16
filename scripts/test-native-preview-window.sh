#!/usr/bin/env bash
set -euo pipefail

workspace="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ "$(uname -s)" != Darwin ]]; then
  echo 'Native preview-window verification requires macOS.' >&2
  exit 1
fi
version="$(sw_vers -productVersion)"
major="${version%%.*}"
if (( major < 14 )); then
  echo "macOS 14 or newer is required; this host is macOS $version." >&2
  exit 1
fi
case "$(uname -m)" in
  arm64) target=aarch64-apple-darwin ;;
  x86_64) target=x86_64-apple-darwin ;;
  *) echo "Unsupported macOS architecture: $(uname -m)" >&2; exit 1 ;;
esac
if ! rustup target list --installed | rg -qx "$target"; then
  echo "Rust target $target is not installed." >&2
  exit 1
fi

export MACOSX_DEPLOYMENT_TARGET=14.0
cd "$workspace"
npm run build
npm run stage:dev:macos
tauri="$workspace/node_modules/.bin/tauri"
architecture=arm64
if [[ "$target" == x86_64-apple-darwin ]]; then architecture=x64; fi
runner_template="$workspace/.cache/templates/macos-$architecture/HmiRunner.app"
mkdir -p "$(dirname "$runner_template")"
rm -rf "$runner_template"
(
  cd "$workspace/apps/runner"
  "$tauri" build --debug --target "$target" --bundles app --no-sign --ci \
    --features custom-protocol,native-preview-smoke
)
runner_debug="$workspace/target/$target/debug/bundle/macos/HMI Runtime.app"
if [[ ! -d "$runner_debug" ]]; then
  echo "Tauri did not create the debug Runner app bundle: $runner_debug" >&2
  exit 1
fi
ditto "$runner_debug" "$runner_template"
(
  cd "$workspace/apps/studio"
  "$tauri" build --debug --target "$target" --bundles app --no-sign --ci \
    --features custom-protocol,native-preview-smoke
)

app="$workspace/target/$target/debug/bundle/macos/HMI Design Studio.app"
if [[ ! -d "$app" ]]; then
  echo "Tauri did not create the debug app bundle: $app" >&2
  exit 1
fi
plugins="$app/Contents/Resources/plugins"
mkdir -p "$plugins"
rm -rf "$plugins/demo-temperature"
ditto "$workspace/.cache/plugins/demo-temperature" "$plugins/demo-temperature"
mkdir -p "$app/Contents/Resources/templates/macos-$architecture"
rm -rf "$app/Contents/Resources/templates/macos-$architecture/HmiRunner.app"
ditto "$runner_template" "$app/Contents/Resources/templates/macos-$architecture/HmiRunner.app"

report="$workspace/artifacts/native-preview-window-$architecture-$(date '+%Y%m%d-%H%M%S').json"
child_report="$workspace/artifacts/native-preview-child-$architecture-$(date '+%Y%m%d-%H%M%S').json"
mkdir -p "$(dirname "$report")"
rm -f "$child_report"
HMI_RESOURCE_ROOT="$app/Contents/Resources" HMI_NATIVE_PREVIEW_CHILD_REPORT="$child_report" \
  "$app/Contents/MacOS/hmi-studio" --native-preview-smoke-report "$report"
python3 - "$report" "$child_report" <<'PY'
import json
import sys

path, child_path = sys.argv[1:]
with open(path, encoding="utf-8") as stream:
    result = json.load(stream)
required = {
    "success": True,
    "studioPid": 0,
    "previewPid": 1,
    "mainVisible": True,
    "logicVisible": True,
    "previewVisible": True,
    "previewScreenMounted": True,
    "duplicateOpenSamePid": True,
    "simulatedConnectionOpened": True,
    "snapshotTemperature": 23.6,
    "temperatureRendered": True,
    "resourceUpdateWaited": True,
    "pendingCandidateReplaced": True,
    "oldRunCompletedOnRevision": 1,
    "newRunCompletedOnRevision": 2,
    "newRevisionApplied": 4,
    "resourceConnectionStillOpen": True,
    "hotUpdatePidUnchanged": True,
    "invalidCandidateRejected": True,
    "ipcDisconnectChildExited": True,
    "sessionStoppedAfterMainClose": True,
    "previewProcessStoppedAfterMainClose": True,
    "windowsRemainingAfterMainClose": [],
}
for key, expected in required.items():
    actual = result.get(key)
    if key == "studioPid":
        if not isinstance(actual, int) or actual <= 0:
            raise SystemExit(f"Native preview verification failed for {key}: {actual!r}")
    elif key == "previewPid":
        if not isinstance(actual, int) or actual <= 0 or actual == result.get("studioPid"):
            raise SystemExit(f"Native preview verification failed for {key}: {actual!r}")
    elif actual != expected:
        raise SystemExit(f"Native preview verification failed for {key}: {result.get(key)!r}")
print(json.dumps(result, ensure_ascii=False, indent=2))
print(f"Native preview-window report: {path}")
with open(child_path, encoding='utf-8') as stream:
    child = json.load(stream)
if child.get("success") is not True or child.get("temperature") != 23.6 or child.get("temperatureRendered") is not True:
    raise SystemExit(f"Native Runner child verification failed: {child!r}")
print(json.dumps(child, ensure_ascii=False, indent=2))
print(f"Native Runner child report: {child_path}")
PY
