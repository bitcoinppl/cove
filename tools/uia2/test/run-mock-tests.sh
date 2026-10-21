#!/bin/bash
# exercise the uia2-* and android-* scripts against test/mock-uia2.py and test/fake-adb; no device needed
# usage: test/run-mock-tests.sh   (exits non-zero when any assertion fails)
set -u
here="$(cd "$(dirname "$0")/.." && pwd)"
tmp="$(mktemp -d)"
port=$((20000 + RANDOM % 20000))
python3 -I "$here/test/mock-uia2.py" "$port" "$tmp/requests.log" &
srv=$!
trap 'kill $srv 2>/dev/null; wait $srv 2>/dev/null; rm -rf "$tmp"' EXIT

# the fake adb shadows the real one, so nothing here can reach an attached device
mkdir -p "$tmp/bin" "$tmp/adb"
ln -s "$here/test/fake-adb" "$tmp/bin/adb"
export PATH="$tmp/bin:$PATH" FAKE_ADB="$tmp/adb"
export UIA2_PORT="$port" UIA2_STATE="$tmp/state" UIA2_CACHE="$tmp/cache" ANDROID_SERIAL=TEST-SERIAL
unset UIA2_BASE_URL
for _ in $(seq 50); do curl -sf "http://127.0.0.1:$port/status" >/dev/null && break; sleep 0.1; done

fail=0
# expect NAME EXIT_CODE PYTHON_EXPR_OVER_d -- COMMAND...
expect() {
  local name="$1" code="$2" expr="$3"; shift 4
  local out rc
  out="$("$@" 2>&1)"; rc=$?
  if [ $rc -ne "$code" ] || ! printf '%s' "$out" | python3 -I -c "import json,sys; d=json.load(sys.stdin); sys.exit(0 if ($expr) else 1)"; then
    echo "FAIL $name (rc=$rc): ${out:0:400}"; fail=1
  else
    echo "ok   $name"
  fi
}

# check NAME COMMAND...: pass when the command succeeds
check() {
  local name="$1"; shift
  if "$@" >/dev/null 2>&1; then echo "ok   $name"; else echo "FAIL $name"; fail=1; fi
}

# sent PATH_SUFFIX BODY_JSON: the mock received exactly this request body on a path ending in PATH_SUFFIX
sent() {
  grep -qF "\"/session/mock-session$1\", \"body\": $2}" "$tmp/requests.log"
}

vis() { "$here/android-vis"; }

