# Cove Architecture

## TL;DR

- The Rust crate in `rust/` is the single source of truth for wallet logic, networking, persistence, and hardware integrations. BDK is the main library powering all things bitcoin-related.
- SwiftUI (iOS) and Jetpack Compose (Android) UIs talk to the Rust core through “Managers”, lightweight view-models that own the generated FFI objects, subscribe to reconciliation callbacks, and expose platform-friendly state.
- All cross-platform bindings are generated with UniFFI via custom scripts that live in `scripts/` and Just recipes; `just build-ios` and `just build-android` rebuild the Rust core, regenerate bindings, and drop artifacts into the mobile projects.

---

## Table of Contents

- [Rust Core](#rust-core)
- [UniFFI Bindings](#uniffi-bindings)
- [Mobile Frontends](#mobile-frontends)
- [Build & Tooling](#build--tooling)
- [Testing](#testing)
- [Extending the Core](#extending-the-core)
- [Development Workflow](#development-workflow)
- [Quick Pointers](#quick-pointers)

---

## Rust Core

**Layout.** The top-level crate (`rust/src/lib.rs`) re-exports a collection of domain-focused modules (wallets, routing, hardware, fiat, etc.) plus internal crates under `rust/crates/`. Everything compiles into `libcove.{a,so}` and the `coveffi` cdylib specified in `rust/uniffi.toml`.

**Internal crates** (`rust/crates/`):

- `cove-bdk` - BDK wallet functionality wrappers
- `cove-bip39` - BIP39 mnemonic handling
- `cove-common` - Shared constants and utilities
- `cove-device` - Platform abstraction for keychain and device features
- `cove-macros` - Common Macros used by the other crates
- `cove-nfc` - NFC communication protocols
- `cove-tap-card` - TAPSIGNER/SATSCARD integration
- `cove-types` - Shared type definitions
- `cove-util` - General utilities (formatting, logging, result extensions)
- `uniffi_cli` - Custom UniFFI CLI wrapper for binding generation

**Async runtime.** The core is async-first: a Tokio runtime is initialised from the host app (`task::init_tokio`) and reused via a global `OnceLock`. Host apps must invoke `FfiApp::init_on_start` once after creating the app object (see the `.task { await app.rust.initOnStart() }` block in `ios/Cove/CoveApp.swift`) before dispatching actions so background tasks and caches spin up correctly.

**Actor system.** Long-lived concurrent components use `act-zero` actors spawned onto the shared Tokio runtime. Use `task::spawn_actor()` to create actors.

Key actors include:

- `WalletActor` (`rust/src/manager/wallet_manager/actor.rs`) - Manages wallet state and operations
- `WalletScanner` - Handles blockchain scanning and syncing

Actors are ideal for components that need to process messages sequentially, maintain internal state, and send reconciliation updates back to the UI when work completes.

**Singleton pattern.** Many core components use singleton patterns for global access, implemented via `OnceLock`, `LazyLock`, or `ArcSwap`. Creating new instances returns cheap clones (typically `Arc` clones) of the global singleton, similar to how `AppManager.shared` works on iOS or `AppManager.getInstance()` on Android. Key singletons include:

- `Database::global()` (`rust/src/database.rs`) - Database access via `OnceCell<ArcSwap<Database>>`. Calling `Database()` from Kotlin/Swift returns an `Arc` clone of the global instance.
- `App::global()` (`rust/src/app.rs`) - Application state and routing coordinator via `OnceCell<App>`.
- `FfiApp::global()` (`rust/src/app.rs`) - FFI wrapper for app, always returns a new `Arc<Self>` that accesses `App::global()`.
- `AUTH_MANAGER` (`rust/src/manager/auth_manager.rs`) - Authentication manager via `LazyLock<Arc<RustAuthManager>>`.
- `Keychain::global()` (`rust/crates/cove-device/src/keychain.rs`) - Platform keychain access via `OnceCell`, initialized once by the host app.
- `Device::global()` (`rust/crates/cove-device/src/device.rs`) - Device capabilities access via `OnceCell`, initialized once by the host app.
- `FIAT_CLIENT` (`rust/src/fiat/client.rs`) - Price fetching client via `LazyLock<FiatClient>`.
- `FEE_CLIENT` (`rust/src/fee_client.rs`) - Fee estimation client via `LazyLock<FeeClient>`.
- `PRICES` & `FEES` - Thread-safe cached data via `LazyLock<ArcSwap<Option<T>>>` for lock-free reads with atomic updates.

This pattern is used throughout the codebase for shared resources and is safe to use from any thread. Platform code mirrors this with `AppManager.shared` (iOS) and `AppManager.getInstance()` (Android).

**State reconciliation.** Each manager module owns a `flume` channel pair. Rust emits typed `…ReconcileMessage` enums through the channel, and the generated FFI surface forwards them to the platform reconcilers. Platform managers should call `listen_for_updates` immediately after instantiating their Rust counterpart (e.g. `AppManager` in `ios/Cove/AppManager.swift`) so no reconciliation messages are missed. Long-lived managers (wallet, send flow, auth) also keep shared state in `Arc<RwLock<_>>` structures so the reconciler can request snapshots.

**Routing & application shell.** `rust/src/app.rs` defines `App`, the singleton that coordinates routing, fees/prices, network selection, and terms acceptance. Its `FfiApp` wrapper implements the UniFFI object exposed to the UI. Route updates use `AppStateReconcileMessage` callbacks to keep Kotlin/Swift state in sync.

**Persistence.** Non-sensitive data is stored with [`redb`](https://crates.io/crates/redb) (`rust/src/database.rs`). Database file defaults to `$ROOT_DATA_DIR/cove.db` where `ROOT_DATA_DIR` is defined in `cove-common/src/consts.rs`.

**Database tables:**

- `GlobalFlagTable` - Feature flags and terms acceptance status
- `GlobalConfigTable` - Network selection, node configuration, color scheme preferences
- `GlobalCacheTable` - Cached data for performance optimization
- `WalletsTable` - Wallet metadata and configuration
- `UnsignedTransactionsTable` - Pending transactions awaiting signature
- `HistoricalPriceTable` - Historical price data for fiat conversions

**Keychain.** Secrets are stored in OS-native keychains via the `KeychainAccess` trait (`rust/crates/cove-device/src/keychain.rs`). This is a UniFFI callback interface that platform code implements - iOS provides the implementation using the iOS Keychain, and Android uses Android KeyStore. Rust code accesses it via `Keychain::global()`, which internally delegates to the platform implementation. The keychain supports encryption/decryption through the `Cryptor` from `cove-util`.

**Wallet & hardware integrations.** BDK powers transaction management (`rust/src/wallet`, `rust/src/transaction`). TAPSIGNER/SATSCARD + NFC flows live in `rust/src/tap_card` and the dedicated crates under `rust/crates/`. The utilities crate (`cove-util`) concentrates helpers such as result extensions, formatting, and logging.

---

## UniFFI Bindings

- `rust/uniffi.toml` names the shared library (`coveffi`) and the Kotlin package (`org.bitcoinppl.cove_core`).
- `cargo run -p uniffi_cli` invokes the custom CLI wrapper that ships with this repo (`rust/crates/uniffi_cli`). The CLI understands both Swift and Kotlin targets and emits consistent module names (`cove_core_ffi`, `cove.kt`).
- **Binding generation flow:**
  - First, bindings are generated into `rust/bindings/` (intermediate, temporary location)
  - Swift: Copied from `rust/bindings/*.swift` → `ios/CoveCore/Sources/CoveCore/generated/`
  - Kotlin: Copied from `rust/bindings/kotlin/` → `android/app/src/main/java/org/bitcoinppl/cove_core/`
  - The build scripts (`scripts/build-ios.sh`, `scripts/build-android.sh`) handle this copying automatically

**Android-specific notes:**

- UniFFI automatically transforms Rust error types ending in `Error` to `Exception` when generating Kotlin bindings (e.g., `SendFlowError` becomes `SendFlowException`). This is standard Kotlin convention where exceptions extend `kotlin.Exception`.
- Rust enum variants use **tuple-style** (unnamed fields), which UniFFI translates to generic `v1`, `v2`, `v3` field names in Kotlin (e.g., `RouteUpdated(Vec<Route>)` becomes `data class RouteUpdated(val v1: List<Route>)`). In contrast, struct-style variants with named fields preserve those names (e.g., `WrongNetwork { address: String, validFor: Network, current: Network }` becomes `data class WrongNetwork(val address: String, val validFor: Network, val current: Network)`).
- When you change any exported API (new method, enum, record), rebuild bindings through the `just` recipes described below so the mobile projects pick up the new code.

---

## Mobile Frontends

> We aim for shared structure and terminology across platforms (same manager names, reconcile shapes, etc.) while still embracing each platform's native idioms for UI, navigation, typography, and interactions. Think "consistent architecture, platform-native experience."

**Platform-specific UI examples:**
- Settings rows: iOS favors inset grouped lists with `NavigationLink` chevrons, while Android uses full-width Material list items with ripple feedback and trailing metadata icons instead of chevrons.
- Toggles: SwiftUI `Toggle` mirrors the iOS switch with a circular thumb and elastic animation; Compose uses `Switch` with a rectangular track, Material colors, and larger touch ripples.
- Typography: iOS leans on SF Pro text styles (Title, Body, Footnote) and tighter letter spacing; Android uses Material 3 `TitleLarge`, `BodyMedium`, etc., aligning baseline grids to 4dp spacing.
- Background treatments: iOS often uses blurred/grouped surfaces floating above a tinted system background; Android prefers flat `colorSurface` backgrounds with tonal elevation for cards or sections so dynamic color and dark theme transitions stay consistent.

### iOS (SwiftUI)

- The Swift Package `ios/CoveCore` wraps the generated bindings. `scripts/build-ios.sh` creates an XCFramework (`cove_core_ffi.xcframework`) and deposits generated Swift sources into the package.
- `AppManager` (`ios/Cove/AppManager.swift`) is a singleton `@Observable` class that owns `FfiApp`, manages routing, and lazily instantiates other managers (wallet, send flow, etc.). Each manager wraps its `Rust…Manager` counterpart, registers itself as a reconciler, and updates SwiftUI-observable state on the main actor.
- UI modules inject managers with `@Environment` or direct initialisers and call `dispatch(...)` for Rust-side actions. Reconcilers run updates on the main actor to keep SwiftUI safe.

### Android (Jetpack Compose)

- Compose screens obtain managers via `remember { ImportWalletManager() }` or DI and interact with them the same way SwiftUI does. The generated bindings (`android/app/src/main/java/org/bitcoinppl/cove_core/cove.kt`) expose suspending functions and listener hooks.
- Each Kotlin manager implements the generated `FfiReconcile` interface and creates its own lifecycle-aware coroutine scope (`Dispatchers.Main` + `SupervisorJob`). In `init`, managers create their Rust counterpart, call `listenForUpdates(this)`, and implement `reconcile(...)` to update Compose state or emit side-effects.
- Shared navigation mirrors the Rust router: `RouterManager` listens for `RouteUpdated` events from the core and reconciles Compose navigation stacks.

**State management patterns:**

- **Use callbacks, not MutableState parameters**: While iOS uses `@Binding` extensively for two-way state binding, Android/Compose follows the "state down, events up" pattern with callbacks. Composables should accept value parameters (e.g., `value: String`) and callback parameters (e.g., `onValueChange: (String) -> Unit`), never `MutableState<T>` parameters.
- **Why callbacks?** This follows official Android guidelines, matches standard library components (`TextField`, `Switch`, etc.), maintains unidirectional data flow, and makes previews easier. The codebase uses callbacks in 99% of components.
- **Bidirectional sync**: When a child component needs to both read and write parent state, use `LaunchedEffect(parentValue) { childValue = parentValue }` to sync changes from parent to child, and callbacks to notify parent of child changes. While this creates boilerplate (~8 lines per field), it's the idiomatic Android pattern.
- **Example pattern**:
  ```kotlin
  @Composable
  fun MyComponent(
      value: String,              // state down
      onValueChange: (String) -> Unit,  // events up
  ) {
      var localValue by remember { mutableStateOf(value) }
      LaunchedEffect(value) { localValue = value }  // sync parent → child

      TextField(
          value = localValue,
          onValueChange = {
              localValue = it
              onValueChange(it)  // notify parent
          }
      )
  }
  ```

**Compose ↔ iOS parity patterns:** For detailed guidance on matching iOS behavior in Compose (opacity, text colors, button centering, AutoSizeText, etc.), see [docs/COMPOSE_IOS_PARITY.md](docs/COMPOSE_IOS_PARITY.md).

### Manager Pattern (cross-platform)

1. UI calls `manager.dispatch(action)` or a helper method (e.g. `importWallet`).
2. The Swift/Kotlin manager forwards the call to `Rust…Manager` through the generated bindings.
3. Rust mutates state, enqueues reconciliation messages, and optionally writes to redb or the keychain.
4. The reconcilers on the UI thread receive the message and update observable state, triggering re-render.

This pattern keeps business logic and validation centralized in Rust while giving each platform idiomatic state containers.

---

## Build & Tooling

**Build commands:**

- `just build-ios [profile] [--device] [--sign]` → Runs `scripts/build-ios.sh`, builds the staticlib for requested targets (aarch64-apple-ios, aarch64-apple-ios-sim), regenerates Swift bindings, and produces `cove_core_ffi.xcframework`
- `just build-android [debug|release]` → Runs `scripts/build-android.sh`, cross-compiles the cdylib with cargo-ndk for Android targets (aarch64-linux-android, x86_64-linux-android), generates Kotlin bindings, and copies them into `android/app/src/main/java/`
- `just compile` → Compiles both iOS and Android without full FFI rebuild
- `just compile-ios` / `just compile-android` → Platform-specific compilation without FFI rebuild

**Development commands:**

- `just bacon` → Runs continuous clippy checking with interactive output
- `just bcheck` → Bacon in check mode for faster iteration
- `just check` → Runs `cargo check` with appropriate flags
- `just fix` → Runs `cargo fix --allow-dirty` to automatically fix warnings
- `just update` → Updates Cargo dependencies

**Format & lint:**

- `just fmt` → Enforces Rust, Swift (`swiftformat`), and Kotlin (`ktlint`) formatting across all codebases

**iOS-specific:**

- `just xcode-clean` → Cleans Xcode derived data
- `just xcode-reset` → Full Xcode cache reset (useful when builds get stuck)
- `just run-ios` → Builds and runs the iOS app

**Android-specific:**

- `just run-android` → Builds and runs the Android app on connected device/emulator

**Cleanup:**

- `just clean` → Full cleanup of build artifacts across all platforms

Because binding generation is deterministic, always rerun the appropriate Just target after touching exported Rust APIs so the mobile projects stay in sync.

---

## Testing

- `just test` runs the full test suite via `cargo nextest run --workspace` across all Rust crates
- `just wtest` (or `just watch-test`) runs tests in watch mode, automatically re-running when files change
- `just btest` uses `bacon` for continuous test monitoring with interactive output
- `just ctest` runs standard `cargo test` for cases where nextest isn't needed
- Tests run on both Ubuntu and macOS in CI (`.github/workflows/ci.yml`)
- The CI pipeline includes separate jobs for: rustfmt, swiftformat, ktlint, clippy, test, compile-android, and compile-ios
- Run `just ci` locally to execute the same checks that run in CI before pushing

---

## Extending the Core

- **New manager / feature flow:** Create a Rust manager module under `rust/src/manager/`, define its state, actions, and reconcile messages, and export it with `#[uniffi::export]`. Implement the matching Swift/Kotlin manager classes that conform to the generated `…Reconciler` protocol/interface.
- **Routing additions:** Extend `rust/src/router.rs` enums, expose necessary helpers on `FfiApp`, and update the `RouterManager` on both platforms to handle the new routes.
- **Database schema changes:** Update the relevant table builders under `rust/src/database/`. Because redb uses typed tables, add migration logic or regenerate tables as needed, and expose read/write helpers through UniFFI.
- **Async work:** Prefer spawning onto the shared Tokio runtime via `task::spawn`. If the work belongs to a long-lived component, consider using an `act-zero` actor so you can push reconciliation messages back to the UI on completion.

---

## Development Workflow

**Initial setup:**

1. Clone the repository and ensure Rust toolchain is installed
2. Install platform-specific tools: Xcode for iOS, Android Studio + NDK for Android
3. Install Just: `cargo install just`
4. Run `just fmt` to verify formatting tools are installed

**Iterative development:**

- For Rust-only changes: Use `just bacon` or `just bcheck` for continuous feedback while coding
- For UI changes without Rust API changes: Use `just compile-ios` or `just compile-android` for faster iteration
- For changes to exported Rust APIs: Run `just build-ios` or `just build-android` to regenerate bindings
- Run `just wtest` in a separate terminal for continuous test feedback

**Before committing:**

1. Run `just ci` to execute all checks locally (format, lint, clippy, tests, compilation)
2. Fix any issues reported by the CI checks
3. If clippy reports warnings, run `just fix` first to auto-fix what's possible

**Debugging tips:**

- iOS builds stuck? Try `just xcode-reset` to clear Xcode caches
- Clean slate needed? Run `just clean` to remove all build artifacts
- UniFFI binding issues? Check that you regenerated bindings after changing Rust exports
- Actor not receiving messages? Verify you called `FfiApp::init_on_start` during app startup

---

## Quick Pointers

- Rust exports live in `rust/src/**` and the supporting crates under `rust/crates/`.
- Swift bindings land in `ios/CoveCore/Sources/CoveCore/generated/`; Kotlin bindings live (after copy) under `android/app/src/main/java/org/bitcoinppl/cove_core/`.
- Database file defaults to `$ROOT_DATA_DIR/cove.db` (see `cove_common::consts::ROOT_DATA_DIR`).
- Hardware / NFC helpers: `rust/crates/cove-device`, `rust/src/tap_card/`, and platform shims in `ios/Cove/FFI/` plus Android's `org.bitcoinppl.cove_core` package.
