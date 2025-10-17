# TODO Plan

## Completed Items

### 1. **Bootstrap Kotlin App Shell and Core Managers** ✅ COMPLETED

**Implementation Summary:**
- ✅ Created `BaseManager.kt` - Foundation class with coroutine scope and lifecycle management
- ✅ Created `RouterManager.kt` - Wrapper around FFI Router with Compose state management
- ✅ Created `RouteHelpers.kt` - Extension functions for RouteFactory (integrated in RouterManager.kt)
- ✅ Created `AppManager.kt` - Central singleton managing app state, router, prices, fees, and cached managers
- ✅ Created `AuthManager.kt` - Singleton managing lock state, PIN validation, and decoy/wipe flows
- ✅ Created `TaggedItem.kt` - Generic wrapper for identifiable items (alerts/sheets)
- ✅ Created `AppAlertState.kt` - Sealed class hierarchy for all app-level alerts
- ✅ Created `AppSheetState.kt` - Sealed class hierarchy for global bottom sheets
- ✅ Created placeholder `WalletManager.kt` and `SendFlowManager.kt` (full implementation in phase 2)
- ✅ Updated `ViewModel.kt` - Fixed package and renamed to `Manager` to match Swift conventions
- ✅ Created `CoveApp.kt` - Root Compose application with auth/terms/loading/navigation flow
- ✅ Created `RouteView.kt` - Route-to-screen mapper with placeholders for all route types
- ✅ Updated `MainActivity.kt` - Cleaned up to use new CoveApp shell

**Key Architectural Decisions:**
1. **Singleton Pattern**: Used `object` declaration pattern with double-checked locking for AppManager and AuthManager
2. **State Management**: Used Compose `mutableStateOf()` for observable properties instead of Swift's `@Observable`
3. **Reconciliation**: Followed existing `ImportWalletManager` pattern - managers implement reconciler interfaces
4. **Coroutines**: Used Kotlin coroutines with `Dispatchers.Main` and `Dispatchers.IO` instead of Swift's DispatchQueue
5. **Naming**: Kept "Manager" suffix for ViewModels to match Swift naming conventions per user request
6. **Memory Management**: No weak references needed in Kotlin (GC handles it), but avoided circular refs
7. **Router State**: Created `RouterManager` wrapper to make FFI Router observable in Compose with `mutableStateOf`

**Deviations from iOS:**
1. No `nfcReader`/`nfcWriter` in AppManager yet (will add when implementing NFC flows)
2. Used sealed classes instead of enums for alert/sheet states (more idiomatic Kotlin)
3. Global accessors `App` and `Auth` instead of `.shared` static property
4. `viewModelScope` from AndroidX lifecycle instead of custom scope management

**Files Created (13 total):**
- `BaseManager.kt` - 39 lines
- `RouterManager.kt` - 140 lines
- `TaggedItem.kt` - 18 lines
- `AppAlertState.kt` - 74 lines
- `AppSheetState.kt` - 11 lines
- `AppManager.kt` - 319 lines
- `AuthManager.kt` - 220 lines
- `WalletManager.kt` - 24 lines (placeholder)
- `SendFlowManager.kt` - 28 lines (placeholder)
- `CoveApp.kt` - 144 lines
- `RouteView.kt` - 120 lines

**Files Modified (2 total):**
- `ViewModel.kt` - Updated package and renamed to `Manager`
- `MainActivity.kt` - Simplified to use CoveApp()

**Lessons Learned:**
1. FFI bindings are complete and working - `RouteFactory`, `FfiApp`, `RustAuthManager`, etc. all available
2. Kotlin sealed classes are excellent for modeling Swift enums with associated values
3. Compose's `key()` parameter is crucial for forcing recomposition on `routeId` changes
4. `remember { getInstance() }` ensures singleton managers survive recomposition
5. Need to be careful with `GlobalScope` - only use for fire-and-forget operations, otherwise use proper coroutine scope

**Follow-up Items:**
- Phase 2 will implement full `WalletManager` and `SendFlowManager`
- Need to implement actual Lock/Terms screens (currently placeholders)
- Camera/NFC permissions not yet requested in MainActivity
- Sheet content rendering not yet implemented (just clears state)
- Most RouteView screens are placeholders - will wire in phase 4

## TODO Items

