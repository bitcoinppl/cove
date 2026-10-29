#!/bin/bash
# exercise the ios-* scripts against test/mock-wda.py; no device needed
# usage: test/run-mock-tests.sh   (exits non-zero on the first failed assertion)
set -u
here="$(cd "$(dirname "$0")/.." && pwd)"
tmp="$(mktemp -d)"
port=$((20000 + RANDOM % 20000))
python3 -I "$here/test/mock-wda.py" "$port" "$tmp/requests.log" &
srv=$!
# stand-in tools record their pids in stub-pids, so none outlives the suite
trap '{ kill $srv $(cat "$tmp/stub-pids" 2>/dev/null); wait $srv; } 2>/dev/null; rm -rf "$tmp"' EXIT
export WDA_BASE_URL="http://127.0.0.1:$port" WDA_STATE="$tmp/state" IOS_UDID=TEST-UDID
for _ in $(seq 50); do curl -sf "$WDA_BASE_URL/status" >/dev/null && break; sleep 0.1; done

fail=0
# expect NAME EXIT_CODE JQ_STYLE_PYTHON_EXPR -- COMMAND...
expect() {
  local name="$1" code="$2" expr="$3"; shift 4
  local out rc
  out="$("$@" 2>&1)"; rc=$?
  if [ $rc -ne "$code" ] || ! printf '%s' "$out" | python3 -I -c "import json,sys; d=json.load(sys.stdin); sys.exit(0 if ($expr) else 1)"; then
    echo "FAIL $name (rc=$rc): $out"; fail=1
  else
    echo "ok   $name"
  fi
}

expect "swipe up" 0 'd["ok"] and d["direction"]=="up" and d["from"][1]>d["to"][1]' -- "$here/ios-swipe" up
expect "swipe bad dir" 2 'd["error"]=="usage"' -- "$here/ios-swipe" sideways
expect "swipe no args" 2 'd["error"]=="usage"' -- "$here/ios-swipe"
expect "swipe dry-run" 0 'd["dry_run"] and "would_send" in d' -- "$here/ios-swipe" --dry-run left
expect "tap" 0 'd["ok"] and d["x"]==10 and d["y"]==20.5' -- "$here/ios-tap" 10 20.5
expect "tap bad args" 2 'd["error"]=="usage"' -- "$here/ios-tap" ten 20
expect "tap one arg" 2 'd["error"]=="usage"' -- "$here/ios-tap" 10
expect "tap-label found" 0 'd["ok"] and d["label"]=="About"' -- "$here/ios-tap-label" About
expect "tap-label by label fallback" 0 'd["label"]=="Back"' -- "$here/ios-tap-label" Back
expect "tap-label missing" 1 'd["error"]=="label_not_found"' -- "$here/ios-tap-label" Nope
expect "type" 0 'd["typed_chars"]==5' -- "$here/ios-type" "hello"
expect "type no args" 2 'd["error"]=="usage"' -- "$here/ios-type"
expect "launch" 0 'd["bundle_id"]=="org.bitcoinppl.cove"' -- "$here/ios-launch" org.bitcoinppl.cove
expect "launch activate" 0 'd["action"]=="activate"' -- "$here/ios-launch" --activate org.bitcoinppl.cove
expect "launch no args" 2 'd["error"]=="usage"' -- "$here/ios-launch"
expect "source xml" 0 '"About" in d["source"]' -- "$here/ios-source"
expect "source json" 0 'd["format"]=="json"' -- "$here/ios-source" --json
expect "shot" 0 'd["bytes"]>0' -- "$here/ios-shot" "$tmp/out/shot.png"
expect "shot no args" 2 'd["error"]=="usage"' -- "$here/ios-shot"
expect "pasteboard get" 0 'd["text"]=="mock clip" and d["app"]=="org.bitcoinppl.cove"' -- "$here/ios-pasteboard" get
expect "pasteboard set" 0 'd["op"]=="set" and d["chars"]==4' -- "$here/ios-pasteboard" set "a #b"
expect "pasteboard set no text" 2 'd["error"]=="usage"' -- "$here/ios-pasteboard" set
expect "pasteboard bad op" 2 'd["error"]=="usage"' -- "$here/ios-pasteboard" clear
expect "pasteboard dry-run" 0 'len(d["would_send"])==3 and d["would_send"][2]["body"]["bundleId"]=="x.y"' -- "$here/ios-pasteboard" --dry-run --app x.y get
expect "swipe hold" 0 'd["hold"]==0.05' -- "$here/ios-swipe" --hold 0.05 up
expect "swipe hold bad" 2 'd["error"]=="usage"' -- "$here/ios-swipe" --hold x up
expect "drag" 0 'd["hold"]==1.0 and d["to"]==[3.0,4.0]' -- "$here/ios-drag" 1 2 3 4
expect "drag hold" 0 'd["hold"]==2.0' -- "$here/ios-drag" --hold 2 1 2 3 4
expect "drag bad args" 2 'd["error"]=="usage"' -- "$here/ios-drag" 1 2 3
expect "hold" 0 'd["seconds"]==1.0 and d["x"]==5' -- "$here/ios-hold" 5 6
expect "hold bad args" 2 'd["error"]=="usage"' -- "$here/ios-hold" 5
WDA_BASE_URL=http://127.0.0.1:1 expect "unreachable" 1 'd["error"]=="wda_unreachable"' -- "$here/ios-source"

