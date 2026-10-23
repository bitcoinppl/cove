# Testing

Cove is tested in layers. Each row of the release checklist is decided by the cheapest layer that can catch a regression in it, and a release only sends an agent or a person to the rows that nothing cheaper covers. The manifest that records this is described in [qa/README.md](../qa/README.md).

## Table of Contents

- [Layers](#layers)
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
| Rust tests | Core behavior shared by both apps: wallets, send flow, labels, KeyTeleport, cloud backup, diagnostics, node and explorer settings | `just test` |
| iOS UI tests | Full-launch flows in the simulator | `just ios-ui-background` runs `CoveUITests/OnboardingFullLaunchUITests`, the class CI runs; pass another class as the second argument, as in `just ios-ui-background "iPhone 17" CoveUITests/<Class>`. `just ios-ui-foreground` shows the simulator |
| Android instrumented tests | Compose screens and full-launch flows on an emulator or device | `just android-instrumented-ci`; staged process-death tests need `just android-ui-manual` |
| Device scripts | Navigate-and-observe checks on a real phone, with JSON results per row | `python3 -I qa/device/run.py`, see [qa/device/README.md](../qa/device/README.md) |
| Agent pass | Checklist rows that need judgment, fixtures or multi-app flows, run by a coding agent driving a real phone | This document |
| Person | Camera, NFC, biometrics, haptics, spoken screen-reader output, hardware signers | Batched into one sitting at the end |

The `UI tests` workflow runs the iOS UI tests and Android instrumented tests nightly, on pushes to master, and on pull requests labeled `ui-tests`. CI runs the manifest check behind `just qa-check` on every pull request, so the manifest stays consistent with the tests it names.

## Planning a release pass

1. **Identify the artifact.** Record the version, build number and commit from Settings → About on each phone. Plan against that commit, not the local branch: a TestFlight or Play build can lag the branch it was cut from.
2. **List the rows.** `just qa-plan <last-tested-commit>...<release-commit>` lists the rows whose code changed and that no automated test decides, split into agent and human rows. Use `just qa-plan-all` for a full pass.
3. **Run the automated layers first.** Green CI and a device-script run on each phone remove rows from the agent's list.
4. **Hand the agent rows to an agent per phone.** One agent per device at a time; WebDriverAgent serves every client on one session, so two drivers interfere.
5. **Batch the human rows** into one sitting after the agents finish, with the fixtures ready.

## Fixtures

Use test networks only. Cove's default Signet node is MutinyNet, whose 30-second blocks make funding fast. Never use production seed material, and keep fixture secrets out of the repository.

- **Two funded Signet wallets** (for example Main and Secondary) with several UTXOs, including one small output, and known history
- **A watch-only hardware wallet** backed by a software signer such as Signer Lab, so the PSBT loop runs without a camera: export the unsigned PSBT to a file, review and sign it with the signer's CLI, then import the signed file in Cove. For QR and file fixtures, `just sign-psbt` signs a PSBT with the test mnemonic in `$MNEMONIC` and writes the result as base64, hex, binary, BBQr and UR images
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

- QR camera scans: PSBT UR and BBQr, SeedQR, KeyTeleport responses, watch-only import
- NFC: TAPSIGNER setup and signing, NFC import, removing a card mid-operation
- Biometric success, and the strength of PIN keypad haptics
- Spoken VoiceOver and TalkBack output and natural focus order
- COLDCARD and other hardware signers
- Passkey creation, approval and cancel during Cloud Backup setup, verification and restore
- Signing in or out of iCloud or Google accounts

Some neighboring checks don't need a person. Cove's HTTP client trusts only its bundled webpki roots, not certificates installed on the phone, so an intercepting proxy can't read the diagnostics upload. The wire format is covered by the tests in `rust/src/diagnostics/upload.rs`, and debug builds send reports to `COVE_DIAGNOSTICS_URL` when it is set, so a local server can inspect them.

## After a pass

- Give every FAIL a regression test at the cheapest layer that can catch it. Record it in the manifest under `pending` with its pull request, then under `coverage` once it merges
- Turn any agent steps that only navigate and observe into a device script in `qa/device`, so the next release runs them with a command
- Update fixtures and this document with anything the run had to work around