1. **Bootstrap Kotlin App Shell and Core Managers** ✅
   - Port `AppManager` from `ios/Cove/AppManager.swift`, mirroring its responsibilities: hold on to the shared `FfiApp`, cache the Rust-driven `Router` (default + stack), `prices`, `fees`, expose `alertState`/`sheetState`, manage `routeId` resets, and implement helpers such as `pushRoute`, `pushRoutes`, `resetRoute`, `loadAndReset`, `scanQr`, and `getWalletManager`. Kotlin also needs to lazily memoize `WalletManager`/`SendFlowManager` instances (see Swift `getWalletManager` / `getSendFlowManager`) and clear them on reset. Ensure the Kotlin `Router` wrapper stays in sync with reconcile messages (`AppStateReconcileMessage.routeUpdated`, `.defaultRouteChanged`, `.pushedRoute`) and propagates changes to Compose via immutable snapshots.
   - Wrap the generated `Route`/`RouteFactory` types (`android/app/src/main/java/org/bitcoinppl/cove/cove.kt:31758+` & `15507+`) with friendly Kotlin helpers so navigation calls mirror Swift usage (e.g., `RouteFactory().newWalletSelect()`, `RouteFactory().nestedWalletSettings(id)`); define equality/hash helpers similar to Swift’s `RouteFactory.isSameParentRoute`.
   - Implement Kotlin `AuthManager` based on `ios/Cove/AuthManager.swift` so Android respects lock-state, decoy/wipe pins, biometric toggles, and can trigger the same app reset flow (`AppManager.reset()`, load `RouteFactory().newWalletSelect()` when appropriate). Ensure it listens to `RustAuthManager` reconcile messages for auth type and wipe/decoy pin toggles.
   - Replace the placeholder `ViewModel.kt` with a lifecycle-aware base (e.g., using `CoroutineScope` + `Job`) that managers can extend to subscribe to Rust callbacks and guarantee `dispose()` calls, similar to the Swift pattern using `WeakReconciler`.
   - Rebuild `CoveApp` for Compose to mirror Swift’s `CoveApp.swift`: set up root state for `AppManager` and `AuthManager`, show `CoverView`/`LockView` analogs until terms are accepted and auth is satisfied, and mount a navigation host that renders the new Kotlin `RouteView`. Hook alerts and sheets by observing `AppManager.alertState`/`sheetState`, respect `AppManager.routeId` when recomposing, and call `ffiApp.initOnStart()` on launch.
   - Update `MainActivity` to initialize the new Compose root, manage `EdgeToEdge`, request camera/NFC permissions where necessary, and drop the current hard-coded `ImportWalletScreen` placeholder.

2. Setup navigation - See `navigation_plan.md` for the consolidated navigation plan.

3. **Implement Kotlin Counterparts for Wallet & Send Managers**
   - Port `WalletManager` from `ios/Cove/WalletManager.swift`, exposing real-time `walletMetadata`, `balance`, `fiatBalance`, `loadState`, `unsignedTransactions`, `foundAddresses`, `transactionDetails`, and `sendFlowErrorAlert`. Implement reconciliation handling for all `WalletManagerReconcileMessage` cases, including fiat updates triggered from `AppManager` on currency change.
   - Port `SendFlowManager` and `SendFlowPresenter` to Kotlin so Android can drive the entire send flow: track entering amounts/addresses, selected fee rate, fee options, presenter focus/alerts/sheets, and dispatch validations (`notifyEnteringAddressChanged`, `notifyAmountChanged`, `finalizeAndGoToNextScreen`, etc.).
   - Implement `CoinControlManager` and `PendingWalletManager` by porting the Swift files (`ios/CoinControlManager.swift`, `ios/Flows/NewWalletFlow/PendingWalletViewModel.swift`) to manage UTXO selection, search/sort bindings, `setCoinControlMode` synchronization with `SendFlowManager`, and pre-generated mnemonic words for the hot-wallet flow.
   - Add Kotlin helpers around other Rust bridges referenced in Swift managers: `LabelManager` for label import/export, `TapSignerNFC` wrappers, `Database()` access for wallet lists and global config, and `NFCReader`/`NFCWriter` analogs (or stubs if platform-specific work is deferred).