# source --raw prints the tree itself, not JSON
"$here/ios-source" --raw | grep -q 'label="About"' && echo "ok   source --raw" || { echo "FAIL source --raw"; fail=1; }
vis="$("$here/ios-vis")"
[ "$vis" = "$(printf '%s\n%s' 'StaticText     (60,110) 80x20 label='"'About'" 'TextField      (160,220) 300x40 name='"'amount'"' value='"'0'"' [disabled]')" ] && echo "ok   vis prints visible elements" || { echo "FAIL vis: $vis"; fail=1; }
"$here/ios-vis" -a | grep -q "^Button .* label='Back' name='BackButton' \[hidden\]$" && echo "ok   vis -a includes hidden" || { echo "FAIL vis -a"; fail=1; }
expect "stop with nothing running" 0 'd["ok"] and d["stopped"]=="none"' -- env -u IOS_UDID WDA_CACHE="$tmp/cache" "$here/wda-stop"
expect "stop one phone dry-run" 0 'd["dry_run"] and d["stopped"]=="none"' -- env WDA_CACHE="$tmp/cache" "$here/wda-stop" --dry-run
expect "fetch dry-run" 0 'd["dir"].endswith("/WebDriverAgent-v16.15.3")' -- env WDA_CACHE="$tmp/cache" "$here/wda-fetch" --dry-run
expect "type text after --" 0 'd["typed_chars"]==9 and not d["dry_run"]' -- "$here/ios-type" -- --dry-run
expect "type dry-run before --" 0 'd["dry_run"] and d["would_send"][0]["body"]["value"]==list("-x")' -- "$here/ios-type" --dry-run -- -x
expect "pasteboard set text after --" 0 'd["chars"]==5 and d["app"]=="org.bitcoinppl.cove"' -- "$here/ios-pasteboard" set -- --app
expect "pasteboard --app without value" 2 'd["error"]=="usage"' -- "$here/ios-pasteboard" --app
expect "launch activate after -- is a bundle id" 2 'd["error"]=="usage"' -- "$here/ios-launch" -- --activate x.y

