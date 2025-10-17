# Cove Architecture

## TL;DR

- The Rust crate in `rust/` is the single source of truth for wallet logic, networking, persistence, and hardware integrations. It wraps BDK, redb, NFC, and TapSigner support behind a Uniffi-generated API surface.
- SwiftUI (iOS) and Jetpack Compose (Android) UIs talk to the Rust core through “Managers”, lightweight view-models that own the generated FFI objects, subscribe to reconciliation callbacks, and expose platform-friendly state.
- All cross-platform bindings are generated with Uniffi via custom scripts that live in `scripts/` and Just recipes; `just build-ios` and `just build-android` rebuild the Rust core, regenerate bindings, and drop artifacts into the mobile projects.

---

## Rust Core

**Layout.** The top-level crate (`rust/src/lib.rs`) re-exports a collection of domain-focused modules (wallets, routing, hardware, fiat, etc.) plus internal crates under `rust/crates/` (`cove-bdk`, `cove-types`, `cove-util`, …). Everything compiles into `libcove.{a,so}` and the `coveffi` cdylib specified in `rust/uniffi.toml`.

**Async runtime.** The core is async-first: a Tokio runtime is initialised from the host app (`task::init_tokio`) and reused via a global `OnceLock`. Host apps must invoke `FfiApp::init_on_start` once after creating the app object (see the `.task { await app.rust.initOnStart() }` block in `ios/Cove/CoveApp.swift`) before dispatching actions so background tasks and caches spin up correctly. Workloads that benefit from actors (wallet scanning, fee refresh, etc.) use `act-zero` actors spawned onto that runtime (`rust/src/manager/wallet_manager.rs`).

**State reconciliation.** Each manager module owns a `flume` channel pair. Rust emits typed `…ReconcileMessage` enums through the channel, and the generated FFI surface forwards them to the platform reconcilers. Platform managers should call `listen_for_updates` immediately after instantiating their Rust counterpart (e.g. `AppManager` in `ios/Cove/AppManager.swift`) so no reconciliation messages are missed. Long-lived managers (wallet, send flow, auth) also keep shared state in `Arc<RwLock<_>>` structures so the reconciler can request snapshots.

**Routing & application shell.** `rust/src/app.rs` defines `App`, the singleton that coordinates routing, fees/prices, network selection, and terms acceptance. Its `FfiApp` wrapper implements the Uniffi object exposed to the UI. Route updates use `AppStateReconcileMessage` callbacks to keep Kotlin/Swift state in sync.

