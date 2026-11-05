# Testing

Cove is tested in layers. Each row of the release checklist is decided by the cheapest layer that can catch a regression in it, and a release only sends an agent or a person to the rows that nothing cheaper covers. The manifest that records this is described in [qa/README.md](../qa/README.md).

## Table of Contents

- [Layers](#layers)
- [Hardware signer tests](#hardware-signer-tests)
- [Planning a release pass](#planning-a-release-pass)
- [Fixtures](#fixtures)
- [Hardware signing UI tests](#hardware-signing-ui-tests)
- [Driving devices](#driving-devices)
- [Running an agent pass](#running-an-agent-pass)
- [Recording results](#recording-results)
- [Rows that need a person](#rows-that-need-a-person)
- [After a pass](#after-a-pass)

---

## Layers

| Layer | What it covers | How to run it |
| --- | --- | --- |
| Rust tests | Core behavior shared by both apps: wallets, send flow, labels, KeyTeleport, cloud backup, diagnostics, node and explorer settings, and the hardware-signer QR loop | `just test` |
| TAPSIGNER emulator tests | Cove's TAPSIGNER setup, PIN, derive, import and signing logic against Coinkite's card emulator | `just test-tapsigner-emulator` |
| iOS UI tests | Full-launch flows in the simulator | `just ios-ui-background` runs `CoveUITests/OnboardingFullLaunchUITests` by default; CI runs every class in the workflow's `IOS_UI_SUITES`. Pass other classes or methods, comma-separated, as the second argument, as in `just ios-ui-background "iPhone 17" CoveUITests/<Class>,CoveUITests/<Class>/<method>`. Classes that need scripted cloud storage and passkeys run in the Cloud Backup host app: name them under `CoveCloudBackupUITests/<Class>` and the scheme of that name is picked for you. `just ios-ui-foreground` shows the simulator |
| Android instrumented tests | Compose screens and full-launch flows on an emulator or device | `just android-instrumented-ci`, or `just android-instrumented-ci UiTest <package>.<Class>#<method>` for some tests in one flavour; staged process-death tests need `just android-ui-manual` |
| Device scripts | Navigate-and-observe checks on a real phone, with JSON results per row | `python3 -I qa/device/run.py android --serial <serial> --package org.bitcoinppl.cove.dev` or `python3 -I qa/device/run.py ios --bundle-id org.bitcoinppl.cove`; see [qa/device/README.md](../qa/device/README.md) for setup, `--list` and running a subset of rows |
| Agent pass | Checklist rows that need judgment, fixtures or multi-app flows, run by a coding agent driving a real phone | This document |
| Person | Camera, NFC, biometrics, haptics, spoken screen-reader output, hardware signers beyond the [hardware signing UI tests](#hardware-signing-ui-tests) | Batched into one sitting at the end |

UI tests that need a funded wallet get it from the fake Esplora and Electrum node in [`rust/crates/cove-fake-node`](../rust/crates/cove-fake-node/README.md), so they need no Signet funds or network. `just ios-ui-background`, `just ios-ui-foreground`, `just android-ui-manual` and `just android-instrumented-ci` start it on ports 39000 (control API), 39001 (Esplora) and 39002 (Electrum) for the length of the run, together with the hardware signer host on 39010 and 39011 (see [Hardware signing UI tests](#hardware-signing-ui-tests)), and forward the ports to Android with `adb reverse`. With more than one Android device connected, set `ANDROID_SERIAL` to pick one, since tests on two devices would reset and fund the same node. To run another command with the node up, use `just fake-node-run <command>` or `cargo xtask fake-node run -- <command>` from `rust/`.

The CI workflow runs the TAPSIGNER emulator tests in their own job. The `UI tests` workflow runs the iOS UI tests and Android instrumented tests nightly, on pushes to master, and on pull requests labeled `ui-tests`. It splits the iOS suites listed in its `IOS_UI_SUITES` across three macOS runners, `ios-ui (1/3)` through `ios-ui (3/3)`; add a new suite class to that list. Android runs one job per flavour.

`xtask ios-ui`, which the `ios-ui-*` recipes and CI run, builds each UI test scheme once with `xcodebuild build-for-testing` and runs each test with `test-without-building` on a simulator of its own. It creates a golden simulator from the model and runtime of the `--device` simulator and boots it once, then clones it for each test and each retry. A clone starts with no app, app data, keychain items or privacy grants, as an erased simulator does, without the erase and the slow first boot after it. The `--device` simulator itself is left alone. The run deletes its simulators, named `Cove UI Test`, when it ends, and the next run deletes any that a stopped run left behind.

Android has no such isolation. The test orchestrator starts a fresh process for each test, but every test in a flavour shares the app's data, and CI runs them in order of fully qualified class name. A full-launch test usually wipes the data when it launches the app, but a test that hosts a screen directly does not. Tests that set up an app lock skip themselves when one is already there, so they never replace a lock a person relies on, which means a lock left behind by an earlier test silently skips them. So a test that sets a PIN, decoy PIN, wipe data PIN or biometric unlock calls `restoreNoAppLock()` from `@After`, which removes all of them, leaves decoy mode and fails the test if the lock survives. Before tapping a full-launch screen with UiAutomator, wait for the target to stop moving: the lock cover slides up over the screen, and a key found mid-slide sends the tap to its neighbour

To check a fix without rerunning every suite, start the workflow on the branch with only the failed tests:

```sh
gh workflow run ui-tests.yml --ref <branch> \
  -f ios_tests=<Target>/<Class>/<method> \
  -f android_tests=<package>.<Class>#<method>
```

Both inputs take comma-separated lists, and a class runs all its tests. An iOS entry must be `<Target>/<Class>`, with `<Target>` `CoveUITests` or `CoveCloudBackupUITests` and a class that target builds, optionally followed by one of its test methods without `()`; anything else, such as a bare target or a module-qualified class, fails the run instead of running tests unexpanded. The iOS shards split the listed tests as they split a full run, each Android entry runs in every flavour whose test sources hold its class, and jobs with nothing listed finish without running tests. A manual run with both inputs empty runs everything.

A manual run shows under Actions but not in a pull request's checks, so it confirms a fix without clearing a failed check. To clear one, use GitHub's "Re-run failed jobs" on the pull request's run, which reruns only the failed shard or flavour, or push the fix.

CI runs the manifest check behind `just qa-check` on pull requests, so the manifest stays consistent with the tests it names. Its workflow filters pull requests to those targeting master, so run `just qa-check` locally before pushing a checklist change on a stacked branch.

## Hardware signer tests

These Rust tests run the PSBT signing loops that a release pass would otherwise leave to a person with a camera or a card.

- **QR signing round trip** (`rust/src/hardware_signing_tests.rs`). A watch-only wallet is imported from the public descriptor of a fixed BIP39 test mnemonic and funded offline. Cove's send code then builds and saves a two-input draft, which is exported as animated UR (`crypto-psbt`) and BBQr frames with Cove's encoders. A BDK signer for the same mnemonic reads those frames with the `foundation-ur`, `ciborium` and `bbqr` crates, signs the PSBT and re-encodes it with the same crates. The signed frames go through `QrScanner::scan` one at a time: out of order, with repeats, and for UR with a missing frame that fountain parts later fill in. The result goes through the draft lookup and `finalize_psbt` that the hardware send screen uses, and the finalized transaction must spend the drafted inputs and pay the drafted recipient, change and fee, with valid signatures. The error-path test covers a truncated BBQr set, a PSBT from an unrelated signer, a PSBT the same seed signed on mainnet, and a signature for a deleted draft. Both tests run in `just test`
- **TAPSIGNER against the card emulator** (`rust/src/tap_card/tap_signer_reader/emulator_tests.rs`). Each test starts its own factory-fresh emulated card and drives Cove's `TapSignerReader` through the `TapcardTransportProtocol` callback that the iOS and Android NFC layers implement. The tests cover setup with the default and an advanced chain code, the factory PIN being replaced, a wrong PIN during setup, derive, backup and signing followed by a successful retry, import of the card's account as a wallet whose addresses must match the card's own xpub, and signing and finalizing a saved draft. The emulator's certificate chain doesn't lead to Coinkite's factory root, so the tests skip only that check after asserting that `create_tap_signer_reader` rejects the card. The emulator implements neither the rate limit nor `wait`, so the auth-delay paths stay covered only by the mock-transport tests in `tap_signer_reader.rs`

`just test-tapsigner-emulator` runs the emulator tests. It finds `ecard.py` in the `coinkite-tap-proto` submodule of the pinned rust-cktap checkout, installs the emulator's Python dependencies with uv into `~/.cache/cove/tapsigner-emulator-venv`, and runs the tests under cargo-nextest with `COVE_TAPSIGNER_EMULATOR_PYTHON` and `COVE_TAPSIGNER_EMULATOR_SCRIPT` set. The run fails when its filter selects no emulator test, so a renamed module cannot pass without running them. `just test` and `cargo test` list these tests as ignored, and when they are run without the emulator they fail with a message naming the recipe.

## Planning a release pass

1. **Identify the artifact.** Record the version, build number and commit from Settings → About on each phone. Plan against that commit, not the local branch: a TestFlight or Play build can lag the branch it was cut from.
2. **List the rows.** `just qa-plan <last-tested-commit>...<release-commit>` lists the rows whose code changed and that no automated test decides, split into agent and human rows. Use `just qa-plan-all` for a full pass.
3. **Run the automated layers first.** Green CI and a device-script run on each phone remove rows from the agent's list.
4. **Hand the agent rows to an agent per phone.** One agent per device at a time; WebDriverAgent serves every client on one session, so two drivers interfere.
5. **Batch the human rows** into one sitting after the agents finish, with the fixtures ready.

## Fixtures

Use test networks only. Cove's default Signet node is MutinyNet, whose 30-second blocks make funding fast. Never use production seed material, and keep fixture secrets out of the repository.

Automated UI tests don't use these fixtures. They reset the fake node, fund the wallet they import through its control API (`POST /reset`, then `POST /fund-wallet` with a public test mnemonic such as abandon ×11 about), and point the app at a custom Esplora node at `http://127.0.0.1:39001`. Debug builds also fetch fee rates from `COVE_FEE_URL` when it is set, and the tests point it at the fake node so a send never reaches mempool.space. On iOS a fresh install can't reach Settings before it has a wallet, so debug builds launched with `COVE_UI_TEST_ESPLORA_URL` select Signet and that Esplora node at startup. The [crate README](../rust/crates/cove-fake-node/README.md) documents the API. `UtxoLockFullLaunchUITests` and `UtxoLockFullLaunchTest` use this setup to lock and unlock UTXOs from coin control and transaction details, then check which outputs the app's broadcasts spent.

- **Two funded Signet wallets** (for example Main and Secondary) with several UTXOs, including one small output, and known history
- **A watch-only hardware wallet** backed by a software signer such as Signer Lab, so the PSBT loop runs without a camera: export the unsigned PSBT to a file, review and sign it with the signer's CLI, then import the signed file in Cove. For QR and file fixtures, `just sign-psbt <psbt>`, where `<psbt>` is base64 or an absolute path to a `.psbt` file, signs it with the test mnemonic in `$MNEMONIC` and writes the result as base64, hex, binary, BBQr and UR images
- **Public-key fixtures** (`xpub`, `ypub`, `zpub`, `upub`, `vpub`, descriptors). To check the first receive address, use a freshly generated key with no history: published test vectors have on-chain history, so the receive index has already advanced past `/0/0` by the time Receive opens
- **A LAN HTTP block explorer** on the test machine, so explorer templates can be checked against the request log
- **The previous release** for in-place upgrade tests. On Android, the GitHub release APK and a locally signed build share the upload key, but a Play install is signed with Google's key and only updates through Play. iOS upgrades need the previous build from the App Store, since TestFlight only lists builds of the current version
- **Approval for destructive rows** before the run: clean install, uninstall, Wipe Data PIN and cloud backup deletion each need a recovery path prepared first

## Hardware signing UI tests

`HardwareSigningFullLaunchUITests` (iOS) and `HardwareSigningFullLaunchTest` (Android, `androidTestUiTest`) sign sends from hardware wallets without a camera or a card. They cover:

- **Animated QR signing:** a watch-only wallet imported from a scanned export sends Max, opens the unsigned PSBT's UR or BBQr export and takes the PSBT from its Copy button (the tests don't decode the displayed QR), imports the signature as animated frames fed in reverse order, checks the confirm screen and Advanced Details, and broadcasts to the fake node
- **Partial frames:** two BBQr frames show `Scanned 2 of N` and the parts left, a repeated frame doesn't count twice, and the rest complete the scan
- **Signatures that match no draft:** a signature for a deleted draft, one for an unrelated transaction and a mainnet one each show No Unsigned Transaction Found. On Android, a pasted signature for the deleted draft shows a Paste Error saying the transaction isn't pending. iOS doesn't test paste: under XCUITest the app reads an empty pasteboard, even after the host sets it with `simctl pbcopy`, and no paste prompt shows. Nothing checks a PSBT's network, so a mainnet signature is reported as an unknown transaction
- **TAPSIGNER:** import and signing through an emulated card, each with a wrong CVC and then a session that drops partway through before the retry succeeds. Until [#965](https://github.com/bitcoinppl/cove/issues/965) is fixed, rust-cktap rejects every BIP84 PSBT before it contacts the card, so the wrong CVC ends in the generic signing failure instead of Wrong PIN. Every signing error shows the same message, so in UI test builds the app also exposes the underlying one: iOS adds it to the Signing Failed! alert when the TAPSIGNER bridge is configured, and Android records it in its debug-only `UiTestSeams`. When the error is #965's Invalid path, the signing test stops there, as an expected failure on iOS and a skipped test on Android. Any other signing error fails the test, and Wrong PIN runs the whole flow

Two more suites use the same seams. `PublicKeyImportFullLaunchUITests` and `PublicKeyImportFullLaunchTest` import a hardware wallet from its bare public key, a vpub in a plain QR and a upub in a single-part BBQr, and check that Receive shows the first address and path of the address type the key names. The signer host derives the expected address from the mnemonic's key at `/0/0`, not from a descriptor. `TapSignerLayoutFullLaunchUITests` and `TapSignerLayoutFullLaunchTest` set up a factory-fresh emulated card and then import a set-up one at the largest text size (the `AccessibilityXXXL` content size on iOS, a system font scale of 2 on Android), dropping a session in each so the retry screens show. On the choose, advanced, PIN, retry and success screens they scroll each title and action into view and fail if it never fits inside the window, if an action is disabled or takes no tap, or, on Android, if it is squeezed. They also fail when a title, description or action label draws truncated in place, which neither platform's UI tree shows, since both report a label's whole text. Android reads the layout of every text in the TAPSIGNER sheet from Compose and fails if a line is ellipsized or the text needs more lines, height or width than it gets. iOS titles, descriptions and button labels carry the truncation probe below. Signing asks for the CVC with its own message, and no test opens that screen at the largest text size, so its layout still needs a person

### Test seams

Debug builds have three seams the tests drive. Each is off unless its environment variable is set, and none is in release builds: iOS compiles them under `#if DEBUG`, which only the Debug configuration defines, Android keeps them in `src/debug`, with inert stubs in `src/release`, and the Rust part is under `cfg(debug_assertions)`, which no release profile enables. `cargo xtask check-release-seams` (from `rust/`; `--ios`, `--android` or `--rust` for one layer) builds the release Swift code, the shrunk release dex and the `release-speed` Rust library, and fails if a seam's environment variable name or the emulator's root key is in any of them, with a debug build of each as the control that the check would find them. Xcode and Gradle package whichever Rust library was built last, so device Release builds and archives in Xcode, and every Gradle release variant, also run `tools/release-seams/check-packaged-rust.sh` on the library they are about to package and fail if it has a seam, as a debug Rust build does. Rebuild Rust with `just build-ios-release` or `just build-android-release` first. The check's markers are in `tools/release-seams/rust-markers.txt`, which `check-release-seams` reads too.

- **QR scanner** (`COVE_UI_TEST_QR_SCRIPT`): set to a file path. While a scanner is open it takes the file, deletes it, and scans each string of its `{"frames": [...]}` 150 ms apart through the same code the camera feeds, so progress, haptics, errors and completion behave as with a camera. Write the file atomically, since the scanner polls for it. iOS tests use a path under `/tmp`, which the simulator shares with the host, and pass it in the launch environment. Android tests use the app's `noBackupFilesDir` and set the variable in process with `Os.setenv`, which the app reads when a scanner opens
- **TAPSIGNER transport** (`COVE_UI_TEST_TAPSIGNER_BRIDGE`): set to `host:port`. TAPSIGNER commands, and the NFC tag read that finds the card, go over TCP to that address instead of an NFC session, each APDU as a frame with a 4-byte big-endian length. The app's TAPSIGNER code above the transport is unchanged, except that Cove, which accepts only cards whose certificates chain to Coinkite's factory root, also accepts the emulator's root while the variable is set. The emulator's root private key is public, which is why that trust is compiled into debug builds only
- **Text truncation probe** (`COVE_UI_TEST_TRUNCATION_PROBE`, iOS only): set to `1`. Each probed text, such as the TAPSIGNER screen titles, descriptions and button labels, lays out a hidden copy of itself with no line limit at the width it draws at and sets its accessibility value to `fits`, or to `truncated` when the copy is taller. A test reads the value from the text's `value`, or from its button's, which takes its label's value. Without the variable, and in release builds, the probe leaves the text unchanged

### Host services

`cargo xtask ui-test-host run [--android] -- <command>` runs the command with the fake node and the hardware signer host up, forwarding all their ports to Android with `--android`. The xtask iOS and Android UI test runners start it themselves. The signer host serves:

- **Control API on 39010:** `POST /sign-psbt` signs a PSBT with a mnemonic as a hardware wallet would and returns it as base64 and as UR and BBQr frames, `/unrelated-psbt` signs one that matches no draft, `/watch-only-descriptor` returns the account a hardware wallet exports, `/public-key-export` returns an account's bare vpub or upub as a plain QR and a single-part BBQr with the first receive address it should import as, and `/simulator-pasteboard` reads an iOS simulator's pasteboard, so iOS tests read a copied PSBT without the paste prompt. `/tapsigner/start` starts a fresh emulated TAPSIGNER with CVC 123456, set up with a key or, with `{"needs_setup": true}`, factory fresh, and `/tapsigner/drop` ends its next session after a number of exchanges, as a card pulled from the field would
- **TAPSIGNER bridge on 39011:** passes APDUs to Coinkite's `ecard.py` emulator from the rust-cktap checkout `Cargo.lock` pins. The host prepares the emulator when it starts, before any test asks for a card, so a first-time setup doesn't count against a test's request timeout. The emulator runs in the same uv venv `just test-tapsigner-emulator` uses, `~/.cache/cove/tapsigner-emulator-venv`, or in the Python `COVE_ECARD_PYTHON` names. If the host can't prepare the emulator, for example because uv is missing, it prints a warning, and only the TAPSIGNER tests fail. The emulator writes its output to `rust/target/ui-test-host/ecard.log`, which a failed `UI tests` job uploads as a `tapsigner-emulator-log-*` artifact. When a card fails to start, the `/tapsigner/start` error, and so the test's failure message, ends with the last lines of that log. The host logs how long each card took to listen. A card that doesn't listen within 45 seconds gets a SIGABRT first, so the log shows where each Python thread was stuck

To run the tests alone:

```sh
just ios-ui-background "iPhone 17" CoveUITests/HardwareSigningFullLaunchUITests
cd android && ANDROID_SERIAL=<serial> cargo run -q --manifest-path ../rust/Cargo.toml -p xtask -- ui-test-host run --android -- ./gradlew connectedUiTestDebugAndroidTest -Pandroid.testInstrumentationRunnerArguments.class=org.bitcoinppl.cove.flows.SendFlow.HardwareSigningFullLaunchTest
```

Camera and NFC hardware themselves, and real signing devices, still need a person.

## Driving devices

### Android

[`tools/uia2`](../tools/uia2/README.md) drives a phone or emulator through the UiAutomator2 server. `android-vis` prints the visible elements with tap centers, `android-tap-label` taps by test tag or text, and `android-shot` takes a screenshot for visual checks.

- Screens that hold secrets set `FLAG_SECURE`, so screenshots and screen recordings of them come back blank. Never print or save the raw UI tree from `android-source` on those screens: it contains the secret values. For word-layout checks, keep word values in memory and record only indexes and bounds
- Restore any font scale, animation scale or airplane mode change at the end, as the device scripts do

### iOS

[`tools/wda`](../tools/wda/README.md) drives an iPhone through WebDriverAgent with `ios-*` commands that mirror the `android-*` ones: `ios-vis` to read the screen, `ios-tap-label` to tap, `ios-shot` for screenshots. iPhone Mirroring ignores synthetic scrolls and can't run while WDA is driving the phone, so use WDA for automation. The tools README covers the pasteboard, swipe holds, stalls and the other behavior to know.

- **System dialogs** such as Allow Paste and the share sheet aren't in Cove's tree. Tap them by coordinates from a screenshot
- **Stalls are findings:** when tree reads and taps time out while screenshots still work, Cove's main thread is blocked. Record it with a screenshot instead of retrying
- **Never take a screenshot of a recovery-word screen.** For word-layout checks, keep word values in memory and record only indexes and bounds

## Running an agent pass

Give the agent a brief with:

- **Scope:** the row ids from `just qa-plan`, the artifact under test and the devices
- **Hard rules:** test networks only, no mainnet sends without approval, no screenshots of recovery words, never print or save secrets, and no wipe, uninstall, cloud backup deletion or account sign-out without approval. Restore every phone and app setting the run changes, and record the final state
- **Fixtures:** wallet names, which signer to use, the explorer address and anything the brief assumes exists
- **Human hand-offs:** the rows the agent should mark HUMAN instead of attempting
- **Results format:** as below

Ask for a checklist file the agent updates as it goes, and a closing summary that lists what needs a person first.

## Recording results

Record one result per platform for each row, with evidence paths and a timestamped note:

- `PASS`: the whole row was exercised and held. If only part was exercised, it isn't a PASS
- `FAIL`: the app misbehaved. Record the exact input, the screen and the evidence. Check the same case on the other platform before marking it N/A there: much of Cove's behavior is shared Rust code
- `HUMAN`: needs a person, with the reason (camera, NFC, biometrics and so on)
- `BLOCKED`: the run couldn't reach the state the row needs. Name the missing fixture, instrument or approval, so the next run can supply it
- `N/A`: the row doesn't apply to that platform

Keep screenshots, UI trees and logs next to the results, and never include secrets in them.

## Rows that need a person

These still need hands and eyes:

- QR camera scans: PSBT UR and BBQr, SeedQR, KeyTeleport responses, watch-only import. The [hardware signer tests](#hardware-signer-tests) cover the PSBT frames and their import; the camera and a strict external signer still need a person
- NFC: TAPSIGNER setup and signing, NFC import, removing a card mid-operation. The emulator tests cover Cove's card logic; a physical card, the NFC session and card removal still need a person
- Biometric success, and the strength of PIN keypad haptics
- Spoken VoiceOver and TalkBack output and natural focus order
- COLDCARD and other hardware signers
- Passkey creation and approval with a real account when enabling or restoring Cloud Backup
- Cloud Backup flows whose tests stop short of the production path, such as recovering an interrupted setup or a partly finished Restore All after a restart, the inventory refresh after reconnecting, retrying a failed setup upload, verification and Sync Now, and a restored wallet's history after its full scan. Rust tests with scripted passkey and cloud providers cover the cancel, retry, inventory and restore logic underneath, and each row's manifest comment says which part stays manual
- Signing in or out of iCloud or Google accounts

Some neighboring checks don't need a person. KeyTeleport transfers between two Cove phones run end to end through both Rust managers in `rust/src/manager/key_teleport_manager/tests/round_trip.rs`, and COLDCARD payloads (mnemonics, master keys, raw master secrets, notes, Seed Vault entries, backups and PSBTs) are encrypted the way COLDCARD firmware does in `tests/coldcard.rs`, so only the camera scan between devices needs a person. Cove's HTTP client trusts only its bundled webpki roots, not certificates installed on the phone, so an intercepting proxy can't read the diagnostics upload. The tests in `rust/src/diagnostics/upload.rs` send a report to a local collector through `COVE_DIAGNOSTICS_URL` and check the request as it arrives: the `application/age` content type, the key-id header, and an age-encrypted body that contains no report text. Debug builds send reports to `COVE_DIAGNOSTICS_URL` when it is set, so the same check works against a running app. The report is encrypted to the production key, so that collector can't read its contents.

## After a pass

- Give every FAIL a regression test at the cheapest layer that can catch it. Record it in the manifest under `pending` with its pull request, then under `coverage` once it merges
- Turn any agent steps that only navigate and observe into a device script in `qa/device`, so the next release runs them with a command
- Update fixtures and this document with anything the run had to work around