# wda-start, wda-stop and wda-build take turns over the shared WDA state
locked_env=(env WDA_STATE="$tmp/lockstate" WDA_CACHE="$tmp/cache" WDA_LOCK_WAIT=0.5)
(export WDA_STATE="$tmp/lockstate"; . "$here/wda-env.sh"; lock_state; sleep 3) &
locker=$!
sleep 1
expect "stop waits for the lock" 1 'd["error"]=="locked"' -- "${locked_env[@]}" "$here/wda-stop"
expect "build waits for the lock" 1 'd["error"]=="locked"' -- "${locked_env[@]}" "$here/wda-build"
expect "stop dry-run skips the lock" 0 'd["dry_run"]' -- "${locked_env[@]}" "$here/wda-stop" --dry-run
wait $locker
expect "stop after the lock frees" 0 'd["ok"]' -- "${locked_env[@]}" "$here/wda-stop"
# wda-start runs wda-stop under its lock, so the child must not wait on its own parent
expect "child of the holder" 0 'd["ok"] and d["action"]=="wda-stop"' -- "${locked_env[@]}" bash -c '. "$1"; lock_state; "$2"; true' _ "$here/wda-env.sh" "$here/wda-stop"

# stand-ins for xcodebuild, iproxy and the device tools. Each runs under a symlink to bash named
# after the tool, so ps reports that name for it the way it does for the real tool
mkdir -p "$tmp/bin" "$tmp/interp"
stub() { # stub NAME BODY
  ln -s /bin/bash "$tmp/interp/$1"
  printf '#!%s\n%s\n' "$tmp/interp/$1" "$2" >"$tmp/bin/$1"
  chmod +x "$tmp/bin/$1"
}
# a long-running stand-in records its pid in NAME-ARG1.pid, then on SIGTERM waits
# STUB_STOP_DELAY seconds, touches NAME-ARG1.stopped and exits
export STUB_DIR="$tmp"
long_running='echo $$ >>"$STUB_DIR/stub-pids"
mark="$STUB_DIR/$(basename "$0")-$1"
echo $$ >"$mark.pid"
trap '\''kill $!; sleep "${STUB_STOP_DELAY:-0}"; touch "$mark.stopped"; exit 143'\'' TERM
sleep 30 & wait'
# STUB_RUNNER_EXIT makes the runner fail a second in, after the forward has started
stub xcodebuild '[ "$1" = test-without-building ] && [ -n "${STUB_RUNNER_EXIT:-}" ] && { sleep 1; exit "$STUB_RUNNER_EXIT"; }
'"$long_running"
stub iproxy "$long_running"
stub idevice_id 'echo "$IOS_UDID"'
stub security 'echo "$IOS_UDID"'
alive() { kill -0 "$(cat "$1" 2>/dev/null)" 2>/dev/null; }
stub_env=(env PATH="$tmp/bin:$PATH" WDA_DERIVED="$tmp/derived")

# wda-build stops its xcodebuild before a signal ends it, so the lock never frees mid-build
git init -q "$tmp/wda-repo" && git -C "$tmp/wda-repo" -c user.name=t -c user.email=t commit -q --allow-empty -m x
"${stub_env[@]}" WDA_STATE="$tmp/buildstate" WDA_REPO="$tmp/wda-repo" WDA_COMMIT="$(git -C "$tmp/wda-repo" rev-parse HEAD)" \
  STUB_STOP_DELAY=1 "$here/wda-build" >"$tmp/build.json" &
build=$!
for _ in $(seq 50); do [ -s "$tmp/xcodebuild-build-for-testing.pid" ] && break; sleep 0.1; done
kill -TERM $build
wait $build
if [ -e "$tmp/xcodebuild-build-for-testing.stopped" ] && ! alive "$tmp/xcodebuild-build-for-testing.pid" &&
  grep -q '"error": "interrupted"' "$tmp/build.json"; then
  echo "ok   build signal stops xcodebuild first"
else
  echo "FAIL build signal: $(cat "$tmp/build.json")"; fail=1
fi

# a start whose runner exits early stops the forward it started, so the port is free again. The
# products below are a ready build, so the start must not rebuild; the fake checkout keeps a
# rebuild it does attempt offline
mkdir -p "$tmp/derived/Build/Products/Debug-iphoneos/WebDriverAgentRunner-Runner.app"
touch "$tmp/derived/Build/Products/WebDriverAgentRunner.xctestrun"
python3 -I -c 'import plistlib, sys; plistlib.dump({"CFBundleIdentifier": "com.avencera.WebDriverAgentRunner.xctrunner"}, open(sys.argv[1], "wb"))' \
  "$tmp/derived/Build/Products/Debug-iphoneos/WebDriverAgentRunner-Runner.app/Info.plist"
