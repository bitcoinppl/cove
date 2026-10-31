# Testing

Cove is tested in layers. Each row of the release checklist is decided by the cheapest layer that can catch a regression in it, and a release only sends an agent or a person to the rows that nothing cheaper covers. The manifest that records this is described in [qa/README.md](../qa/README.md).

## Table of Contents

- [Layers](#layers)
- [Hardware signer tests](#hardware-signer-tests)
- [Planning a release pass](#planning-a-release-pass)
- [Fixtures](#fixtures)
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
| iOS UI tests | Full-launch flows in the simulator | `just ios-ui-background` runs `CoveUITests/OnboardingFullLaunchUITests`, the class CI runs; pass another class as the second argument, as in `just ios-ui-background "iPhone 17" CoveUITests/<Class>`. `just ios-ui-foreground` shows the simulator |
| Android instrumented tests | Compose screens and full-launch flows on an emulator or device | `just android-instrumented-ci`; staged process-death tests need `just android-ui-manual` |
| Device scripts | Navigate-and-observe checks on a real phone, with JSON results per row | `python3 -I qa/device/run.py android --serial <serial> --package org.bitcoinppl.cove.dev` or `python3 -I qa/device/run.py ios --bundle-id org.bitcoinppl.cove`; see [qa/device/README.md](../qa/device/README.md) for setup, `--list` and running a subset of rows |
| Agent pass | Checklist rows that need judgment, fixtures or multi-app flows, run by a coding agent driving a real phone | This document |
| Person | Camera, NFC, biometrics, haptics, spoken screen-reader output, hardware signers | Batched into one sitting at the end |

UI tests that need a funded wallet get it from the fake Esplora and Electrum node in [`rust/crates/cove-fake-node`](../rust/crates/cove-fake-node/README.md), so they need no Signet funds or network. `just ios-ui-background`, `just ios-ui-foreground`, `just android-ui-manual` and `just android-instrumented-ci` start it on ports 39000 (control API), 39001 (Esplora) and 39002 (Electrum) for the length of the run, and forward the ports to Android with `adb reverse`. With more than one Android device connected, set `ANDROID_SERIAL` to pick one, since tests on two devices would reset and fund the same node. To run another command with the node up, use `just fake-node-run <command>` or `cargo xtask fake-node run -- <command>` from `rust/`.

The CI workflow runs the TAPSIGNER emulator tests in their own job. The `UI tests` workflow runs the iOS UI tests and Android instrumented tests nightly, on pushes to master, and on pull requests labeled `ui-tests`. CI runs the manifest check behind `just qa-check` on pull requests, so the manifest stays consistent with the tests it names. Its workflow filters pull requests to those targeting master, so run `just qa-check` locally before pushing a checklist change on a stacked branch.

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
- Passkey creation, approval and cancel during Cloud Backup setup, verification and restore
- Signing in or out of iCloud or Google accounts

Some neighboring checks don't need a person. KeyTeleport transfers between two Cove phones run end to end through both Rust managers in `rust/src/manager/key_teleport_manager/tests/round_trip.rs`, and COLDCARD payloads (mnemonics, master keys, raw master secrets, notes, Seed Vault entries, backups and PSBTs) are encrypted the way COLDCARD firmware does in `tests/coldcard.rs`, so only the camera scan between devices needs a person. Cove's HTTP client trusts only its bundled webpki roots, not certificates installed on the phone, so an intercepting proxy can't read the diagnostics upload. The wire format is covered by the tests in `rust/src/diagnostics/upload.rs`, and debug builds send reports to `COVE_DIAGNOSTICS_URL` when it is set, so a local server can check that a report arrives and what headers it carries. The report itself is still encrypted to the production key, so that server can't read its contents.

## After a pass

- Give every FAIL a regression test at the cheapest layer that can catch it. Record it in the manifest under `pending` with its pull request, then under `coverage` once it merges
- Turn any agent steps that only navigate and observe into a device script in `qa/device`, so the next release runs them with a command
- Update fixtures and this document with anything the run had to work around
