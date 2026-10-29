# UiAutomator2 tools for Cove Android testing

Drives an Android phone or emulator through Appium's
[UiAutomator2 server](https://github.com/appium/appium-uiautomator2-server), the
Android counterpart of WebDriverAgent. These tools let an agent read the screen as
text, tap elements by name, type and scroll. The server stays running on the device,
so each step is one HTTP call instead of a fresh `uiautomator dump`.

The `android-*` commands mirror the `ios-*` WDA scripts: same names, same arguments
where the platforms agree, and the same one-JSON-object output with an `"ok"` field.
They use only the Python standard library and talk to the server directly, with no
Appium server or Node.

Measured on a Galaxy S20 (Android 13), Cove's wallet screen:

| Step | raw adb | uia2 |
| --- | --- | --- |
| read the screen | 2.4 s (`uiautomator dump` + parse) | 0.4 s (`android-vis`) |
| tap | 0.12 s (`input tap`) | 0.13 s (`android-tap`) |
| output for the same screen | 45 lines, 1.8 KB | 18 lines, 1.3 KB, plus tags and overlays |

## Setup

```sh
export PATH="$PWD/tools/uia2:$PATH"
export ANDROID_SERIAL=<serial from adb devices>   # needed only when several devices are attached
```

1. **Connect the device with USB debugging on.** Allow the debugging prompt on the phone.
2. **Run `uia2-doctor`.** It checks adb, the device, the screen lock, the cached APKs, the
   installed server, other running instrumentation and the port. The `next` field says what to fix.
3. **Run `uia2-start`.** It downloads and verifies the pinned server APKs (v10.6.6) into
   `~/.cache/cove-uia2`, installs each one whose installed copy doesn't match its checksum,
   starts the server, and forwards it to `localhost:8200`. A server that already runs for this
   device is reused only when it is ready and on the pinned version. Samsung may ask to confirm
   the first install on the phone.
4. **Use the `android-*` commands.** Start with `android-launch org.bitcoinppl.cove`, then `android-vis`.
5. **Run `uia2-stop` when done.** Plain `uiautomator dump` and instrumented tests need the
   device's automation connection back (see below).

## Commands

| Command | Purpose |
| --- | --- |
| `uia2-doctor` | check prerequisites, print the next fix |
| `uia2-fetch` | download the pinned server APKs with `gh` and verify their checksums; `uia2-start` calls it |
| `uia2-start [--force]` | install if needed, run the server, forward `localhost:$UIA2_PORT`; refuses while other instrumentation, or another agent's server for the same device, is running |
| `uia2-stop` | stop the server and remove its forward, leaving other forwards on the port alone; when the device can't be reached it reports `cleanup_incomplete` and keeps its state, so rerun it after reconnecting |
| `android-launch [--activate] PACKAGE` | force-stop and start the app (`--activate` brings a running app forward instead); PACKAGE must be an Android package name, so shell syntax never reaches the device |
| `android-vis [-a]` | one line per visible element worth acting on, with its tap center in pixels; `-a` includes hidden ones |
| `android-source [--raw]` | full UI tree as XML; `--raw` prints only the tree |
| `android-tap X Y` | tap at pixel coordinates |
| `android-tap-label [--nth N] [--wait S] LABEL` | tap the element whose test tag, text or content-desc equals LABEL, waiting up to S seconds (default 5) for it to appear; the first lookup always completes |
| `android-pin DIGITS` | tap a PIN on Cove's keypad by its test tags (`numberPadPin.digit.N`); fails when that keypad isn't showing |
| `android-type [--enter] [--] TEXT` | set the focused field to TEXT, report whether the field took it, then press Enter if asked |
| `android-swipe [--hold S] up\|down\|left\|right` | finger swipe across the middle of the screen; `up` moves the finger up, so a list shows items further down |
| `android-drag [--hold S] X1 Y1 X2 Y2` | press at X1,Y1 for S seconds (default 1), then drag to X2,Y2 |
| `android-hold X Y [SECONDS]` | long-press at pixel coordinates (default 1 s) |
| `android-back` | press Back |
| `android-key NAME\|KEYCODE` | press a key: `enter`, `del`, `tab`, `paste`, `home`, `app_switch`, ... or a numeric keycode |
| `android-clipboard set [--] TEXT \| get` | write the clipboard; `get` works only below Android 10 (see below) |
| `android-shot [-s MAX_PIXELS] [--element LABEL] PATH.png` | screenshot to a file; `-s` downscales it with `sips`, `--element` crops to one element |

Action commands accept `--dry-run`, which prints the requests without sending them. Options
are read only before `--`, so `android-type -- --enter` types the text `--enter`.

## Reading `android-vis`

```
# U: 44 elements sit under a later full-screen layer (numberPadPin.viewport); they are probably hidden, check with android-shot
View           (540,349) 984x159 CU  contains='$1,216.15 | 1,435,591 SATS | Visible'
View           (954,709) 156x144 CX  row='Enable PIN'
View           (84,252) 72x72        desc='Back'
TextView       (540,408) 984x99      text='Enter Cove PIN'
View           (204,1465) 312x219 C  tag='numberPadPin.digit.1'
```

- `(x,y)` is the tap center in pixels, ready for `android-tap`; then the element's size
- Flags: `C` clickable, `E` editable, `S` scrollable, `F` focused, `X` checked, `D` disabled,
  `U` under a later full-screen layer
- `tag=` is a Compose `testTag` or a view's full resource ID, such as
  `com.android.permissioncontroller:id/permission_allow_button`. Cove exports test tags as
  resource IDs, so tags are the stablest way to find an element. Pass the tag exactly as
  printed to `android-tap-label`
- `contains=` names a tappable row by the words on its children. Those child lines are
  skipped because they would only repeat the row
- `row=` names a switch or checkbox by the text on the same row
- `U` matters on Cove's lock screen: the PIN pad is drawn over the screen it protects,
  and both stay in the tree. A line flagged `U` is probably not visible. Check with
  `android-shot` before relying on it

## Things that differ from iOS or raw adb

- **One automation connection per device.** Android allows a single UiAutomation client.
  While the server runs, `adb shell uiautomator dump` is killed and leaves the previous
  `/sdcard/ui.xml` in place, so a script that reads it silently shows an old screen. Run
  `uia2-stop` before using dump-based helpers or `connectedAndroidTest`
- **`uia2-start` refuses while other instrumentation runs.** Starting the server would take
  the connection from a running instrumented test and break it. It also refuses when it can't
  list the device's processes. Pass `--force` only when that run can be interrupted.
  `uia2-start` and `uia2-stop` hold a lock per device, so two agents can't start or stop the
  same device at once. A second one waits up to `UIA2_LOCK_WAIT` seconds, then fails with
  `locked`. If `uia2-start` fails or is interrupted after launching the server, it stops the
  server again before exiting, and reports `cleanup_incomplete` with the original error as
  `cause` when the device can't be reached to stop it
- **Coordinates are pixels**, not points as on iOS. Use the centers `android-vis` prints
- **`android-type` replaces the field's text.** It sets the whole text through
  accessibility, so characters can't arrive out of order the way fast `adb shell input
  text` keystrokes do, and `# & ? % "` need no escaping. It reports `readback_matches`
  instead of echoing the field, and its errors leave the text out, so typed secrets stay out
  of the output
- **The clipboard can be written but not read on Android 10+.** Only the foreground app or
  keyboard may read it. To check what was copied, paste it into a field with
  `android-key paste` and read the field with `android-vis`
- **Secure screens can't be captured.** `android-shot` returns `"protected": true` and
  writes no file on Cove's secret screens (`FLAG_SECURE`). That is the expected result
  there. The UI tree still exposes their text, so `android-vis` can read words that a
  screenshot can't show
- **Re-read after a swipe.** A fast swipe flings the list, and a read taken right away can
  catch it mid-scroll
- **`android-tap-label` taps the first match.** A section header and a row can share a label;
  pass `--nth 2` or tap the row's center from `android-vis`

## Screenshots

Navigate with `android-vis` and take screenshots only for what the tree can't show: layout,
clipping, colors, dark mode, large text. `-s 1200` cuts a 1080x2400 capture to about a
quarter of the image tokens. Without `-s`, keep the full-size file as evidence.

## Several devices

Each device needs its own port. Set `ANDROID_SERIAL` and `UIA2_PORT` together for every
command:

```sh
ANDROID_SERIAL=emulator-5554 UIA2_PORT=8201 uia2-start
ANDROID_SERIAL=emulator-5554 UIA2_PORT=8201 android-vis
```

## Environment

| Variable | Default | Meaning |
| --- | --- | --- |
| `ANDROID_SERIAL` | the only attached device | device to drive |
| `UIA2_PORT` | `8200` | localhost port forwarded to the server's 6790 |
| `UIA2_BASE_URL` | `http://127.0.0.1:$UIA2_PORT` | full server URL |
| `UIA2_WAIT` | `5` | seconds `android-tap-label` waits for its element |
| `UIA2_WAIT_FOR_IDLE` | `1` | seconds the server waits for the UI to settle before each command; read when a session is created, so restart the server after changing it |
| `UIA2_CACHE` | `~/.cache/cove-uia2` | APK cache and state, shared across worktrees |
| `UIA2_START_TIMEOUT` | `60` | seconds `uia2-start` waits for the server; a value that isn't a positive whole number fails with `invalid_timeout` before anything starts |
| `UIA2_LOCK_WAIT` | `30` | seconds `uia2-start` and `uia2-stop` wait for another one on the same device to finish |

State lives in `$UIA2_CACHE/state/`: the adb pid, port and server log per device under `<serial>/`, the device lock in `<serial>.flock`, and one cached session ID per server URL.

## Tests

`test/run-mock-tests.sh` runs every command against `test/mock-uia2.py`, with no device needed.
It needs macOS for `sips`.