**Persistence.** Non-sensitive data is stored with [`redb`](https://crates.io/crates/redb) (`rust/src/database.rs`). The wrapper builds typed tables for global config, wallets, unsigned transactions, and cached prices. Secrets remain in the OS keychains (`Keychain::global()` on the Rust side). If you ever see “redwood” in older docs, that should read “redb”.

**Wallet & hardware integrations.** BDK powers transaction management (`rust/src/wallet`, `rust/src/transaction`). TapSigner + NFC flows live in `rust/src/tap_card` and the dedicated crates under `rust/crates/`. The utilities crate (`cove-util`) concentrates helpers such as result extensions, formatting, and logging.

---

## Uniffi Bindings

- `rust/uniffi.toml` names the shared library (`coveffi`) and the Kotlin package (`org.bitcoinppl.cove`).
- `cargo run -p uniffi_cli` invokes the custom CLI wrapper that ships with this repo (`rust/crates/uniffi_cli`). The CLI understands both Swift and Kotlin targets and emits consistent module names (`cove_core_ffi`, `cove.kt`).
- Generated Swift lives in `rust/bindings/*.swift` and is copied into `ios/CoveCore/Sources/CoveCore/generated/`. Kotlin lives in `rust/bindings/kotlin/` before being copied into `android/app/src/main/java/org/bitcoinppl/cove/`.
- `scripts/strip_ffi_duplicates.py` trims redundant scaffolding from the per-crate Kotlin files so only one copy of the FFI helpers ships with the app.

When you change any exported API (new method, enum, record), rebuild bindings through the Just recipes described below so the mobile projects pick up the new code.

---

## Mobile Frontends

### iOS (SwiftUI)

- The Swift Package `ios/CoveCore` wraps the generated bindings. `scripts/build-ios.sh` creates an XCFramework (`cove_core_ffi.xcframework`) and deposits generated Swift sources into the package.
- `AppManager` (`ios/Cove/AppManager.swift`) is a singleton `@Observable` class that owns `FfiApp`, manages routing, and lazily instantiates other managers (wallet, send flow, etc.). Each manager wraps its `Rust…Manager` counterpart, registers itself as a reconciler, and updates SwiftUI-observable state on the main actor.
- UI modules inject managers with `@Environment` or direct initialisers and call `dispatch(...)` for Rust-side actions. Reconcilers run updates on the main actor to keep SwiftUI safe.

### Android (Jetpack Compose)

- Compose screens obtain managers via `remember { ImportWalletManager() }` or DI and interact with them the same way SwiftUI does. The generated bindings (`android/app/src/main/java/org/bitcoinppl/cove/cove.kt`) expose suspending functions and listener hooks.
- Each Kotlin manager extends `BaseManager` when it needs lifecycle-aware coroutine scopes (`Dispatchers.Main` + `SupervisorJob`). In `init`, managers create their Rust counterpart, call `listenForUpdates(this)`, and implement `reconcile(...)` to update Compose state or emit side-effects.
- Shared navigation mirrors the Rust router: `RouterManager` listens for `RouteUpdated` events from the core and reconciles Compose navigation stacks.

### Manager Pattern (cross-platform)

1. UI calls `manager.dispatch(action)` or a helper method (e.g. `importWallet`).
2. The Swift/Kotlin manager forwards the call to `Rust…Manager` through the generated bindings.
3. Rust mutates state, enqueues reconciliation messages, and optionally writes to redb or the keychain.
4. The reconcilers on the UI thread receive the message and update observable state, triggering re-render.

This pattern keeps business logic and validation centralized in Rust while giving each platform idiomatic state containers.

---

## Build & Tooling

- `just build-ios [profile] [--device] [--sign]` → Runs `scripts/build-ios.sh`, builds the staticlib for requested targets, regenerates Swift bindings, and produces `cove_core_ffi.xcframework`.
- `just build-android [debug|release]` → Runs `scripts/build-android.sh`, cross-compiles the cdylib with cargo-ndk, generates Kotlin bindings, and copies them into `android/app/src/main/java/`.
- `just test` wraps `cargo nextest run --workspace`. For iterative development you can use `just wtest` to watch-run tests and `just bacon` to run `bacon` (continuous `cargo` tasks).
- `just fmt` enforces Rust, Swift (`swiftformat`), and Kotlin (`ktlint`) formatting.
- The `just` file contains additional helpers for Xcode maintenance, Android run targets, and CI gatekeeping (`just ci`).

Because binding generation is deterministic, always rerun the appropriate Just target after touching exported Rust APIs so the mobile projects stay in sync.

---

## Extending the Core

- **New manager / feature flow:** Create a Rust manager module under `rust/src/manager/`, define its state, actions, and reconcile messages, and export it with `#[uniffi::export]`. Implement the matching Swift/Kotlin manager classes that conform to the generated `…Reconciler` protocol/interface.
- **Routing additions:** Extend `rust/src/router.rs` enums, expose necessary helpers on `FfiApp`, and update the `RouterManager` on both platforms to handle the new routes.
- **Database schema changes:** Update the relevant table builders under `rust/src/database/`. Because redb uses typed tables, add migration logic or regenerate tables as needed, and expose read/write helpers through Uniffi.
- **Async work:** Prefer spawning onto the shared Tokio runtime via `task::spawn`. If the work belongs to a long-lived component, consider using an `act-zero` actor so you can push reconciliation messages back to the UI on completion.

---

## Quick Pointers

- Rust exports live in `rust/src/**` and the supporting crates under `rust/crates/`.
- Swift bindings land in `ios/CoveCore/Sources/CoveCore/generated/`; Kotlin bindings live (after copy) under `android/app/src/main/java/org/bitcoinppl/cove/`.
- Database file defaults to `$ROOT_DATA_DIR/cove.db` (see `cove_common::consts::ROOT_DATA_DIR`).
- Hardware / NFC helpers: `rust/crates/cove-device`, `rust/src/tap_card/`, and platform shims in `ios/Cove/FFI/` plus Android’s `org.bitcoinppl.cove` package.
