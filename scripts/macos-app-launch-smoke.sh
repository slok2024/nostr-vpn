#!/usr/bin/env bash
# Build or launch a macOS app bundle and verify it survives startup.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ARTIFACT_ROOT="${ARTIFACT_ROOT:-$ROOT/artifacts}"
APP_PATH="${NVPN_MACOS_APP_PATH:-${1:-}}"
STARTUP_TIMEOUT_SECONDS="${NVPN_MACOS_APP_SMOKE_STARTUP_TIMEOUT_SECONDS:-30}"
ALIVE_SECONDS="${NVPN_MACOS_APP_SMOKE_ALIVE_SECONDS:-5}"
RESULT_PATH="$ARTIFACT_ROOT/macos-app-launch-smoke.json"
LOG_PATH="$ARTIFACT_ROOT/macos-app-launch-smoke.log"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "macOS app launch smoke requires macOS." >&2
  exit 1
fi

mkdir -p "$ARTIFACT_ROOT"

if [[ -z "$APP_PATH" ]]; then
  case "${NVPN_MACOS_APP_SMOKE_BUILD:-1}" in
    0|false|FALSE|False|no|NO|No|off|OFF|Off)
      echo "Set NVPN_MACOS_APP_PATH or pass a .app path when NVPN_MACOS_APP_SMOKE_BUILD=0." >&2
      exit 2
      ;;
    *)
      "$ROOT/scripts/macos-build" macos-build
      APP_PATH="$("$ROOT/scripts/build-output-path" --raw)"
      ;;
  esac
fi

if [[ ! -d "$APP_PATH" ]]; then
  echo "macOS app bundle not found: $APP_PATH" >&2
  exit 1
fi

executable="$(
  /usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$APP_PATH/Contents/Info.plist" 2>/dev/null \
    || basename "$APP_PATH" .app
)"

app_pids() {
  pgrep -x "$executable" 2>/dev/null || pgrep -f "$APP_PATH/Contents/MacOS/$executable" 2>/dev/null || true
}

write_result() {
  local ok="$1"
  local error="${2:-}"
  local pid="${3:-}"
  cat >"$RESULT_PATH" <<JSON
{
  "ok": $ok,
  "appPath": "$APP_PATH",
  "executable": "$executable",
  "processId": "$pid",
  "error": "$error",
  "generatedAt": "$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
}
JSON
}

capture_logs() {
  log show --style compact --last 2m --predicate "process == \"$executable\"" >"$LOG_PATH" 2>&1 || true
}

if pids="$(app_pids)" && [[ -n "$pids" ]]; then
  kill $pids >/dev/null 2>&1 || true
  sleep 1
fi

touch "$APP_PATH"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister \
  -f -R -trusted "$APP_PATH" >/dev/null 2>&1 || true

open -n "$APP_PATH"

pid=""
deadline=$((SECONDS + STARTUP_TIMEOUT_SECONDS))
while (( SECONDS < deadline )); do
  pid="$(app_pids | head -n 1 || true)"
  if [[ -n "$pid" ]]; then
    break
  fi
  sleep 0.5
done

if [[ -z "$pid" ]]; then
  capture_logs
  write_result false "macOS app did not create a process within ${STARTUP_TIMEOUT_SECONDS}s"
  echo "macOS app launch smoke failed: no process appeared for $APP_PATH" >&2
  tail -n 80 "$LOG_PATH" >&2 || true
  exit 1
fi

alive_until=$((SECONDS + ALIVE_SECONDS))
while (( SECONDS < alive_until )); do
  if ! kill -0 "$pid" >/dev/null 2>&1; then
    capture_logs
    write_result false "macOS app exited during startup" "$pid"
    echo "macOS app launch smoke failed: process $pid exited during startup." >&2
    tail -n 80 "$LOG_PATH" >&2 || true
    exit 1
  fi
  sleep 0.5
done

write_result true "" "$pid"
kill "$pid" >/dev/null 2>&1 || true
echo "MACOS_APP_LAUNCH_SMOKE_OK"
echo "Result: $RESULT_PATH"
