# WebDriverAgent tools for Cove iOS testing

Drives a real iPhone through [WebDriverAgent](https://github.com/appium/WebDriverAgent) with XCTest touches: taps, swipes, typing, screenshots and the accessibility tree as text. iPhone Mirroring ignores synthetic scrolls, so agents use these instead.

The `ios-*` commands match the [`android-*` UiAutomator2 tools](../uia2/README.md): same names, same arguments where the platforms agree, and the same one-JSON-object output with an `"ok"` field. They use only the Python standard library and talk to WDA directly, with no Appium server or Node.

## Setup

```sh
export PATH="$PWD/tools/wda:$PATH"
export IOS_UDID=<udid>   # from xcrun devicectl list devices
```

Needs Xcode signed in to the signing team (`WDA_TEAM`, default Cove's team) and `iproxy` (`brew install libimobiledevice`).

1. **Plug in the phone with USB, unlock it and trust this Mac.** `iproxy` forwards over USB.
2. **Turn on Developer Mode if asked:** Settings → Privacy & Security → Developer Mode, then restart.
3. **Run `wda-doctor`.** It checks Xcode, the signing team, the device, Developer Mode, USB visibility, `iproxy` and the build products. The `next` field says what to fix. Rerun until `"ok": true`.
4. **Run `wda-start`.** It clones the pinned WebDriverAgent (v16.15.3) into `~/.cache/cove-wda` and verifies its commit. Then it builds and signs the runner if needed, starts it on the phone, forwards it to `localhost:8100` and waits for `/status`. The first launch may refuse to open the runner: trust the developer certificate under Settings → General → VPN & Device Management, then rerun `wda-start`. Keep the phone unlocked from here on, and set Auto-Lock to Never for long runs.
5. **Use the `ios-*` commands.** Start with `ios-launch org.bitcoinppl.cove`, then `ios-vis`.
6. **Run `wda-stop` when done.** iPhone Mirroring only works while the phone is locked, so lock it to get Mirroring back.

## Commands

| Command | Purpose |
| --- | --- |
| `wda-doctor` | check prerequisites, print the next fix; read-only |
| `wda-fetch` | clone the pinned WebDriverAgent release and verify its commit; `wda-build` calls it |
| `wda-build` | signed `build-for-testing` for a generic iOS destination; `wda-start` calls it when needed |
| `wda-start` | build if needed, run WDA on the phone, forward `localhost:$WDA_PORT`, wait for `/status` |
| `wda-stop` | stop the runner and the forward that `wda-start` started |
| `ios-launch [--activate] BUNDLE_ID` | launch an app (`--activate` brings a running app forward instead of relaunching) |
| `ios-vis [-a]` | one line per visible element with a label, id or value, with its tap center and size in points; `-a` includes hidden ones |
| `ios-source [--json] [--raw]` | accessibility tree, XML by default; `--raw` prints only the tree |
| `ios-tap X Y` | tap at point coordinates (points, not pixels) |
| `ios-tap-label LABEL` | tap the element whose accessibility id, label or name matches exactly |
| `ios-type TEXT` | type into the focused field |
| `ios-swipe [--hold S] up\|down\|left\|right` | finger swipe across the middle of the screen; `up` moves the finger up, so a list shows items further down. The finger rests S seconds first (default 0.3) |
| `ios-drag [--hold S] X1 Y1 X2 Y2` | press at X1,Y1 for S seconds (default 1), then drag to X2,Y2 |
| `ios-hold X Y [SECONDS]` | long-press at point coordinates (default 1 s), for example to open the Paste menu |
| `ios-pasteboard [--app ID] get \| set TEXT` | read or write the pasteboard; see below |
| `ios-shot PATH.png` | screenshot to a file |

`wda-doctor`, `wda-start` and `wda-stop` read `IOS_UDID`. The `ios-*` commands only need WDA on localhost and echo `IOS_UDID` when it is set. Every command accepts `--dry-run`, which prints what it would do without touching the phone.

## Reading `ios-vis`

```
StaticText     (60,110) 80x20 label='About'
Button         (22,62) 44x44 label='Back' name='BackButton'
TextField      (160,220) 300x40 name='amount' value='0' [disabled]
```

Each line is the element type, its tap center in points, its size, then its label, its accessibility id (`name`, shown when it differs from the label) and its value. SwiftUI keeps hidden layers in the tree, so `ios-vis` hides elements XCTest reports as not visible; `-a` shows them marked `[hidden]`. A visible row can still be covered by a bar or sheet, so check `ios-shot` before tapping near screen edges.

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `IOS_UDID` | none | the phone for `wda-doctor`, `wda-start` and `wda-stop` |
| `WDA_PORT`, `WDA_BASE_URL` | `8100`, `http://127.0.0.1:$WDA_PORT` | where the `ios-*` commands find WDA |
| `WDA_TEAM` | Cove's team | signing team for the runner |
| `WDA_BUNDLE_ID` | `com.avencera.WebDriverAgentRunner` | runner bundle id; `ios-pasteboard` activates `$WDA_RUNNER_ID` (`<bundle id>.xctrunner`) |
| `WDA_CACHE` | `~/.cache/cove-wda` | WebDriverAgent checkout, build products and state, shared across worktrees |
| `WDA_WAIT_FOR_IDLE` | `0` | XCTest idle wait for new sessions; see below |

State is in `$WDA_CACHE/state/`: `wda.log` (the test run), `iproxy.log`, `build.log`, pids and the cached session id.

Offline check of the commands against a mock server: `tools/wda/test/run-mock-tests.sh`.

## Behavior to know

- **Pasteboard:** iOS only lets the foreground app read or write the pasteboard, so `ios-pasteboard` brings the runner forward, then reactivates the app. Cove sees a background and foreground cycle, which can lock it and dismiss an open sheet. `xcrun devicectl device pasteboard copy --device <udid> --file <path>` writes the pasteboard without switching apps. Universal Clipboard from a nearby Mac can overwrite it at any time
- **Allow Paste:** the first paste in an app raises iOS's Allow Paste prompt, which isn't in Cove's tree. Tap it by coordinates from a screenshot
- **Idle wait:** Cove's animations keep XCTest from reaching idle, which leaves the tree stale and drops swipes. New sessions set `waitForIdleTimeout` and `animationCoolOffTimeout` to `WDA_WAIT_FOR_IDLE`, default 0. Delete `state/session-id` after changing it
- **Swipe holds:** a 0.3 s rest starts SwiftUI drag-to-reorder in lists such as Cove's sidebar. Use `--hold 0.02` to scroll there
- **Long text:** XCTest truncates element values at 512 characters and labels at 4,096. Page through long text with screenshots and OCR
- **Stalls:** when Cove's main thread blocks, screenshots still work but tree reads and taps time out after 20–30 s
- **Text entry:** fast typing can drop characters. Read the field back with `ios-vis`, and type a character at a time if it doesn't match
- **Keyboard:** WDA's keyboard dismiss fails on iPhone. Use the app's own Done button or scroll the content
- **System UI:** the app switcher and Control Center can't be opened, and notification banners can cover the top bar

## Troubleshooting

- **`device_connected` or `usb_visible` fails in `wda-doctor`.** The phone must be on USB, unlocked and trusted. `idevice_id` may be at `/opt/homebrew/opt/libimobiledevice/bin/idevice_id` rather than on `PATH`
- **`xcodebuild_exited` from `wda-start`.** Read `state/wda.log`:
  - "untrusted developer", or the runner won't open: trust the certificate (setup step 4)
  - "Developer Mode disabled": setup step 2
  - "No profiles" or "device not registered": `wda-start` rebuilds with the phone as the destination and `-allowProvisioningUpdates`, which needs Xcode signed in to `WDA_TEAM`
  - "Unable to find a device matching": the phone is locked or not on USB
- **`status_timeout`.** The phone is probably locked or showing a passcode or trust prompt. Unlock it and rerun
- **`wda_unreachable` from an `ios-*` command.** `wda-start` isn't running or `iproxy` died. Check `state/iproxy.log`, then `lsof -i :8100` for another process on the port. `WDA_PORT=8101 wda-start` moves it; use the same variable for the `ios-*` commands
- **Black screenshots, the same small image every time, or elements not found.** The screen is asleep even though WDA reports it unlocked. Unlock it, and set Auto-Lock to Never for the session
- **Session errors after the phone sleeps.** Delete `state/session-id`; the next command creates a session
- **Stuck state.** `wda-stop`, replug the phone, `wda-start`
- **Newer WebDriverAgent.** Change `WDA_VERSION` and `WDA_COMMIT` in `wda-env.sh`, then run `wda-build`

These tools only install WDA's own runner app. Never use them to uninstall, reinstall or reset Cove.
