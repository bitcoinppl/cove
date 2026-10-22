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

usage() { # usage MESSAGE: exit 2 like the android-* commands, before any side effect
  json ok:raw=false error=usage "message=$1"
  exit 2
}

# the first adb call can start the host adb daemon, which would inherit the device lock on fd 9
# and hold it long after this script exits. The redirect goes on the binary's path: on a builtin
# such as `command`, bash 3.2 applies it in the shell and leaves a copy of fd 9 that adb inherits
UIA2_ADB="$(type -P adb)"
adb() {
  [ -n "$UIA2_ADB" ] || { echo "adb not found" >&2; return 127; }
  "$UIA2_ADB" "$@" 9>&-
}

require_adb() {
  [ -n "$UIA2_ADB" ] || die missing_adb "adb not found" "fix=add the Android SDK platform-tools to PATH"
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

# print "SERIAL DESTINATION" for the adb forward on localhost PORT; fields compare literally,
# since a serial such as [::1]:5555 is not a safe regex
forward_entry() { # forward_entry FORWARD_LIST PORT
  awk -v p="tcp:$2" '$2 == p { print $1, $3; exit }' <<<"$1"
}

# print the local ports forwarded to this device's server, one per line
server_ports() { # server_ports FORWARD_LIST
  awk -v s="$ANDROID_SERIAL" -v d="tcp:$UIA2_DEVICE_PORT" '$1 == s && $3 == d { sub(/^tcp:/, "", $2); print $2 }' <<<"$1"
}

# start time of a process, so a pid reused by another process is not mistaken for ours
proc_start() {
  ps -p "$1" -o lstart= 2>/dev/null | sed 's/^ *//; s/ *$//'
}

# hold a per-device lock so concurrent starts and stops can't interleave their checks,
# state files and forwards; a uia2-stop run by uia2-start inherits its parent's lock.
# macOS has no flock(1), so a helper locks fd 9: the lock belongs to that open file, outlives
# the helper, and the OS releases it when this script exits, even when it is killed
lock_device() {
  [ -n "${UIA2_LOCKED:-}" ] && return 0
  local lock
  lock="$(state_dir).flock"
  mkdir -p "$(dirname "$lock")"
  exec 9>>"$lock"
  python3 -I -c 'import fcntl, sys, time
deadline = time.monotonic() + float(sys.argv[1])
while True:
    try:
        fcntl.flock(9, fcntl.LOCK_EX | fcntl.LOCK_NB)
        sys.exit(0)
    except BlockingIOError:
        if time.monotonic() >= deadline:
            sys.exit(1)
        time.sleep(0.1)' "${UIA2_LOCK_WAIT:-30}" ||
    die locked "another uia2-start or uia2-stop for $ANDROID_SERIAL held $lock for ${UIA2_LOCK_WAIT:-30} s" "fix=wait for it to finish"
  export UIA2_LOCKED=1
}