start_state="$tmp/startstate/devices/TEST-UDID"
expect "start stops its forward when the runner exits" 1 'd["error"]=="xcodebuild_exited"' -- \
  "${stub_env[@]}" WDA_STATE="$tmp/startstate" WDA_REPO="$tmp/wda-repo" WDA_COMMIT="$(git -C "$tmp/wda-repo" rev-parse HEAD)" \
  WDA_PORT=1 WDA_BASE_URL=http://127.0.0.1:1 STUB_RUNNER_EXIT=65 "$here/wda-start"
if [ -s "$tmp/iproxy--u.pid" ] && ! alive "$tmp/iproxy--u.pid" && [ ! -e "$start_state/iproxy.pid" ] && [ ! -e "$start_state/port" ]; then
  echo "ok   failed start leaves no forward or pid files"
else
  echo "FAIL failed start left a forward or $(ls "$start_state")"; fail=1
fi
if [ ! -e "$start_state/wda-build.json" ]; then
  echo "ok   start reuses a ready build"
else
  echo "FAIL start rebuilt a ready build: $(cat "$start_state/wda-build.json")"; fail=1
fi

# wda-stop leaves a recorded pid alone when another phone's runner reused it
stop_state="$tmp/stopstate/devices"
mkdir -p "$stop_state/PHONE-A" "$stop_state/PHONE-B"
"$tmp/bin/xcodebuild" test-without-building -xctestrun x -destination id=PHONE-B &
echo $! | tee "$stop_state/PHONE-A/xcodebuild.pid" >"$stop_state/PHONE-B/xcodebuild.pid"
"$tmp/bin/iproxy" -u PHONE-B 8101:8100 &
echo $! | tee "$stop_state/PHONE-A/iproxy.pid" >"$stop_state/PHONE-B/iproxy.pid"
expect "stop skips another phone's pids" 0 'd["stopped"]=="none"' -- \
  env WDA_STATE="$tmp/stopstate" IOS_UDID=PHONE-A "$here/wda-stop"
if alive "$stop_state/PHONE-B/xcodebuild.pid" && alive "$stop_state/PHONE-B/iproxy.pid" && [ -z "$(ls "$stop_state/PHONE-A")" ]; then
  echo "ok   other phone's runner survives, stale pid files removed"
else
  echo "FAIL other phone's runner was stopped or stale pid files kept"; fail=1
fi
expect "stop stops its own phone's pids" 0 '"PHONE-B:xcodebuild:" in d["stopped"] and "PHONE-B:iproxy:" in d["stopped"]' -- \
  env WDA_STATE="$tmp/stopstate" IOS_UDID=PHONE-B "$here/wda-stop"
expect "vis bad args" 2 'd["error"]=="usage"' -- "$here/ios-vis" --all
expect "vis dry-run" 0 'd["dry_run"] and "would_send" in d' -- "$here/ios-vis" --dry-run
[ "$(head -c4 "$tmp/out/shot.png" | tail -c3)" = "PNG" ] && echo "ok   shot file is a PNG" || { echo "FAIL shot file"; fail=1; }
grep -q '"/session/mock-session/wda/dragfromtoforduration"' "$tmp/requests.log" && echo "ok   drag request sent" || { echo "FAIL drag request"; fail=1; }
grep -q '"/session/mock-session/wda/setPasteboard", "body": {"contentType": "plaintext", "content": "YSAjYg=="}' "$tmp/requests.log" && echo "ok   pasteboard set body is base64" || { echo "FAIL pasteboard set body"; fail=1; }
grep -q '"/session/mock-session/wda/touchAndHold"' "$tmp/requests.log" && echo "ok   hold request sent" || { echo "FAIL hold request"; fail=1; }
exit $fail