# server commands
expect "swipe up" 0 'd["direction"]=="up" and d["from"]==[540,1800] and d["to"]==[540,600]' -- "$here/android-swipe" up
expect "swipe bad dir" 2 'd["error"]=="usage"' -- "$here/android-swipe" sideways
expect "swipe dry-run" 0 'd["dry_run"] and "would_send" in d' -- "$here/android-swipe" --dry-run left
expect "swipe hold" 0 'd["hold"]==0.3' -- "$here/android-swipe" --hold 0.3 up
check "swipe hold sent as a pause" grep -q '"pause", "duration": 300}, {"type": "pointerMove", "duration": 300, "x": 540, "y": 600}' "$tmp/requests.log"
expect "swipe hold bad" 2 'd["error"]=="usage"' -- "$here/android-swipe" --hold x up
expect "tap" 0 'd["x"]==10 and d["y"]==20' -- "$here/android-tap" 10 20.5
check "tap request" sent /appium/gestures/click '{"offset": {"x": 10, "y": 20}}'
expect "tap bad args" 2 'd["error"]=="usage"' -- "$here/android-tap" ten 20
expect "hold" 0 'd["seconds"]==1.0 and d["x"]==5' -- "$here/android-hold" 5 6
check "hold request" sent /appium/gestures/long_click '{"offset": {"x": 5, "y": 6}, "duration": 1000}'
expect "hold bad args" 2 'd["error"]=="usage"' -- "$here/android-hold" 5
expect "drag" 0 'd["hold"]==1.0 and d["to"]==[3,4]' -- "$here/android-drag" 1 2 3 4
check "drag request" grep -qF '"pause", "duration": 1000}, {"type": "pointerMove", "duration": 500, "x": 3, "y": 4}' "$tmp/requests.log"
expect "drag bad args" 2 'd["error"]=="usage"' -- "$here/android-drag" 1 2 3
expect "tap-label text" 0 'd["matched_by"]=="text"' -- "$here/android-tap-label" About
check "tap-label clicks the match" sent /element/el-1/click '{}'
expect "tap-label desc" 0 'd["matched_by"]=="desc"' -- "$here/android-tap-label" Back
expect "tap-label tag" 0 'd["matched_by"]=="tag"' -- "$here/android-tap-label" numberPadPin.digit.1
expect "tap-label nth" 0 'd["nth"]==2' -- "$here/android-tap-label" --nth 2 Send
check "tap-label nth clicks the second match" sent /element/el-2/click '{}'
expect "tap-label missing" 1 'd["error"]=="label_not_found" and d["matches"]=={"tag":0,"text":0,"desc":0}' -- "$here/android-tap-label" --wait 0 Nope
expect "tap-label nth too high" 1 'd["error"]=="label_not_found"' -- "$here/android-tap-label" --wait 0 --nth 2 About
expect "type readback" 0 'd["typed_chars"]==5 and d["readback_matches"] is True' -- "$here/android-type" hello
check "type replaces the field" sent /keys '{"text": "hello", "replace": true}'
expect "type mismatch" 0 'd["readback_matches"] is False' -- "$here/android-type" other
: >"$tmp/requests.log"
expect "type enter" 0 'd["enter"] and d["readback_matches"] is True' -- "$here/android-type" --enter hello
check "type enter sends the text without a newline" sent /keys '{"text": "hello", "replace": true}'
check "type enter presses Enter" sent /appium/device/press_keycode '{"keycode": 66}'
expect "type no args" 2 'd["error"]=="usage"' -- "$here/android-type"
expect "back" 0 'd["ok"]' -- "$here/android-back"
check "back request" sent /back '{}'
expect "key name" 0 'd["keycode"]==66' -- "$here/android-key" KEYCODE_ENTER
expect "key number" 0 'd["keycode"]==279' -- "$here/android-key" 279
check "key request" sent /appium/device/press_keycode '{"keycode": 279}'
expect "key bad" 2 'd["error"]=="usage"' -- "$here/android-key" nope
expect "source" 0 '"numberPadPin" in d["source"]' -- "$here/android-source"
: >"$tmp/requests.log"
expect "pin" 0 'd["typed_digits"]==3 and d["matched_by"]=="tag"' -- "$here/android-pin" 121
check "pin taps digit 1 at its center" sent /appium/gestures/click '{"offset": {"x": 204, "y": 1465}}'
check "pin taps digit 2 at its center" sent /appium/gestures/click '{"offset": {"x": 540, "y": 1465}}'
[ "$(grep -c gestures/click "$tmp/requests.log")" = 3 ] && echo "ok   pin taps once per digit" || { echo "FAIL pin tap count"; fail=1; }
expect "pin missing digit" 1 'd["error"]=="keypad_not_visible"' -- "$here/android-pin" 13
expect "shot" 0 'd["bytes"]>0 and d["protected"] is False' -- "$here/android-shot" "$tmp/out/shot.png"
check "shot file is a PNG" grep -q PNG "$tmp/out/shot.png"
expect "shot scaled" 0 'd["max_pixels"]==1' -- "$here/android-shot" -s 1 "$tmp/out/small.png"
expect "shot protected" 0 'd["protected"] is True and d["path"] is None' -- "$here/android-shot" --element About "$tmp/out/secret.png"
check "shot protected writes no file" test ! -e "$tmp/out/secret.png"
expect "shot no args" 2 'd["error"]=="usage"' -- "$here/android-shot"
expect "clipboard set" 0 'd["chars"]==4' -- "$here/android-clipboard" set "a #b"
check "clipboard set body is base64" sent /appium/device/set_clipboard '{"content": "YSAjYg==", "contentType": "plaintext"}'
expect "clipboard get blocked" 1 'd["error"]=="clipboard_read_blocked"' -- "$here/android-clipboard" get
expect "clipboard bad op" 2 'd["error"]=="usage"' -- "$here/android-clipboard" clear
UIA2_PORT=1 expect "unreachable" 1 'd["error"]=="uia2_unreachable"' -- "$here/android-source"
UIA2_WAIT=x expect "bad env gives JSON" 1 'd["error"]=="internal"' -- "$here/android-tap-label" About

# android-vis
check "source --raw prints xml" bash -c "'$here/android-source' --raw | grep -q 'resource-id=\"numberPadPin.viewport\"'"
check "vis flags covered elements" bash -c "'$here/android-vis' | grep -qE '^# U: [0-9]+ elements .*numberPadPin.viewport'"
check "vis names rows by their text" bash -c "'$here/android-vis' | grep -qF \"CU  contains='Sent | -209 SATS'\""
check "vis names switches by their row" bash -c "'$here/android-vis' | grep -qE \"CXU +row='Enable PIN'\""
check "vis does not put a layer's ancestors under it" bash -c "'$here/android-vis' | grep -qE \"^FrameLayout .* {5}tag='screen.root'\""
check "vis prints tags" bash -c "'$here/android-vis' | grep -qF \"tag='numberPadPin.digit.1'\""
if vis | grep -q "text='-209 SATS'"; then echo "FAIL vis repeats row text"; fail=1; else echo "ok   vis does not repeat row text"; fi
( "$here/android-vis" 2>"$tmp/pipe.err" ) | head -1 >/dev/null
[ -s "$tmp/pipe.err" ] && { echo "FAIL vis into a closed pipe: $(head -c 300 "$tmp/pipe.err")"; fail=1; } || echo "ok   vis into a closed pipe"