4. **Wire Compose Screens to Real Managers and Routes**
   - Replace mock data in Compose screens with live manager state: `wallet_transactions/WalletTransactionsScreen.kt`, `transaction_details/TransactionDetailsScreen.kt`, `send/SendScreen.kt`, `send/send_confirmation/SendConfirmationScreen.kt`, `send/advanced_details/AdvancedDetailsBottomSheet.kt`, `send/network_fee/NetworkFeeBottomSheet.kt`, and `utxo_list/UtxoListScreen.kt` should all consume their corresponding Kotlin managers and emit actions via `dispatch`/`navigate`.
   - Build a Kotlin `RouteView` (and supporting `RememberedRouteState`) that matches `ios/Cove/RouteView.swift`: handle `.loadAndReset`, `.settings`, `.listWallets`, `.newWallet`, `.selectedWallet`, `.secretWords`, `.transactionDetails`, `.send`, and `.coinControl` by returning the appropriate Compose screens and injecting required `AppManager`/manager instances through composition locals. Respect `AppManager.routeId` when recomposing (force `key(routeId)` so nested stacks reset like Swift).
   - Implement a thin Kotlin `RouterObserver` that listens to the Kotlin `AppManager.router` and updates `NavigationStack` state; handle nested route resets (`Route.loadAndReset`) by calling into Rust via the new `AppManager.resetRoute` helpers, mirroring Swift’s `LoadAndResetContainer`.
   - Implement `SelectedWalletContainer` and `SelectedWalletScreen` behaviours: lazily load the `WalletManager`, update balances, kick off `startWalletScan`, manage label import/export, present receive/choose-address sheets, handle `WalletErrorAlert` cases (node failure, no balance), and propagate `TaggedItem`-based alerts via the new global alert system.
   - Connect settings screens to live data: wire `settings/SettingsScreen.kt` to `AppManager` for network/fiat/node actions, and `settings/WalletSettingsScreen.kt` to `WalletManager` for name, color, labels toggle, and danger-zone flows. Use `RouteFactory().nestedSettings` and `RouteFactory().nestedWalletSettings` to drive navigation the same way Swift’s sidebar does.
   - Port reusable Swift components into Compose equivalents: loading overlays (`FullPageLoadingView`), QR scanner sheet, copy-to-clipboard popups, custom popups (e.g., `MiddlePopupView`), and any animation-critical views (e.g., dot menu) so UX stays consistent across platforms.

5. **Implement Advanced Hardware, Import, and Recovery Flows**
   - Replicate the TapSigner flow from `ios/Flows/TapSignerFlow/*`: introduce a Kotlin `TapSignerManager`, Compose screens for select/advanced/starting PIN/new PIN/confirm PIN/setup success & retry/import success/retry/enter PIN routes, and integrate `TapSignerReader` APIs from `android/app/src/main/java/org/bitcoinppl/cove/cove_tap_card.kt`. Manage NFC lifecycle, progress states, and Rust error handling to match Swift’s experience.
   - Integrate multi-format QR/NFC import: expose `MultiQr`, `SeedQr`, and `MultiFormatError` handling in Kotlin, watch `AppManager.sheetState` for `.qr`, and on scan results call `toMultiFormat()` equivalents, pushing routes or showing alerts identical to the Swift logic in `CoveApp.swift:440-556`.
   - Complete hot-wallet creation & verification: tie `flow/new_wallet/NewWalletSelectScreen.kt`, `hot_wallet/HotWalletCreateScreen.kt`, and `hot_wallet/HotWalletVerifyScreen.kt` to `PendingWalletManager`, call `saveWallet()` to persist via Rust, push verification routes using `RouteFactory().hotWallet(...)`, and connect skip/show-secret interactions to `AppManager` routes just like Swift (`VerifyWordsScreen`).
   - Add secret-word viewing guarded by auth: port `ios/Flows/SelectedWalletFlow/SecretWordsScreen.swift`, ensure `AuthManager.lock()` runs on appear, fetch mnemonic from Rust, and present warnings about wallet control. Provide copy/export options and respect color/typography cues.

6. **Support Infrastructure, Alerts, and Validation**
   - Port the sidebar experience: recreate `SidebarContainer` and `SidebarView` in Compose, animate drawers, list wallets with color indicators, surface “Add Wallet” + “Settings” actions, and implement `AppManager.toggleSidebar()` semantics. Ensure route navigation uses `RouteFactory` helpers, compares current route stacks using the Kotlin `Router`, and respects `AppManager.isSidebarVisible`.
   - Build a central alert & sheet system: mimic Swift’s use of `TaggedItem` for alerts/sheets, so SendFlow, wallet errors, TapSigner prompts, and label imports show consistent dialogs or bottom sheets. Compose must observe `AppManager.alertState`, `AppManager.sheetState`, `SendFlowPresenter.alertState`, etc., and render Material equivalents.
   - Add testing/QA coverage: create instrumentation tests or scripted manual test plans that walk each route path—initial empty state, wallet import, hot wallet creation, send flow (set amount → coin control → confirm → hardware), coin control selection, wallet settings, TapSigner setup—to verify Kotlin matches Swift behaviour.
   - Document Android-specific requirements: NFC enablement, camera permissions, preview-only stubs (if any), and any UX deviations that cannot be implemented immediately, so future work is clearly scoped and parity expectations are managed.
