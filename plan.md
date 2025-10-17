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

### 2. **Implement Kotlin Counterparts for Wallet & Send Managers** ✅ COMPLETED

**Implementation Summary:**
- ✅ Implemented full `WalletManager.kt` (313 lines) - Complete wallet state management with 13 reconciliation message types
- ✅ Implemented `SendFlowPresenter.kt` (204 lines) - UI state management for send flow with alert/sheet handling
- ✅ Implemented full `SendFlowManager.kt` (274 lines) - Send flow orchestration with validation and 11 reconciliation messages
- ✅ Implemented `CoinControlManager.kt` (180 lines) - UTXO selection with search/sort and SendFlowManager integration
- ✅ Implemented `PendingWalletManager.kt` (56 lines) - Hot wallet creation with mnemonic generation

**WalletManager Features:**
- Observable state: `walletMetadata`, `balance`, `fiatBalance`, `loadState` (loading/scanning/loaded), `foundAddresses`, `unsignedTransactions`, `errorAlert`, `sendFlowErrorAlert`
- Computed properties: `unit`, `hasTransactions`, `isVerified`, `accentColor`
- Methods: `forceWalletScan()`, `updateWalletBalance()`, `firstAddress()`, `amountFmt()`, `displayAmount()`, `transactionDetails()` with caching
- Three constructors: from `WalletId`, from `xpub`, from `TapSigner`
- 13 reconciliation messages handled: scan states, balance changes, metadata updates, address discovery, error handling

**SendFlowManager Features:**
- User input state: `enteringBtcAmount`, `enteringFiatAmount`, `enteringAddress` with auto-dispatch
- Validated state: `address`, `amount`, `fiatAmount`
- Fee state: `selectedFeeRate`, `feeRateOptions`, `maxSelected`
- Presenter strings: `sendAmountFiat`, `sendAmountBtc`, `totalSpentInFiat`, `totalSpentInBtc`, `totalFeeString`
- Validation methods: `validate()`, `validateAddress()`, `validateAmount()`, `validateFeePercentage()`
- 11 reconciliation messages: amount updates, fee options, focus field, alerts, max mode
- Debounced dispatch for high-frequency updates (66ms default)

**SendFlowPresenter Features:**
- UI-only state: `focusField`, `sheetState` (Qr, Fee), `alertState`
- Alert handling: `alertTitle()`, `alertMessage()`, `alertButtonAction()`
- Error mapping: Maps all `SendFlowError` types to user-friendly messages
- Sheet states: QR scanner and fee selector
- Disappearing state management for transitions

**CoinControlManager Features:**
- UTXO selection state: `search`, `selected`, `totalSelected`, `utxos`, `unit`, `sort`
- UI helpers: `buttonColor()`, `buttonTextColor()`, `buttonArrow()` for sort buttons
- SendFlowManager integration: `continuePressed()` applies selection, debounced updates (100ms)
- 7 reconciliation messages: sort, search, UTXO list, selection, unit, total amount

**PendingWalletManager Features:**
- Simple wrapper around `RustPendingWalletManager`
- Observable state: `numberOfWords`, `bip39Words`
- 1 reconciliation message: word count changes trigger regeneration

**Threading Model:**
- All managers use `GlobalScope.launch(Dispatchers.IO)` for rust bridge
- State updates on `Dispatchers.Main` via `withContext`
- Consistent pattern across all managers

**Key Implementation Notes:**
1. All managers implement their respective `*Reconciler` interfaces from FFI
2. Used `@Stable` annotation for Compose optimization
3. Private `apply()` method for reconciliation logic, public `reconcile()` for interface
4. Consistent logging with manager-specific tags
5. Transaction details caching in WalletManager prevents redundant fetches
6. Debouncing in SendFlowManager and CoinControlManager for responsive UI

**Files Created (5 total):**
- `WalletManager.kt` - 313 lines (replaced 24-line placeholder)
- `SendFlowPresenter.kt` - 204 lines (new)
- `SendFlowManager.kt` - 274 lines (replaced 28-line placeholder)
- `CoinControlManager.kt` - 180 lines (new)
- `PendingWalletManager.kt` - 56 lines (new)

