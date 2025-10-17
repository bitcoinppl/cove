# Completed Implementation History

> **Purpose:** Historical reference of completed phases
> **Note:** This is a READ-ONLY record. Do not modify unless documenting newly completed work.

---

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

### 4. **Wire Compose Screens to Real Managers and Routes** ✅ COMPLETED

**Phase 4 Implementation Summary:**
- ✅ Created helper components (`FullPageLoadingView.kt`, `WalletColorExt.kt`)
- ✅ Implemented `ListWalletsScreen.kt` - Auto-selects first wallet or navigates to add wallet
- ✅ Created `SelectedWalletContainer.kt` - Manages WalletManager lifecycle with loading, scanning, and error handling
- ✅ Updated `WalletTransactionsScreen.kt` - Now uses WalletManager for real data (balance, transactions, wallet name)
- ✅ Created `SendFlowContainer.kt` - Manages WalletManager + SendFlowManager lifecycle with zero balance check
- ✅ Created `CoinControlContainer.kt` - Manages WalletManager + CoinControlManager lifecycle
- ✅ Created `SettingsContainer.kt` - Lightweight router for all settings screens with background pattern
- ✅ Created `NewWalletContainer.kt` - Simple router for new wallet flows (hot/cold/hardware)
- ✅ Created `NewHotWalletContainer.kt` - Routes to hot wallet flow screens (select/create/import/verify)
- ✅ Created `WalletSettingsContainer.kt` - Lazy loads WalletManager for wallet-specific settings
- ✅ Updated `RouteView.kt` - All routes now use containers instead of placeholders
- ✅ **Refactored `SendScreen.kt`** - Now uses AppManager, WalletManager, SendFlowManager with real state binding
- ✅ **Refactored `UtxoListScreen.kt`** - Now uses CoinControlManager, WalletManager, AppManager with UTXO selection
- ✅ **Refactored `SettingsScreen.kt`** - Now uses AppManager for navigation with RouteFactory

**Files Created (11 total):**
- `components/FullPageLoadingView.kt` - 20 lines - Reusable centered loading spinner
- `utils/WalletColorExt.kt` - 18 lines - FFI WalletColor to Compose Color conversion
- `ListWalletsScreen.kt` - 36 lines - Port of iOS ListWalletsScreen
- `SelectedWalletContainer.kt` - 95 lines - Port of iOS SelectedWalletContainer with manager lifecycle
- `SendFlowContainer.kt` - 145 lines - Port of iOS SendFlowContainer with manager initialization
- `CoinControlContainer.kt` - 75 lines - Port of iOS CoinControlContainer with async initialization
- `SettingsContainer.kt` - 80 lines - Port of iOS SettingsContainer as lightweight router
- `NewWalletContainer.kt` - 60 lines - Port of iOS NewWalletContainer for new wallet flows
- `NewHotWalletContainer.kt` - 55 lines - Port of iOS NewHotWalletContainer for hot wallet routes
- `WalletSettingsContainer.kt` - 75 lines - Port of iOS WalletSettingsContainer with lazy loading

**Files Modified (5 total):**
- `WalletTransactionsScreen.kt` - Updated to accept app + manager parameters, bind real data (~80 lines changed)
- `RouteView.kt` - Replaced all placeholders with container calls (~60 lines changed)
- **`SendScreen.kt`** - Refactored to use manager parameters instead of 14 callback/data parameters (~200 lines changed)
- **`UtxoListScreen.kt`** - Refactored to use CoinControlManager instead of mock data (~150 lines changed)
- **`SettingsScreen.kt`** - Added AppManager parameter and wired RouteFactory navigation (~30 lines changed)

**WalletTransactionsScreen Updates:**
- Now accepts `app: AppManager` and `manager: WalletManager` parameters
- Binds wallet name from `manager.walletMetadata.name`
- Displays real balance: spendable amount and fiat conversion from `manager.balance` and `manager.fiatBalance`
- Renders transaction list from `manager.loadState` (loading/scanning/loaded)
- Formats amounts using `manager.amountFmt()` and `manager.amountFmtUnit()`
- Navigation callbacks use RouteFactory: send → sendSetAmount, receive → TODO sheet, QR → app.sheetState, more → wallet settings
- Transactions are clickable and navigate to transaction details
- Shows empty state when no transactions exist
- Formats dates using SimpleDateFormat

**SendFlowContainer Features:**
- Lazy initializes WalletManager via `app.getWalletManager(id)`
- Creates SendFlowPresenter and SendFlowManager via app helpers
- Pre-populates address/amount from route parameters
- Waits for `sendFlowManager.rust.waitForInit()` before showing content
- Checks for zero balance and shows alert if needed
- Routes to appropriate screen based on SendRoute type (SetAmount, CoinControlSetAmount, Confirm, HardwareExport)
- Cleanup on disappear via DisposableEffect
- Shows CircularProgressIndicator during initialization

