# shared config for the uia2-* scripts; sourced, not executed
UIA2_HOME="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UIA2_SERVER_VERSION="${UIA2_SERVER_VERSION:-10.6.6}"
UIA2_SERVER_SHA256="${UIA2_SERVER_SHA256:-8ff760a2a86b487f53090fbdcd5b0360e67d02bb811887d527a9557b0d59c80d}"
UIA2_TEST_SHA256="${UIA2_TEST_SHA256:-e6f729287d72351388fd66597c35b69988b9091a7011954bf92a55d416cb1e1c}"
UIA2_SERVER_PKG=io.appium.uiautomator2.server
UIA2_RUNNER="$UIA2_SERVER_PKG.test/androidx.test.runner.AndroidJUnitRunner"
UIA2_DEVICE_PORT=6790
UIA2_PORT="${UIA2_PORT:-8200}"
UIA2_BASE_URL="${UIA2_BASE_URL:-http://127.0.0.1:$UIA2_PORT}"

# shared across worktrees: the APKs are 18 MB and one server per device is global anyway
UIA2_CACHE="${UIA2_CACHE:-$HOME/.cache/cove-uia2}"
UIA2_APK_DIR="$UIA2_CACHE/apk/v$UIA2_SERVER_VERSION"
UIA2_SERVER_APK="$UIA2_APK_DIR/appium-uiautomator2-server-v$UIA2_SERVER_VERSION.apk"
UIA2_TEST_APK="$UIA2_APK_DIR/appium-uiautomator2-server-debug-androidTest.apk"

# print a JSON object built from key=value pairs; values are strings unless the key ends in :raw
json() {
  python3 -I - "$@" <<'PY'
import json, sys
out = {}
for a in sys.argv[1:]:
    k, _, v = a.partition("=")
    if k.endswith(":raw"):
        out[k[:-4]] = json.loads(v)
    else:
        out[k] = v
print(json.dumps(out))
PY
}

die() { # die ERROR_CODE MESSAGE [extra key=value...]
  local code="$1" msg="$2"; shift 2
  json ok:raw=false "error=$code" "message=$msg" "$@"
  exit 1
}

# pick the device: ANDROID_SERIAL when set (adb honors it too), else the only attached device
resolve_serial() {
  if [ -n "${ANDROID_SERIAL:-}" ]; then
    return 0
  fi

  local devices
  devices="$(adb devices | awk 'NR > 1 && $2 == "device" { print $1 }')"
  case "$(printf '%s\n' "$devices" | grep -c .)" in
    0) die no_device "no authorized device in adb devices" "fix=plug in the phone, allow USB debugging, run uia2-doctor" ;;
    1) export ANDROID_SERIAL="$devices" ;;
    *) die several_devices "more than one device attached; set ANDROID_SERIAL" "devices=$(echo $devices)" ;;
  esac
}

state_dir() {
  echo "${UIA2_STATE:-$UIA2_CACHE/state}/$ANDROID_SERIAL"
}
