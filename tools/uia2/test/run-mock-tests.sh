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

# clicks: print the x,y of each gestures/click request in order
clicks() {
  python3 -I -c 'import json,sys
for l in open(sys.argv[1]):
    r = json.loads(l)
    if r["path"].endswith("/appium/gestures/click"):
        print("%d,%d" % (r["body"]["offset"]["x"], r["body"]["offset"]["y"]))' "$tmp/requests.log"
}

# requests: print the paths of the requests sent since the log was last cleared
requests() {
  python3 -I -c 'import json,sys
for l in open(sys.argv[1]):
    print(json.loads(l)["path"].removeprefix("/session/mock-session"))' "$tmp/requests.log"
}

# screen FILE_CONTENT: serve this XML as the screen until the next call to screen with no argument
screen() {
  if [ $# -eq 0 ]; then rm -f "$tmp/requests.log.source"; else printf '%s' "$1" >"$tmp/requests.log.source"; fi
}

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
expect "type mismatch" 0 'd["readback_matches"] is False' -- "$here/android-type" trunc-secret
out="$("$here/android-type" FAILsecret 2>&1)"
expect "type failure" 1 'd["error"]=="uia2_invalid_element_state"' -- "$here/android-type" FAILsecret
[[ "$out" != *FAILsecret* ]] && echo "ok   type failure keeps the text out of the output" || { echo "FAIL type failure echoes the text: $out"; fail=1; }
: >"$tmp/requests.log"
expect "type enter" 0 'd["enter"] and d["readback_matches"] is True' -- "$here/android-type" --enter hello
check "type enter sends the text without a newline" sent /keys '{"text": "hello", "replace": true}'
check "type enter presses Enter" sent /appium/device/press_keycode '{"keycode": 66}'
[ "$(requests | grep -E '^/keys$|press_keycode' | tr '\n' ' ')" = "/keys /appium/device/press_keycode " ] &&
  echo "ok   type enter presses Enter after setting the text" || { echo "FAIL type enter order: $(requests | tr '\n' ' ')"; fail=1; }
expect "type no args" 2 'd["error"]=="usage"' -- "$here/android-type"
expect "back" 0 'd["ok"]' -- "$here/android-back"
check "back request" sent /back '{}'
expect "key name" 0 'd["keycode"]==66' -- "$here/android-key" KEYCODE_ENTER
expect "key number" 0 'd["keycode"]==279' -- "$here/android-key" 279
check "key request" sent /appium/device/press_keycode '{"keycode": 279}'
expect "key bad" 2 'd["error"]=="usage"' -- "$here/android-key" nope
expect "source" 0 '"numberPadPin" in d["source"]' -- "$here/android-source"
: >"$tmp/requests.log"
expect "pin" 0 'd["typed_digits"]==4' -- "$here/android-pin" 2112
[ "$(clicks | tr '\n' ' ')" = "540,1465 204,1465 204,1465 540,1465 " ] &&
  echo "ok   pin taps each digit's center in order" || { echo "FAIL pin taps: $(clicks | tr '\n' ' ')"; fail=1; }
expect "pin missing digit" 1 'd["error"]=="keypad_not_visible"' -- "$here/android-pin" 13
screen '<hierarchy width="1080" height="2400"><android.widget.TextView class="android.widget.TextView" resource-id="" text="1" content-desc="" displayed="true" bounds="[60,100][120,160]"/></hierarchy>'
expect "pin ignores digits outside the keypad" 1 'd["error"]=="keypad_not_visible"' -- "$here/android-pin" 1111
screen
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
screen

# summaries must not hide controls, cut text or borrow labels from hidden or unrelated nodes
long="$(printf 'word%.0s ' $(seq 60))"
screen "<hierarchy width=\"1080\" height=\"2400\">
  <android.view.View class=\"android.view.View\" resource-id=\"\" text=\"\" content-desc=\"\" clickable=\"true\" displayed=\"true\" bounds=\"[0,100][1080,300]\">
    <android.widget.TextView class=\"android.widget.TextView\" resource-id=\"\" text=\"Hidden note\" content-desc=\"\" displayed=\"false\" bounds=\"[0,200][900,300]\"/>
    <android.widget.TextView class=\"android.widget.TextView\" resource-id=\"\" text=\"$long\" content-desc=\"\" displayed=\"true\" bounds=\"[0,100][900,150]\"/>
    <android.widget.TextView class=\"android.widget.TextView\" resource-id=\"\" text=\"Tail words\" content-desc=\"\" displayed=\"true\" bounds=\"[0,150][900,200]\"/>
    <android.widget.EditText class=\"android.widget.EditText\" resource-id=\"\" text=\"Amount\" content-desc=\"\" displayed=\"true\" bounds=\"[0,200][900,300]\"/>
  </android.view.View>
  <android.view.View class=\"android.view.View\" resource-id=\"\" text=\"\" content-desc=\"\" displayed=\"true\" bounds=\"[0,400][1080,500]\">
    <android.widget.TextView class=\"android.widget.TextView\" resource-id=\"\" text=\"Enable PIN\" content-desc=\"\" displayed=\"true\" bounds=\"[60,420][400,480]\"/>
    <android.view.View class=\"android.view.View\" resource-id=\"\" text=\"\" content-desc=\"\" clickable=\"true\" displayed=\"true\" bounds=\"[880,400][1020,500]\"/>
    <android.widget.TextView class=\"android.widget.TextView\" resource-id=\"\" text=\"Delete wallet\" content-desc=\"\" displayed=\"false\" bounds=\"[700,420][860,480]\"/>
  </android.view.View>
  <android.widget.TextView class=\"android.widget.TextView\" resource-id=\"\" text=\"Other row\" content-desc=\"\" displayed=\"true\" bounds=\"[860,420][879,480]\"/>
</hierarchy>"
check "vis keeps an editable child of a row" bash -c "'$here/android-vis' | grep -qE \"^EditText .* E .*text='Amount'\""
check "vis keeps text cut from a long summary" bash -c "'$here/android-vis' | grep -qF \"text='Tail words'\""
if vis | grep -q "Hidden note"; then echo "FAIL vis summary includes a hidden child"; fail=1; else echo "ok   vis summary skips hidden children"; fi
check "vis names a switch from its own row" bash -c "'$here/android-vis' | grep -qF \"row='Enable PIN'\""
screen

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

# uia2-doctor through the fake adb
expect "doctor" 0 'd["ok"] and {c["name"]: c["ok"] for c in d["checks"]}["screen_unlocked"]' -- "$here/uia2-doctor"
touch "$FAKE_ADB/query_fail"
expect "doctor with failing device queries" 1 'not d["ok"] and not any(c["ok"] for c in d["checks"] if c["name"] in ("screen_unlocked", "no_other_instrumentation"))' -- "$here/uia2-doctor"
expect "start with failing device queries" 1 'd["error"]=="device_query_failed"' -- "$here/uia2-start"
rm "$FAKE_ADB/query_fail"

# uia2-start and uia2-stop through the fake adb; the mock answers /status like a started server
expect "start dry-run" 0 'd["dry_run"] and "am instrument" in d["instrument_command"]' -- "$here/uia2-start" --dry-run
expect "start bad flag" 2 'd["error"]=="usage"' -- "$here/uia2-start" --forse
: >"$FAKE_ADB/calls.log"
expect "stop bad flag" 2 'd["error"]=="usage"' -- "$here/uia2-stop" --dr-run
check "stop bad flag does nothing" bash -c "! grep -q force-stop '$FAKE_ADB/calls.log'"
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
pid="$(cut -d' ' -f1 "$tmp/state/TEST-SERIAL/instrument.pid")"
check "start forwards the port" grep -q "^TEST-SERIAL tcp:$port tcp:6790$" "$FAKE_ADB/forwards"
check "start runs am instrument" kill -0 "$pid"
expect "start again" 0 'd["already_running"]' -- "$here/uia2-start"
touch "$FAKE_ADB/force_stop_fail"
expect "stop with a failing device" 1 'd["error"]=="cleanup_incomplete" and "force-stop" in d["failed"] and "adb:" in d["stopped"]' -- "$here/uia2-stop"
check "failed stop keeps its state for a retry" test -s "$tmp/state/TEST-SERIAL/port"
rm "$FAKE_ADB/force_stop_fail"
expect "stop after a failed stop" 0 'd["stopped"]=="io.appium.uiautomator2.server"' -- "$here/uia2-stop"
sleep 0.2
check "stop kills am instrument" bash -c "! kill -0 $pid 2>/dev/null"
check "stop removes the forward" bash -c "! grep -q TEST-SERIAL '$FAKE_ADB/forwards'"

# a pid file whose process was replaced must not get that process killed, even when the new
# process looks like this device's adb
bash -c 'exec -a "adb -s TEST-SERIAL shell am instrument" sleep 30' &
other=$!
sleep 0.2
echo "$other Mon Jan  1 00:00:00 2024" >"$tmp/state/TEST-SERIAL/instrument.pid"
expect "stop with a reused pid" 0 '"adb:" not in d["stopped"]' -- "$here/uia2-stop"
check "stop leaves a reused pid alone" kill -0 "$other"
kill "$other"; wait "$other" 2>/dev/null

# a lock left by a dead process does not block the next start
mkdir -p "$tmp/state/TEST-SERIAL.lock"; echo 999999 >"$tmp/state/TEST-SERIAL.lock/pid"
expect "start with a stale lock" 0 'd["ok"]' -- "$here/uia2-start"
"$here/uia2-stop" >/dev/null

# an interrupted start stops the server it launched; this port has no server, so start keeps waiting
dead=$((port + 1))
UIA2_PORT=$dead UIA2_START_TIMEOUT=20 "$here/uia2-start" >"$tmp/interrupt.json" 2>&1 &
starter=$!
for _ in $(seq 50); do [ -s "$tmp/state/TEST-SERIAL/instrument.pid" ] && break; sleep 0.1; done
pid="$(cut -d' ' -f1 "$tmp/state/TEST-SERIAL/instrument.pid")"
sleep 0.5; kill -TERM "$starter"; wait "$starter" 2>/dev/null; sleep 0.3
expect "interrupted start" 0 'd["error"]=="interrupted"' -- cat "$tmp/interrupt.json"
check "interrupted start kills am instrument" bash -c "! kill -0 $pid 2>/dev/null"
check "interrupted start removes its forward" bash -c "! grep -q 'tcp:$dead' '$FAKE_ADB/forwards'"
exit $fail