**CoinControlContainer Features:**
- Lazy initializes WalletManager via `app.getWalletManager(id)`
- Creates CoinControlManager via `walletManager.rust.newCoinControlManager()`
- Async initialization with suspend function
- Error handling with app-level alert on failure
- Routes to UtxoListScreen when ready
- Shows CircularProgressIndicator during loading

**SettingsContainer Features:**
- Lightweight router pattern (no manager initialization needed)
- Routes to Main/Network/Appearance/Node/FiatCurrency/Wallet/AllWallets screens
- Background with settings pattern image overlay
- Delegates to WalletSettingsContainer for wallet-specific settings
- All screens currently placeholders (TODO markers added)

**NewWalletContainer Features:**
- Simple router for new wallet flows
- Routes to Select/HotWallet/ColdWallet/Hardware/Import screens
- Delegates to NewHotWalletContainer for hot wallet sub-routes
- All screens currently placeholders (TODO markers added)

**NewHotWalletContainer Features:**
- Routes to Select/Create/Import/VerifyWords screens
- All screens currently placeholders (TODO markers added)
- Follows iOS pattern exactly

**WalletSettingsContainer Features:**
- Lazy loads WalletManager for wallet-specific settings
- Routes to Main/ChangeName/ChangeColor screens
- Error handling with app-level alert
- Shows CircularProgressIndicator during loading
- All screens currently placeholders (TODO markers added)

**SendScreen Refactoring:**
- **Before:** Accepted 14 callback/data parameters (onBack, onNext, onScanQr, balanceAmount, amountText, etc.)
- **After:** Accepts 3 manager parameters (app, walletManager, sendFlowManager)
- Balance: Binds to `walletManager.balance.spendable()` and formats based on `metadata.selectedUnit`
- Amount input: Binds to `sendFlowManager.amountField` with `dispatch(SendFlowAction.ChangeAmountField)`
- Amount in fiat: Displays `sendFlowManager.sendAmountFiat`
- Address input: Binds to `sendFlowManager.enteringAddress` with `dispatch(SendFlowAction.ChangeEnteringAddress)`
- Fee display: Shows `selectedFeeRate.duration()` and `selectedFeeRate.totalFeeString()` from SendFlowManager
- Account info: Displays `metadata.identOrFingerprint()` (fingerprint or xpub identifier)
- Total spending: Shows `sendFlowManager.totalSpentInBtc` and `sendFlowManager.totalSpentInFiat`
- Navigation: Back uses `app.popRoute()`, Next validates and dispatches `SendFlowAction.FinalizeAndGoToNextScreen`
- QR scanner: TODO - will show sheet when implemented
- Fee selector: TODO - will show sheet when implemented
- Follows iOS SendFlowSetAmountScreen.swift pattern exactly

**UtxoListScreen Refactoring:**
- **Before:** Accepted 10 callback/data parameters with mock data (utxos: List<UtxoUi>, selected: Set<String>, etc.)
- **After:** Accepts 3 manager parameters (manager: CoinControlManager, walletManager, app)
- UTXO list: Converts `manager.utxos` (Rust UTXOs) to List<UtxoUi> for display
- Selection: Binds to `manager.selected` with `dispatch(CoinControlAction.ToggleUtxo)`
- Search: Binds to `manager.search` with `dispatch(CoinControlAction.ChangeSearch)` and `ClearSearch`
- Sort: Converts `manager.sortBy` to UtxoSort enum, dispatches `dispatch(CoinControlAction.ChangeSort)`
- Select/Deselect All: Uses `dispatch(CoinControlAction.ToggleSelectAll)`
- Total selected: Displays `manager.totalSelectedAmount` (formatted by manager)
- Continue button: Calls `manager.continuePressed()` and navigates to send screen with `RouteFactory().coinControlSend()`
- Navigation: Back uses `app.popRoute()`, more menu shows TODO
- Follows iOS UtxoListScreen.swift pattern exactly

**SettingsScreen Refactoring:**
- **Before:** No parameters, TODO comments for navigation
- **After:** Accepts AppManager parameter
- Network navigation: `app.pushRoute(RouteFactory().newSettingsRoute_network())`
- Appearance navigation: `app.pushRoute(RouteFactory().newSettingsRoute_appearance())`
- Node navigation: `app.pushRoute(RouteFactory().newSettingsRoute_node())`
- Currency navigation: `app.pushRoute(RouteFactory().newSettingsRoute_fiatCurrency())`
- Back button: `app.popRoute()`
- Follows iOS MainSettingsScreen.swift pattern exactly

