# Device scripts

A stdlib-only Python runner that executes release-checklist rows against one device and writes a JSON result per row. A row is covered by a script when its manifest entry lists `device-script:qa/device/<platform>/rows.py::<row id>` (see [../README.md](../README.md)).

Scripts only navigate and observe. They never send, broadcast, sign, delete, export, share, wipe, sign out or touch cloud backup.

## Safety rules

- Every tap goes through the driver, which refuses any control whose label matches `FORBIDDEN_LABEL` in `common.py` (send, delete, export, backup, restore, sign out, and similar). Rows must not use raw `input tap`
- Preflight blocks every row unless the app is on Signet, Testnet or Testnet4. It reads the network from Settings and never changes it
- Android requires `--serial`, so a second attached phone is never picked by accident. Pass the serial of the device you mean to test
- OS settings a row changes (font scale, animation scales, airplane mode) go through context managers in `android/driver.py` that restore the original value, even when the row fails. iOS rows don't change OS settings, so none are scripted that would need to
- The Android upgrade row reinstalls an apk over the existing app and keeps its data, so run it only on a device you are happy to reinstall on
- A failure screenshot is taken only while the app is in front, so another app's screen never ends up in the results

## Running

From the repo root:

```sh
# Android: one device or emulator, app already installed with a wallet on a test network
python3 -I qa/device/run.py android --serial emulator-5554 --package org.bitcoinppl.cove.dev

# the upgrade row needs the build to install over an older release of the same package, otherwise it is blocked
python3 -I qa/device/run.py android --serial emulator-5554 --upgrade-apk path/to/app-dev-debug.apk

# iOS: needs WebDriverAgent running on the phone, see below
python3 -I qa/device/run.py ios --bundle-id org.bitcoinppl.cove

# list rows, or run a subset
python3 -I qa/device/run.py android --serial emulator-5554 --list
python3 -I qa/device/run.py android --serial emulator-5554 --rows s15-repeated-navigation-and-background-route-scope --loops 30
```

`--loops` sets how many background and foreground cycles the s15 row repeats (default 10, about 20 seconds each on an emulator); raise it for a long-session soak. It must be at least 1. Each cycle opens the next of the Appearance, Network and Currency settings screens and the Receive sheet, on both platforms.

The s14 scan rows pass only after they see a history scan running and then finishing. A wallet that has finished its first scan hides scan progress for its first 5 seconds, so a scan that finishes sooner can't be told apart from one that never started, and the row is blocked instead. Use a wallet with transactions whose scan takes longer than that. An error shown while the row waits for the scan, such as `Node Connection Failed`, fails the row with that error's text.

Offline unit tests, which need no device: `python3 -I -m unittest discover -s qa/device -p "test_*.py"`

### Android prerequisites

- `adb` on `PATH`, and the device visible to `adb devices`
- The app installed and a wallet created on Signet, Testnet or Testnet4. Onboarding can be completed by hand, and a new wallet can skip word verification
- A run takes about 7 minutes on an emulator, most of it the s15 loop and the font and animation rows
- For the upgrade row: the previous release installed with a wallet open in it, an apk of the build under test for the same package with a higher versionCode, and `aapt2` from the SDK build-tools, found through `ANDROID_HOME`, `ANDROID_SDK_ROOT` or the SDK holding `adb`. The row reads the apk's package and versionCode before installing, and is blocked when the package differs or the versionCode isn't higher. The row compares the wallet names in the sidebar before and after the upgrade, so every wallet must fit in the sidebar without scrolling; when the list scrolls, the row is blocked

### iOS prerequisites

- A phone prepared with WebDriverAgent as described in `$WDA_TOOLS/README.md` (default `~/code/tools/wda`): `wda-doctor`, then `wda-start`, with the phone unlocked for the whole run
- WDA reachable at `WDA_BASE_URL` (default `http://127.0.0.1:8100`); `wdalib.py` from `$WDA_TOOLS` is imported for sessions and HTTP
- The app installed, on a wallet home, on a test network
- Nothing else driving the phone during the run. WDA serves every client on the same session, and another client switching apps mid-row makes the row fail. The driver reads and taps only while the app is in front, so a label in another app is never tapped

The iOS rows have been checked only against canned WDA output (`test_device_scripts.py`), not a phone. Watch the first run. The iOS rows read the network from the checkmark in Settings, Network, open the sidebar through the `app.sidebar.open` identifier, and recognize the settings root by its `General` section header; if any of these lookups fails, preflight blocks the rows and says why.

## Results

Each run writes to `--out`, default `_scratch/device-results/<platform>-<timestamp>/`:

- `<platform>-preflight.json` and `<platform>-<row id>.json`
- `<platform>-<row id>.png`, a screenshot taken when a row fails
- `summary.json`: platform, preflight status, counts and each row's status

A row file holds:

```json
{
  "row": "s16-receive-copy-address-missing-at-largest-font",
  "platform": "android",
  "status": "pass",
  "message": "",
  "started_at": 1792611949.67,
  "seconds": 22.6,
  "evidence": {"crashes": [], "largest_font": {"found": ["Copy Address", "Create New Address"], "scrolls": 0}}
}
```

`status` is one of:

- `pass`: the row's assertions held
- `fail`: an assertion about the app failed, which is a finding. Evidence names the screen labels seen
- `blocked`: the device or fixture wasn't in a state the row can run from (no wallet, wrong network, no `--upgrade-apk` or one that isn't an upgrade, a scan too quick to see, more wallets than the sidebar shows at once), so the result says nothing about the app. A failed preflight blocks every row
- `error`: the script itself broke or tried a forbidden tap; `evidence.traceback` has the stack

## Exit codes

- `0`: every selected row passed
- `1`: any row failed, was blocked or errored
- `2`: bad command line arguments, such as a missing `--serial`, an unknown row id or a `--loops` below 1

## Calling it from a scheduler

No schedule is set up. A scheduler should run the command above on a machine that has the device attached, pass an explicit `--serial` (Android) and a fresh `--out`, and treat the exit code as the verdict. Keep `summary.json` and the per-row files, and upload the screenshots from failed rows. A `blocked` result means the device needs attention, not that the app regressed, so route it to whoever maintains the device.