# a plain screen with no layer over it, a hidden full-screen node and an Android view id
cat >"$tmp/requests.log.source" <<'XML'
<hierarchy width="1080" height="2400">
  <android.widget.TextView class="android.widget.TextView" resource-id="" text="Settings" content-desc="" clickable="false" displayed="true" bounds="[60,100][500,160]"/>
  <android.widget.Button class="android.widget.Button" resource-id="com.android.permissioncontroller:id/permission_allow_button" text="Allow" content-desc="" clickable="true" displayed="true" bounds="[60,300][1020,400]"/>
  <android.view.View class="android.view.View" resource-id="sheet.scrim" text="" content-desc="" clickable="false" displayed="false" bounds="[0,0][1080,2400]"/>
</hierarchy>
XML
if vis | grep -qE '^# U|U  '; then echo "FAIL vis flags a screen with no visible layer"; fail=1; else echo "ok   vis has no U flags without a visible layer"; fi
check "vis prints a view id in full" bash -c "'$here/android-vis' | grep -qF \"tag='com.android.permissioncontroller:id/permission_allow_button'\""
rm "$tmp/requests.log.source"

# android-launch through the fake adb
expect "launch" 0 'd["action"]=="launch" and d["activity"]=="org.bitcoinppl.cove/.MainActivity"' -- "$here/android-launch" org.bitcoinppl.cove
check "launch force-stops first" grep -q "shell am force-stop org.bitcoinppl.cove" "$FAKE_ADB/calls.log"
: >"$FAKE_ADB/calls.log"
expect "activate" 0 'd["action"]=="activate"' -- "$here/android-launch" --activate org.bitcoinppl.cove
check "activate does not force-stop" bash -c "! grep -q force-stop '$FAKE_ADB/calls.log'"
expect "launch misspelled flag" 2 'd["error"]=="usage"' -- "$here/android-launch" --activte org.bitcoinppl.cove
touch "$FAKE_ADB/resolve_fail"
expect "launch adb failure" 1 'd["error"]=="adb_failed" and "more than one device" in d["message"]' -- "$here/android-launch" org.bitcoinppl.cove
rm "$FAKE_ADB/resolve_fail"

# uia2-start and uia2-stop through the fake adb; the mock answers /status like a started server
expect "start dry-run" 0 'd["dry_run"] and "am instrument" in d["instrument_command"]' -- "$here/uia2-start" --dry-run
echo "OTHER-SERIAL tcp:$port tcp:6790" >"$FAKE_ADB/forwards"
expect "start port taken" 1 'd["error"]=="port_taken"' -- "$here/uia2-start"
echo "TEST-SERIAL tcp:9999 tcp:6790" >"$FAKE_ADB/forwards"
expect "start with this device's server on another port" 1 'd["error"]=="device_busy" and "9999" in d["message"]' -- "$here/uia2-start"
: >"$FAKE_ADB/forwards"
echo "com.android.commands.am.Am instrument -w org.bitcoinppl.cove.test/androidx.test.runner.AndroidJUnitRunner" >"$FAKE_ADB/ps"
expect "start while instrumented tests run" 1 'd["error"]=="device_busy" and "am.Am instrument" in d["running"]' -- "$here/uia2-start"
: >"$FAKE_ADB/ps"

mkdir -p "$tmp/cache/apk/v10.6.6"
echo server >"$tmp/cache/apk/v10.6.6/appium-uiautomator2-server-v10.6.6.apk"
echo test >"$tmp/cache/apk/v10.6.6/appium-uiautomator2-server-debug-androidTest.apk"
export UIA2_SERVER_SHA256="$(shasum -a 256 "$tmp/cache/apk/v10.6.6/appium-uiautomator2-server-v10.6.6.apk" | cut -d' ' -f1)"
export UIA2_TEST_SHA256="$(shasum -a 256 "$tmp/cache/apk/v10.6.6/appium-uiautomator2-server-debug-androidTest.apk" | cut -d' ' -f1)"

touch "$FAKE_ADB/forward_fail"
expect "start forward failure" 1 'd["error"]=="forward_failed"' -- "$here/uia2-start"
pid="$(cat "$tmp/state/TEST-SERIAL/instrument.pid" 2>/dev/null)"
check "failed start stops the server" bash -c "[ -z '$pid' ] && grep -q 'am force-stop io.appium.uiautomator2.server' '$FAKE_ADB/calls.log'"
rm "$FAKE_ADB/forward_fail"

expect "start" 0 'd["ok"] and d["serial"]=="TEST-SERIAL"' -- "$here/uia2-start"
pid="$(cat "$tmp/state/TEST-SERIAL/instrument.pid")"
check "start forwards the port" grep -q "^TEST-SERIAL tcp:$port tcp:6790$" "$FAKE_ADB/forwards"
check "start runs am instrument" kill -0 "$pid"
expect "start again" 0 'd["already_running"]' -- "$here/uia2-start"
expect "stop" 0 '"adb:" in d["stopped"] and "forward:" in d["stopped"]' -- "$here/uia2-stop"
sleep 0.2
check "stop kills am instrument" bash -c "! kill -0 $pid 2>/dev/null"
check "stop removes the forward" bash -c "! grep -q TEST-SERIAL '$FAKE_ADB/forwards'"
exit $fail