**RouteView Updates:**
- ListWallets → uses existing ListWalletsScreen
- SelectedWallet → uses SelectedWalletContainer (was placeholder)
- NewWallet → uses NewWalletContainer (was placeholder with inline when)
- Settings → uses SettingsContainer (was placeholder)
- Send → uses SendFlowContainer (was placeholder)
- CoinControl → uses CoinControlContainer (was placeholder)
- SecretWords → still placeholder (TODO for Phase 5)
- TransactionDetails → still placeholder (TODO for Phase 5)
- LoadAndReset → uses existing LoadAndResetContainer

**Key Architectural Patterns Established:**
```kotlin
Container (loads manager) → Screen (renders UI) → Manager (provides state)
```

**Container Types:**
1. **Lifecycle Containers** (SendFlowContainer, CoinControlContainer, SelectedWalletContainer):
   - Load managers lazily on appearance
   - Handle initialization errors
   - Show loading states
   - Clean up on disappear

2. **Router Containers** (SettingsContainer, NewWalletContainer, NewHotWalletContainer):
   - Lightweight routing only
   - No manager initialization
   - Delegate to other containers for complex flows

3. **Hybrid Containers** (WalletSettingsContainer):
   - Lazy load manager
   - Route to multiple screens based on state

**Phase 4 Complete - MVP Screens Working:**
- ✅ All core screens refactored to use managers
- ✅ SendScreen.kt fully integrated with SendFlowManager
- ✅ UtxoListScreen.kt fully integrated with CoinControlManager
- ✅ SettingsScreen.kt fully integrated with AppManager
- ✅ All containers created and wired up
- ✅ Navigation flows working end-to-end through Rust

**Remaining Work (Optional/Future):**
- Phase 5: Hardware wallet flows (TapSigner, NFC)
- Phase 5: Hot wallet creation and verification screens
- Phase 5: Secret words viewing with auth guard
- Phase 6: Sheet and alert rendering in CoveApp.kt
- Phase 6: QR scanner sheet for SendScreen
- Phase 6: Fee selector sheet for SendScreen
- Phase 6: Network/Appearance/Node/Currency settings screens
- Phase 6: Transaction details screen
- Phase 6: Wallet settings screens (change name, change color)

### 5. **Phase 5A: Hot Wallet Creation & Verification Flow** ✅ COMPLETED

**Implementation Summary:**
- Created 3 new screens: HotWalletImportScreen, VerificationCompleteScreen, VerifyWordsContainer
- Modified 2 existing screens: HotWalletSelectScreen, HotWalletCreateScreen
- Wired NewHotWalletContainer with proper lifecycle management
- Implemented two complete flows: Create (with verification) and Import (direct)

**Files Created (3 total):**
- `flow/new_wallet/hot_wallet/HotWalletImportScreen.kt` - 450 lines - Full mnemonic import with BIP-39 validation
- `flow/new_wallet/hot_wallet/VerificationCompleteScreen.kt` - 150 lines - Success screen after verification
- `flow/new_wallet/hot_wallet/VerifyWordsContainer.kt` - 90 lines - Lifecycle container for verify words flow

**Files Modified (3 total):**
- `flow/new_wallet/hot_wallet/HotWalletSelectScreen.kt` - Added modal sheet, AppManager integration, RouteFactory navigation
- `flow/new_wallet/hot_wallet/HotWalletCreateScreen.kt` - Added HorizontalPager, save wallet functionality
- `NewHotWalletContainer.kt` - Complete route wiring with lifecycle containers (Select/Create/Import/VerifyWords)

**Key Features:**
- Word-by-word BIP-39 validation with real-time feedback
- Auto-advance on valid word entry
- Color-coded validation states (green=valid, red=invalid)
- Error handling for invalid words and duplicate wallets
- Proper manager lifecycle with loading/error states
- Two complete flows: Create (with verification) and Import (direct to wallet)

**Flows Implemented:**
1. **Create Flow:** Select → Create (display recovery words) → Verify Words → Complete → Wallet
2. **Import Flow:** Select → Import (enter words) → Wallet (direct, no verification needed)

**HotWalletImportScreen Features:**
- 12 or 24 word mnemonic input in grid layout
- BIP-39 word validation using `Bip39WordSpecificAutocomplete` from FFI
- Real-time validation with color feedback per field
- Auto-advance when valid word entered
- Error alerts for invalid words or duplicate wallet detection
- Direct navigation to wallet on successful import

**VerificationCompleteScreen Features:**
- Large success checkmark icon
- "You're all set!" messaging
- Progress indicator (step 4 of 5)
- "Go To Wallet" button
- Calls `manager.rust.markWalletAsVerified()` before navigation

