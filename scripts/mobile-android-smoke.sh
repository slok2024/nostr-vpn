#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PACKAGE_NAME="${NVPN_ANDROID_PACKAGE:-org.nostrvpn.app}"
MAIN_ACTIVITY="${NVPN_ANDROID_ACTIVITY:-org.nostrvpn.app/.MainActivity}"
DEBUG_ACTION_EXTRA="${NVPN_ANDROID_DEBUG_ACTION_EXTRA:-org.nostrvpn.app.DEBUG_ACTION}"
APK_PATH="${NVPN_ANDROID_APK:-$ROOT/android/app/build/outputs/apk/debug/app-debug.apk}"

build=1
clear_state=0
vpn_cycle=0
serial="${NVPN_ANDROID_SERIAL:-${ANDROID_SERIAL:-}}"

usage() {
  cat >&2 <<'EOF'
usage: scripts/mobile-android-smoke.sh [--no-build] [--clear] [--vpn-cycle] [--serial SERIAL]

Builds and installs the debug APK, launches the app through adb, and optionally
cycles the debug VPN action. Pass device identifiers through env or CLI only;
do not commit them.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-build)
      build=0
      ;;
    --clear)
      clear_state=1
      ;;
    --vpn-cycle)
      vpn_cycle=1
      ;;
    --serial)
      if [[ $# -lt 2 ]]; then
        echo "--serial requires a value" >&2
        exit 2
      fi
      serial="$2"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage
      exit 2
      ;;
  esac
  shift
done

sdk_from_local_properties() {
  local file="$ROOT/android/local.properties"
  if [[ -f "$file" ]]; then
    sed -n 's/^sdk\.dir=//p' "$file" | head -n 1
  fi
}

resolve_adb() {
  local sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
  if [[ -z "$sdk" ]]; then
    sdk="$(sdk_from_local_properties)"
  fi
  if [[ -z "$sdk" && -d "$HOME/Library/Android/sdk" ]]; then
    sdk="$HOME/Library/Android/sdk"
  fi
  if [[ -n "$sdk" && -x "$sdk/platform-tools/adb" ]]; then
    printf '%s\n' "$sdk/platform-tools/adb"
    return
  fi
  if command -v adb >/dev/null 2>&1; then
    command -v adb
    return
  fi
  echo "adb not found; set ANDROID_HOME/ANDROID_SDK_ROOT or add adb to PATH" >&2
  exit 1
}

select_serial() {
  local adb="$1"
  if [[ -n "$serial" ]]; then
    printf '%s\n' "$serial"
    return
  fi
  "$adb" devices | awk 'NR > 1 && $2 == "device" { print $1; exit }'
}

ADB="$(resolve_adb)"

if [[ "$build" -eq 1 ]]; then
  "$ROOT/tools/run-android" build
fi

if [[ ! -f "$APK_PATH" ]]; then
  echo "Debug APK not found at $APK_PATH; run just android-build first" >&2
  exit 1
fi

serial="$(select_serial "$ADB")"
if [[ -z "$serial" ]]; then
  echo "No online Android device or emulator found; set NVPN_ANDROID_SERIAL or start an emulator" >&2
  exit 1
fi

"$ADB" -s "$serial" wait-for-device
"$ADB" -s "$serial" install -r "$APK_PATH"

if [[ "$clear_state" -eq 1 ]]; then
  "$ADB" -s "$serial" shell pm clear "$PACKAGE_NAME" >/dev/null
fi

"$ADB" -s "$serial" shell am start -n "$MAIN_ACTIVITY" >/dev/null
"$ADB" -s "$serial" shell pm path "$PACKAGE_NAME" >/dev/null

if [[ "$vpn_cycle" -eq 1 ]]; then
  "$ADB" -s "$serial" shell am start -n "$MAIN_ACTIVITY" --es "$DEBUG_ACTION_EXTRA" disconnect >/dev/null
  sleep 2
  "$ADB" -s "$serial" shell am start -n "$MAIN_ACTIVITY" --es "$DEBUG_ACTION_EXTRA" connect >/dev/null
fi

echo "Android smoke passed on adb serial: $serial"