**Lessons Learned:**
1. FFI reconciler interfaces work perfectly - no need to wrap or abstract
2. Kotlin's `by mutableStateOf()` delegate is cleaner than manual getters/setters
3. GlobalScope is acceptable for fire-and-forget rust bridge operations
4. Debouncing is crucial for text input and UTXO selection to avoid excessive reconciliation
5. SendFlowPresenter benefits from being separate - keeps UI concerns isolated from business logic
6. Transaction details caching significantly reduces rust calls for frequently accessed data

**Deviations from iOS:**
1. Used `Set<String>` instead of `Set<Utxo.ID>` for UTXO selection (simpler in Kotlin)
2. Color conversion from `WalletColor` to Compose `Color` not yet implemented (TODO marker added)
3. No preview constructors yet (will add when needed for Compose previews)
4. Alert button actions return nullable lambda instead of SwiftUI ViewBuilder

**Follow-up Items:**
- Need to implement `WalletColor` to Compose `Color` conversion
- Phase 4 will wire these managers to actual Compose screens
- Add preview constructors for Compose @Preview functions
- Consider adding proper CoroutineScope management instead of GlobalScope

### 3. **Setup Navigation (Rust-First)** ✅ COMPLETED

**Implementation Summary:**
- ✅ Updated `RouterManager.kt` - Added `structuralEqualityPolicy()` to prevent recomposition feedback loops
- ✅ Enhanced `AppManager.kt` navigation methods - Added route comparison guards to prevent ping-pong
- ✅ Implemented `LoadAndResetContainer` in `RouteView.kt` - Shows loading, delays, then executes route reset
- ✅ Added hardware back button support in `CoveApp.kt` - `BackHandler` intercepts system back and routes through Rust
- ✅ Improved reconciliation to use immutable copies - All route updates use `.toList()` so Compose detects changes

**RouterManager Enhancements:**
- Observable properties now use `structuralEqualityPolicy()` to avoid unnecessary recompositions
- Prevents Compose ↔ Rust feedback loops when route objects are structurally equal
```kotlin
var default: Route by mutableStateOf(ffiRouter.default, structuralEqualityPolicy())
var routes: List<Route> by mutableStateOf(ffiRouter.routes, structuralEqualityPolicy())
```

**AppManager Navigation Guardrails:**
- All navigation methods (`pushRoute`, `popRoute`, `setRoute`, `pushRoutes`) now:
  - Log navigation actions for debugging
  - Compare new routes to current routes before dispatching
  - Only dispatch `AppAction.UpdateRoute` if routes actually changed
  - Use immutable copies in reconciliation (`.toList()`)
- Prevents duplicate dispatches and unnecessary Rust operations

**LoadAndResetContainer:**
- Ported from iOS `LoadAndResetContainer.swift`
- Shows `CircularProgressIndicator` during loading
- Uses `LaunchedEffect` to delay for specified milliseconds
- Handles both single route and nested route resets
- Properly calls `app.resetRoute()` after delay

**Hardware Back Button:**
- `BackHandler` in `MainAppContent` intercepts system back
- Enabled only when `router.routes.isNotEmpty()`
- Calls `app.popRoute()` which dispatches through Rust
- Maintains single source of truth (Rust owns navigation state)

**Reconciliation Improvements:**
- `RouteUpdated`: Creates immutable copy with `.toList()`
- `PushedRoute`: Uses `+` operator then `.toList()` for immutability
- `DefaultRouteChanged`: Creates immutable copy and logs new `routeId`
- All route updates ensure Compose sees new list references

**Key Architectural Decisions:**
1. **Direct Mapping**: Continued with direct `when` statement in RouteView (no NavHostController needed)
2. **Structural Equality**: Used `structuralEqualityPolicy()` as recommended by navigation_plan.md
3. **Immutable Snapshots**: Always create new lists in reconciliation for Compose change detection
4. **Back Button Routing**: Hardware back always goes through Rust to maintain consistency
5. **Comparison Guards**: Navigation methods check if change is needed before dispatching

**Files Modified (4 total):**
- `RouterManager.kt` - Added structural equality policy (8 lines modified)
- `AppManager.kt` - Enhanced navigation methods with logging and guards (85 lines modified)
- `RouteView.kt` - Added LoadAndResetContainer implementation (30 lines added)
- `CoveApp.kt` - Added BackHandler and improved loading screen (18 lines modified)