**VerifyWordsContainer Features:**
- Manages WalletManager and WordValidator lifecycle
- Shows VerifyWordsScreen or VerificationCompleteScreen based on state
- Handles loading/error states
- Proper cleanup via DisposableEffect

**Lessons Learned:**
1. 75% of expected screens already existed - always search first before creating
2. BIP-39 validation via FFI `Bip39WordSpecificAutocomplete` works seamlessly
3. Lifecycle containers are essential for proper manager cleanup
4. Import flow intentionally skips verification (users already have words)
5. Create flow requires full verification to ensure users backed up words
6. Container pattern (PendingWalletContainer, ImportWalletContainer) keeps code clean
7. Auto-advance on valid word improves UX significantly

**Deviations from iOS:**
1. No QR/NFC import in first pass (manual only)
2. Simpler field layout (no autocomplete suggestions above keyboard)
3. Focus management handled differently (FocusRequester vs @FocusState)

**Follow-up Items:**
- Add QR code scanning for import (deferred to Phase 5C with global sheets)
- Add NFC scanning for import (deferred to Phase 6 with TapSigner)
- Consider adding autocomplete suggestions display

### 6. **Phase 5B: Settings Screens** ✅ COMPLETED

**Implementation Summary:**
- Created generic reusable SettingsPicker component for list-based settings
- Implemented 4 settings screens: Network, Appearance, FiatCurrency, Node (placeholder)
- Wired all screens to SettingsContainer router
- Main settings already existed, just needed individual setting screens

**Files Created (5 total):**
- `settings/SettingsPicker.kt` - 120 lines - Generic reusable picker component for enum-based settings
- `settings/NetworkSettingsScreen.kt` - 160 lines - Network selection with warning dialog
- `settings/AppearanceSettingsScreen.kt` - 90 lines - Theme selection (Light/Dark/System)
- `settings/FiatCurrencySettingsScreen.kt` - 90 lines - Currency picker for fiat display
- `settings/NodeSettingsScreen.kt` - 90 lines - Placeholder for node configuration

**Files Modified (1 total):**
- `SettingsContainer.kt` - Wired all 4 new settings screens to their routes, removed TODOs

**Key Features:**
- **SettingsPicker component** - Type-safe generic picker for any list-based settings with checkmark UI
- **Network change warnings** - Confirmation dialog prevents accidental network switches
- **Theme selection** - Light/Dark/System with emoji icons
- **Currency selection** - All fiat currencies with symbols
- **Consistent navigation** - All screens use AppManager for back navigation and state updates

**SettingsPicker Component:**
- Generic `<T>` type parameter for type-safe reuse
- Card-based Material Design UI with selection states
- Checkmark icon for selected items
- Optional symbol/emoji display per item
- Scrollable list for long option lists

**NetworkSettingsScreen:**
- Bitcoin network selection (Mainnet, Testnet, Signet, Regtest)
- Warning dialog when changing network
- Confirmation flow before applying changes
- Resets app to wallet list on network change (via `app.loadAndReset()`)
- Prevents accidental back navigation when network changed

**AppearanceSettingsScreen:**
- Theme selection: Light, Dark, System
- Emoji icons for visual clarity (☀️ 🌙 ⚙️)
- Immediate state update via `AppAction.ChangeColorScheme`

**FiatCurrencySettingsScreen:**
- All FiatCurrency enum options
- Displays "USD - $" format (name + symbol)
- Immediate state update via `AppAction.ChangeFiatCurrency`

**NodeSettingsScreen:**
- Placeholder screen with TODO documentation
- Lists planned features: preset nodes, custom Electrum/Esplora, connectivity testing
- Clean UI explaining under development status

**Lessons Learned:**
1. Generic SettingsPicker component highly reusable - used for 3/4 screens
2. Network change warning critical for user safety
3. Wallet-specific settings (name, color) already existed in WalletSettingsScreen.kt
4. Card-based picker more Android-idiomatic than iOS Form style
5. Emoji symbols work well as simple icon replacements

**Deviations from iOS:**
1. **Simplified Node Settings** - iOS has full custom node URL input with validation; Android has placeholder (deferred for complexity)
2. **No security settings** - iOS has PIN/FaceID/Decoy PIN settings in MainSettingsScreen; Android defers to future phase (requires AuthManager integration)
3. **Card-based picker UI** - Android uses Material cards instead of iOS Form/List style (more idiomatic for Android)
4. **Emoji symbols** - Used simple emojis instead of SF Symbols (platform constraint)
5. **No wallet list settings** - iOS AllWallets route not implemented (TODO remains)

**Follow-up Items:**
- Implement full NodeSettingsScreen with custom Electrum/Esplora URL input and validation
- Add security settings (PIN, biometric) when AuthManager Android integration is ready
- Implement AllWallets settings list screen
- Consider adding SF Symbol equivalents using Material Icons