**Navigation Flow:**
```
User Action → UI calls app.pushRoute()
           → Compares with current routes
           → Dispatches AppAction.UpdateRoute if different
           → Rust updates router state
           → Reconcile message updates RouterManager
           → Immutable copy triggers Compose recomposition
           → RouteView renders new screen
```

**Hardware Back Flow:**
```
User presses back → BackHandler intercepts
                 → Calls app.popRoute()
                 → Dispatches trimmed stack through Rust
                 → Same reconciliation flow as above
```

**Lessons Learned:**
1. `structuralEqualityPolicy()` is crucial for preventing feedback loops with FFI objects
2. Route comparison before dispatch prevents unnecessary Rust operations
3. Immutable copies (`.toList()`) are required for Compose to detect changes
4. Hardware back must be explicitly handled in Compose with `BackHandler`
5. LoadAndReset pattern is elegant with `LaunchedEffect` and `delay()`
6. Logging navigation actions is invaluable for debugging
7. Direct routing (no NavHostController) is simpler and more aligned with Rust-first architecture

**Deviations from iOS:**
1. No `NavigationStack` - using direct `when` statement instead
2. `BackHandler` instead of SwiftUI's automatic back handling
3. `LaunchedEffect` with `delay()` instead of Task.sleep
4. No equivalent to SwiftUI's `.id(app.routeId)` - using Box with key parameter

**Follow-up Items:**
- Deep link handling deferred to Phase 4+
- Process death restoration deferred to Phase 4+
- Multi-stack tabs/sidebar deferred to Phase 4+
- Consider NavHostController only if animations/transitions are needed

### 4. **Wire Compose Screens to Real Managers and Routes** 🚧 IN PROGRESS

**Phase 4 Progress Summary:**
- ✅ Created helper components (`FullPageLoadingView.kt`, `WalletColorExt.kt`)
- ✅ Implemented `ListWalletsScreen.kt` - Auto-selects first wallet or navigates to add wallet
- ✅ Created `SelectedWalletContainer.kt` - Manages WalletManager lifecycle with loading, scanning, and error handling
- ⏳ Remaining: Screen updates, containers for Send/CoinControl/Settings/NewWallet flows

**Files Created (5 total so far):**
- `components/FullPageLoadingView.kt` - 20 lines - Reusable centered loading spinner
- `utils/WalletColorExt.kt` - 18 lines - FFI WalletColor to Compose Color conversion
- `ListWalletsScreen.kt` - 36 lines - Port of iOS ListWalletsScreen
- `SelectedWalletContainer.kt` - 95 lines - Port of iOS SelectedWalletContainer with manager lifecycle

**ListWalletsScreen Features:**
- Shows FullPageLoadingView while checking database
- Uses `Database().wallets().all()` to get wallet list
- Auto-selects first wallet via `app.rust.selectWallet()`
- Navigates to add wallet screen if no wallets exist
- Error handling navigates to add wallet as fallback

**SelectedWalletContainer Features:**
- Lazy loads WalletManager via `app.getWalletManager(id)`
- Calls `manager.rust.getTransactions()` and `manager.rust.startWalletScan()` on appear
- Updates balance after 500ms delay
- Handles wallet not found → tries other wallet or navigates to add wallet
- Dispatches `.SelectedWalletDisappeared` on cleanup via DisposableEffect
- Updates `app.walletManager` when load state becomes LOADED
- Currently renders WalletTransactionsScreen (needs update)

**WalletColorExt Features:**
- Converts all 13 FFI WalletColor variants to Compose Color
- Uses iOS system color values for consistency
- Extension function pattern: `walletColor.toComposeColor()`

**Next Steps:**
- Update WalletTransactionsScreen to accept manager parameters
- Create SendFlowContainer, CoinControlContainer, NewWalletContainer, SettingsContainer
- Update existing screens to bind to manager state
- Wire all routes in RouteView.kt
- Estimated remaining: ~15-20 files, ~1500-2000 lines

**Key Pattern Established:**
```kotlin
Container (loads manager) → Screen (renders UI) → Manager (provides state)
```

## TODO Items

1. **Bootstrap Kotlin App Shell and Core Managers** ✅

2. **Implement Kotlin Counterparts for Wallet & Send Managers** ✅

3. **Setup Navigation (Rust-First)** ✅

4. **Wire Compose Screens to Real Managers and Routes** 🚧 IN PROGRESS
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
