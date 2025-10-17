# Android Deviations from iOS Implementation

> **Purpose:** Comprehensive documentation of all differences between Android (Kotlin/Compose) and iOS (Swift/SwiftUI) implementations
> **Last Updated:** 2025-10-17
> **Branch:** android-core-managers

---

## Table of Contents

1. [Core Architecture](#1-core-architecture-phase-1)
2. [Manager Implementation](#2-manager-implementation-phase-2)
3. [Navigation System](#3-navigation-system-phase-3)
4. [Screen & Container Patterns](#4-screen--container-patterns-phase-4)
5. [Hot Wallet Flow](#5-hot-wallet-flow-phase-5a)
6. [Settings Screens](#6-settings-screens-phase-5b)
7. [UI Components](#7-ui-components)
8. [FFI & Platform Integration](#8-ffi--platform-integration)
9. [Deferred Features](#9-deferred-features-not-yet-implemented)

---

## 1. Core Architecture (Phase 1)

### 1.1 Singleton Pattern

**iOS:**
```swift
@Observable final class AppManager {
    static let shared = AppManager()
}
// Usage: AppManager.shared
```

**Android:**
```kotlin
@Stable
class AppManager private constructor() {
    companion object {
        @Volatile
        private var instance: AppManager? = null

        fun getInstance(): AppManager {
            return instance ?: synchronized(this) {
                instance ?: AppManager().also { instance = it }
            }
        }
    }
}
// Usage: AppManager.getInstance()
```

**Reason:** Kotlin uses object declaration pattern with double-checked locking for thread-safe lazy initialization.

### 1.2 Global Accessors

**iOS:**
```swift
AppManager.shared
AuthManager.shared
```

**Android:**
```kotlin
val App: AppManager
    get() = AppManager.getInstance()

val Auth: AuthManager
    get() = AuthManager.getInstance()
```

**Reason:** Provides more idiomatic Kotlin access pattern similar to companion object properties.

### 1.3 State Management

**iOS:**
```swift
@Observable final class AppManager {
    var isLoading = false
    var router: Router
}
```

**Android:**
```kotlin
@Stable
class AppManager {
    var isLoading by mutableStateOf(false)
    var router: RouterManager by mutableStateOf(...)
}
```

**Reason:** Compose uses `mutableStateOf()` for observable state instead of Swift's `@Observable` macro. Requires explicit state delegation.

### 1.4 Router Management

**iOS:**
```swift
// Direct FFI Router usage
var router: Router
```

**Android:**
```kotlin
// Wrapper class for Compose observability
class RouterManager(internal var ffiRouter: Router) {
    var default: Route by mutableStateOf(ffiRouter.default, structuralEqualityPolicy())
    var routes: List<Route> by mutableStateOf(ffiRouter.routes, structuralEqualityPolicy())
}
```

**Reason:** Need wrapper to make FFI Router observable in Compose with proper structural equality to prevent feedback loops.

### 1.5 Memory Management

**iOS:**
```swift
// Weak references used to prevent retain cycles
weak var delegate: SomeDelegate?
```

**Android:**
```kotlin
// No weak references needed - GC handles it
// But still avoid circular references in design
```

**Reason:** Kotlin's garbage collector handles memory automatically, no manual weak reference management needed.

### 1.6 NFC Integration

**iOS:**
```swift
var nfcReader = NFCReader()
var nfcWriter = NFCWriter()
```

**Android:**
```kotlin
// Not yet implemented in AppManager
// TODO: Add when implementing NFC flows
```

**Reason:** NFC integration deferred to Phase 6 (TapSigner flow).

### 1.7 Sealed Classes vs Enums

**iOS:**
```swift
enum AppAlertState {
    case importedSuccessfully
    case duplicateWallet
    case errorImportingHotWallet(String)
}
```

**Android:**
```kotlin
sealed class AppAlertState {
    data object ImportedSuccessfully : AppAlertState()
    data object DuplicateWallet : AppAlertState()
    data class ErrorImportingHotWallet(val error: String) : AppAlertState()
}
```

**Reason:** Sealed classes are more idiomatic Kotlin for modeling Swift enums with associated values. Provides better type safety and exhaustive when expressions.

### 1.8 Coroutine Scopes

**iOS:**
```swift
// No scope management needed for @Observable
```

**Android:**
```kotlin
open class BaseManager : ViewModel() {
    // Lifecycle-aware coroutine scope
    // Automatically cancelled when cleared
}
```

**Reason:** Android managers extend ViewModel to get `viewModelScope` for lifecycle-aware coroutine management.

---

## 2. Manager Implementation (Phase 2)

### 2.1 Threading Model

**iOS:**
```swift
func forceWalletScan() async {
    await rust.forceWalletScan()
    await updateWalletBalance()
}
```

**Android:**
```kotlin
suspend fun forceWalletScan() {
    GlobalScope.launch(Dispatchers.IO) {
        rust.forceWalletScan()
        withContext(Dispatchers.Main) {
            updateWalletBalance()
        }
    }
}
```

**Reason:** Kotlin uses coroutines with explicit dispatchers (`Dispatchers.IO` for rust bridge, `Dispatchers.Main` for state updates) instead of Swift's async/await.

### 2.2 Property Delegates

**iOS:**
```swift
var balance: WalletBalance?
```

**Android:**
```kotlin
var balance: WalletBalance? by mutableStateOf(null)
```

**Reason:** Kotlin uses property delegation `by mutableStateOf()` for cleaner syntax vs manual getters/setters.

### 2.3 UTXO Selection Data Types

**iOS:**
```swift
var selected: Set<Utxo.ID>
```

**Android:**
```kotlin
var selected: Set<String>  // UTXO IDs as strings
```

**Reason:** Simpler type in Kotlin - uses string IDs directly instead of nested type.

### 2.4 Color Conversion

**iOS:**
```swift
// Built-in Color support for WalletColor
let color = Color(walletColor)
```

**Android:**
```kotlin
// Custom extension needed
fun WalletColor.toComposeColor(): Color {
    return when (this) {
        WalletColor.ORANGE -> Orange
        WalletColor.PURPLE -> Purple
        // ...
    }
}
```

**Reason:** No automatic conversion from FFI WalletColor to Compose Color - requires manual mapping extension.

### 2.5 Alert Button Actions

**iOS:**
```swift
@ViewBuilder
func alertButtonAction() -> some View {
    Button("OK") { /* action */ }
}
```

**Android:**
```kotlin
fun alertButtonAction(): (() -> Unit)? {
    return { /* action */ }
}
```

**Reason:** Returns nullable lambda instead of SwiftUI ViewBuilder. Simpler model for Android's AlertDialog.

### 2.6 Debouncing

**iOS:**
```swift
// Task-based debouncing with Task.sleep
```

**Android:**
```kotlin
private var debounceJob: Job? = null

private fun debounceDispatch(delay: Long, action: AppAction) {
    debounceJob?.cancel()
    debounceJob = GlobalScope.launch {
        delay(delay)
        rust.dispatch(action)
    }
}
```

**Reason:** Explicit job management for debouncing high-frequency updates (text input, UTXO selection).

### 2.7 Preview Constructors

**iOS:**
```swift
// Preview constructors for SwiftUI previews
init(preview: Bool) { }
```

**Android:**
```kotlin
// Not yet implemented
// TODO: Add when needed for @Preview functions
```

**Reason:** Compose previews work differently - may not need special constructors. Deferred until needed.

---

## 3. Navigation System (Phase 3)

### 3.1 Navigation Architecture

**iOS:**
```swift
NavigationStack(path: $router.routes) {
    RouteView(route: router.default)
        .navigationDestination(for: Route.self) { route in
            RouteView(route: route)
        }
}
```

**Android:**
```kotlin
// Direct when statement mapping
@Composable
fun RouteView(app: AppManager, route: Route) {
    when (route) {
        is Route.SelectedWallet -> SelectedWalletContainer(app, route.id)
        is Route.Send -> SendFlowContainer(app, route.send)
        // ...
    }
}
```

**Reason:** No NavigationStack needed - direct mapping is simpler and more aligned with Rust-first architecture. Avoids NavHostController complexity.

### 3.2 Hardware Back Button

**iOS:**
```swift
// Automatic back handling in NavigationStack
```

**Android:**
```kotlin
BackHandler(enabled = app.router.routes.isNotEmpty()) {
    app.popRoute()
}
```

**Reason:** Must explicitly intercept system back button and route through Rust to maintain single source of truth.

### 3.3 Structural Equality Policy

**iOS:**
```swift
// No special handling needed
var router: Router
```

**Android:**
```kotlin
var default: Route by mutableStateOf(
    ffiRouter.default,
    structuralEqualityPolicy()  // Required!
)
```

**Reason:** Prevents Compose ↔ Rust feedback loops when route objects are structurally equal but different references.

### 3.4 Immutable Route Snapshots

**iOS:**
```swift
// Direct array mutation
router.routes.append(route)
```

**Android:**
```kotlin
// Must create immutable copies
val newRoutes = (router.routes + route).toList()
router.updateRoutes(newRoutes)
```

**Reason:** Compose change detection requires new list references to trigger recomposition.

### 3.5 Route Comparison Guards

**iOS:**
```swift
func pushRoute(_ route: Route) {
    router.routes.append(route)
}
```

**Android:**
```kotlin
fun pushRoute(route: Route) {
    logDebug("pushRoute: $route")
    val newRoutes = router.routes + route

    // only dispatch if routes actually changed
    if (newRoutes != router.routes) {
        dispatch(AppAction.UpdateRoute(newRoutes))
    }
}
```

**Reason:** Navigation methods check if change is needed before dispatching to prevent duplicate operations and unnecessary Rust calls.

### 3.6 LoadAndReset Pattern

**iOS:**
```swift
Task {
    try? await Task.sleep(for: .milliseconds(time))
    resetRoute(to: route)
}
```

**Android:**
```kotlin
LaunchedEffect(Unit) {
    delay(millis.toLong())
    app.resetRoute(resetTo)
}
```

**Reason:** `LaunchedEffect` with `delay()` instead of Task.sleep. More idiomatic for Compose side effects.

### 3.7 Route ID for Lifecycle Reset

**iOS:**
```swift
ContentView()
    .id(app.routeId)
```

**Android:**
```kotlin
Box(
    modifier = Modifier.fillMaxSize(),
    key = app.routeId  // Forces recomposition
) {
    RouteView(app, app.currentRoute)
}
```

**Reason:** Uses `key` parameter instead of `.id()` modifier to force recomposition on route resets.

---

## 4. Screen & Container Patterns (Phase 4)

### 4.1 Container Types

**Pattern:** Three distinct container types established in Android:

1. **Lifecycle Containers** - Manage complex state with manager initialization/cleanup
   - Examples: SendFlowContainer, CoinControlContainer, SelectedWalletContainer
   - Pattern: Load → Initialize → Show → Cleanup on dispose

2. **Router Containers** - Lightweight routing only, no manager initialization
   - Examples: SettingsContainer, NewWalletContainer, NewHotWalletContainer
   - Pattern: Direct when-based routing to screens

3. **Hybrid Containers** - Router + lazy manager loading
   - Examples: WalletSettingsContainer
   - Pattern: Routes decide whether to load manager or show simple screen

**iOS:** Uses more flexible view composition without strict container types.

**Reason:** Android needs explicit lifecycle management with `DisposableEffect` for manager cleanup. Containers provide consistent patterns.

### 4.2 Manager Lifecycle

**iOS:**
```swift
struct SomeView: View {
    @State var manager: SomeManager?

    var body: some View {
        content
            .onAppear {
                manager = SomeManager()
            }
            .onDisappear {
                manager = nil
            }
    }
}
```

**Android:**
```kotlin
@Composable
fun SomeContainer(app: AppManager) {
    var manager by remember { mutableStateOf<SomeManager?>(null) }
    var loading by remember { mutableStateOf(true) }

    LaunchedEffect(Unit) {
        manager = SomeManager()
        loading = false
    }

    DisposableEffect(Unit) {
        onDispose {
            manager?.cleanup()
        }
    }

    when {
        loading -> FullPageLoadingView()
        manager != null -> SomeScreen(app, manager!!)
    }
}
```

**Reason:** Explicit loading states and cleanup needed. `DisposableEffect` ensures managers are properly disposed when container leaves composition.

### 4.3 Parameter Passing

**iOS:**
```swift
struct SomeView: View {
    @Environment(\.appManager) var app
    @State var manager: WalletManager
}
```

**Android:**
```kotlin
@Composable
fun SomeScreen(
    app: AppManager,
    manager: WalletManager
) {
    // Direct parameter access
}
```

**Reason:** No environment system in Compose - managers passed explicitly as parameters. More explicit but less "magical".

### 4.4 Loading States

**iOS:**
```swift
// Often implicit in SwiftUI with ProgressView
if loading {
    ProgressView()
}
```

**Android:**
```kotlin
// Explicit with reusable component
@Composable
fun FullPageLoadingView() {
    Box(
        modifier = Modifier.fillMaxSize(),
        contentAlignment = Alignment.Center
    ) {
        CircularProgressIndicator()
    }
}
```

**Reason:** Created dedicated component for consistent loading UI across app.

### 4.5 Error Handling in Containers

**iOS:**
```swift
// Error thrown and caught higher up
try WalletManager(id: id)
```

**Android:**
```kotlin
LaunchedEffect(Unit) {
    try {
        manager = WalletManager(id)
    } catch (e: Exception) {
        app.alertState = TaggedItem(
            AppAlertState.General("Failed to load wallet")
        )
    }
    loading = false
}
```

**Reason:** Explicit error handling with app-level alerts in containers. Prevents crashes and provides user feedback.

---

## 5. Hot Wallet Flow (Phase 5A)

### 5.1 QR/NFC Import

**iOS:**
```swift
// Full QR and NFC import support
.sheet(isPresented: $showQrScanner) {
    QrScannerView()
}
```

**Android:**
```kotlin
// Not implemented in first pass
// TODO: Add QR code scanning (deferred to Phase 5C)
// TODO: Add NFC scanning (deferred to Phase 6)
```

**Reason:** Manual word entry only for MVP. QR scanning requires sheet system (Phase 5C). NFC requires full TapSigner integration (Phase 6).

### 5.2 Field Layout and Autocomplete

**iOS:**
```swift
// Autocomplete suggestions displayed above keyboard
TextField("Word", text: $word)
    .autocorrectionDisabled()
// Suggestions view above
SuggestionsView(suggestions: suggestions)
```

**Android:**
```kotlin
// Simpler field layout
// No autocomplete suggestions above keyboard
OutlinedTextField(
    value = word,
    onValueChange = { /* validation */ },
    // Real-time validation with color feedback
    isError = !isValid
)
```

**Reason:** Simpler UX for first pass. Real-time validation with color coding (green=valid, red=invalid) instead of suggestion list. Auto-advance on valid word compensates for missing suggestions.

### 5.3 Focus Management

**iOS:**
```swift
@FocusState var focusedField: Int?

TextField(...)
    .focused($focusedField, equals: index)
```

**Android:**
```kotlin
val focusRequesters = remember {
    List(wordCount) { FocusRequester() }
}

OutlinedTextField(
    modifier = Modifier.focusRequester(focusRequesters[index])
)

// Manual focus transfer
LaunchedEffect(index) {
    if (shouldFocus) {
        focusRequesters[index].requestFocus()
    }
}
```

**Reason:** Different focus API - `FocusRequester` objects instead of `@FocusState`. More verbose but same functionality.

### 5.4 Word Validation API

**iOS & Android:** Both use same FFI API
```kotlin
val validator = Bip39WordSpecificAutocomplete(prefix)
val isValid = validator.exactMatch(word)
```

**No deviation** - FFI validation works identically on both platforms.

### 5.5 Verification Flow

**iOS:**
```swift
// Integrated in single view with state machine
```

**Android:**
```kotlin
// Separate container for verification
@Composable
fun VerifyWordsContainer(app: AppManager, id: WalletId) {
    // Shows VerifyWordsScreen or VerificationCompleteScreen
}
```

**Reason:** Container pattern used for consistency with other flows. Cleaner separation of concerns.

---

## 6. Settings Screens (Phase 5B)

### 6.1 Node Settings

**iOS:**
```swift
// Full custom node URL input with validation
struct NodeSettingsScreen: View {
    @State var customUrl: String
    @State var nodeType: NodeType

    var body: some View {
        Form {
            Picker("Type", selection: $nodeType) {
                Text("Electrum").tag(NodeType.electrum)
                Text("Esplora").tag(NodeType.esplora)
            }
            TextField("URL", text: $customUrl)
            Button("Test Connection") { }
        }
    }
}
```

**Android:**
```kotlin
// Placeholder screen
@Composable
fun NodeSettingsScreen(app: AppManager) {
    Column {
        Text("Node Settings")
        Text("Under Development")
        Text("""
            Planned features:
            - Preset nodes (Blockstream, Mempool.space)
            - Custom Electrum URL input
            - Custom Esplora URL input
            - Connection testing
        """)
    }
}
```

**Reason:** Deferred due to complexity - requires URL validation, connection testing, error handling. Placeholder documents planned features.

### 6.2 Security Settings

**iOS:**
```swift
// PIN, FaceID, Decoy PIN settings in MainSettingsScreen
struct MainSettingsScreen: View {
    Section("Security") {
        Toggle("Enable PIN", isOn: $pinEnabled)
        Toggle("Use Face ID", isOn: $faceIdEnabled)
        Button("Setup Decoy PIN") { }
    }
}
```

**Android:**
```kotlin
// Not implemented in MainSettingsScreen
// AuthManager exists but no UI integration yet
// TODO: Add security settings when AuthManager Android integration ready
```

**Reason:** AuthManager exists but needs Android-specific biometric integration, PIN UI, and decoy wallet flows. Deferred to future phase.

### 6.3 Picker UI Pattern

**iOS:**
```swift
Form {
    Picker("Network", selection: $selectedNetwork) {
        ForEach(networks) { network in
            Text(network.name).tag(network)
        }
    }
}
```

**Android:**
```kotlin
// Generic reusable component with Material Design
@Composable
fun <T> SettingsPicker(
    items: List<T>,
    selectedItem: T,
    onItemSelected: (T) -> Unit,
    itemLabel: (T) -> String,
    itemSymbol: ((T) -> String)? = null
) {
    LazyColumn {
        items(items) { item ->
            Card(
                onClick = { onItemSelected(item) }
            ) {
                Row {
                    Text(itemLabel(item))
                    if (item == selectedItem) {
                        Icon(Icons.Default.Check)
                    }
                    itemSymbol?.let { Text(it(item)) }
                }
            }
        }
    }
}
```

**Reason:** Card-based Material Design picker more idiomatic for Android vs iOS Form/List style. Type-safe generic component reusable across multiple settings screens.

### 6.4 Symbols/Icons

**iOS:**
```swift
Label("Light", systemImage: "sun.max")
Label("Dark", systemImage: "moon.fill")
Label("System", systemImage: "gear")
```

**Android:**
```kotlin
// Simple emoji symbols
"☀️ Light"
"🌙 Dark"
"⚙️ System"
```

**Reason:** No SF Symbols on Android. Used emojis for visual clarity instead of Material Icons for simplicity.

### 6.5 Wallet Settings

**iOS:**
```swift
// Part of WalletSettingsScreen in iOS
```

**Android:**
```kotlin
// Already existed in WalletSettingsScreen.kt
// No changes needed
```

**No deviation** - Wallet-specific settings (name, color) already implemented correctly before Phase 5B.

---

## 7. UI Components

### 7.1 Modal Sheets

**iOS:**
```swift
.sheet(isPresented: $showSheet) {
    SheetContent()
}
```

**Android:**
```kotlin
val sheetState = rememberModalBottomSheetState()
var showSheet by remember { mutableStateOf(false) }

if (showSheet) {
    ModalBottomSheet(
        onDismissRequest = { showSheet = false },
        sheetState = sheetState
    ) {
        SheetContent()
    }
}
```

**Reason:** Composable-based sheets instead of modifier-based. Requires explicit state management with `ModalBottomSheetState`.

### 7.2 Alert Dialogs

**iOS:**
```swift
.alert(
    "Title",
    isPresented: $showAlert
) {
    Button("OK") { }
} message: {
    Text("Message")
}
```

**Android:**
```kotlin
if (showAlert) {
    AlertDialog(
        onDismissRequest = { showAlert = false },
        title = { Text("Title") },
        text = { Text("Message") },
        confirmButton = {
            TextButton(onClick = { showAlert = false }) {
                Text("OK")
            }
        }
    )
}
```

**Reason:** Composable component instead of modifier. More flexible but more verbose.

### 7.3 Animations

**iOS:**
```swift
withAnimation(.easeInOut(duration: 0.3)) {
    value = newValue
}
```

**Android:**
```kotlin
val animatable = remember { Animatable(initialValue) }

LaunchedEffect(trigger) {
    animatable.animateTo(
        targetValue = targetValue,
        animationSpec = tween(
            durationMillis = 300,
            easing = LinearEasing
        )
    )
}
```

**Reason:** Different animation API - `Animatable` with explicit animation specs instead of implicit animation blocks.

### 7.4 Horizontal Paging

**iOS:**
```swift
TabView(selection: $currentPage) {
    ForEach(pages) { page in
        PageView(page)
            .tag(page.id)
    }
}
.tabViewStyle(.page)
```

**Android:**
```kotlin
val pagerState = rememberPagerState(pageCount = { pages.size })

HorizontalPager(state = pagerState) { pageIndex ->
    PageView(pages[pageIndex])
}
```

**Reason:** Dedicated `HorizontalPager` component instead of styled TabView. More explicit pager semantics.

### 7.5 List Components

**iOS:**
```swift
List {
    ForEach(items) { item in
        ItemRow(item)
    }
}
```

**Android:**
```kotlin
LazyColumn {
    items(items) { item ->
        ItemRow(item)
    }
}
```

**Reason:** `LazyColumn` instead of List. Similar API but different performance characteristics.

### 7.6 Theme System

**iOS:**
```swift
@Environment(\.colorScheme) var colorScheme

// Dynamic colors
Color(.systemBackground)
Color(.label)
```

**Android:**
```kotlin
// Material3 theming
MaterialTheme.colorScheme.background
MaterialTheme.colorScheme.onBackground

// Custom theme object
CoveTheme {
    // Automatically handles light/dark mode
}
```

**Reason:** Material3 theming system vs iOS semantic colors. Both support dark mode but different API.

---

## 8. FFI & Platform Integration

### 8.1 Async/Await vs Coroutines

**iOS:**
```swift
func someOperation() async throws {
    let result = try await rust.someOperation()
}
```

**Android:**
```kotlin
suspend fun someOperation() {
    GlobalScope.launch(Dispatchers.IO) {
        val result = rust.someOperation()
        withContext(Dispatchers.Main) {
            // update state
        }
    }
}
```

**Reason:** Kotlin coroutines with explicit dispatchers instead of Swift's structured concurrency.

### 8.2 Error Handling

**iOS:**
```swift
do {
    try rust.operation()
} catch {
    logger.error("Failed: \(error)")
}
```

**Android:**
```kotlin
try {
    rust.operation()
} catch (e: Exception) {
    Log.e(tag, "Failed", e)
}
```

**Reason:** Similar pattern but different exception types and logging APIs.

### 8.3 Package Structure

**iOS:**
```swift
import CoveCore

// Generated bindings in CoveCore Swift package
```

**Android:**
```kotlin
package org.bitcoinppl.cove

// Generated bindings in same package as app code
```

**Reason:** iOS uses separate Swift Package. Android uses same package namespace for simpler integration.

### 8.4 Binding File Structure

**iOS:**
```
ios/CoveCore/Sources/CoveCore/generated/
  - cove.swift (monolithic)
```

**Android:**
```
android/app/src/main/java/org/bitcoinppl/cove/
  - cove.kt
  - cove_device.kt
  - cove_nfc.kt
  - cove_tap_card.kt
  - cove_types.kt
  - cove_util.kt
```

**Reason:** Android splits per-crate. Uses `strip_ffi_duplicates.py` to remove redundant scaffolding.

### 8.5 Callback Handling

**iOS:**
```swift
protocol FfiReconcile {
    func reconcile(message: AppStateReconcileMessage)
}

rust.listenForUpdates(updater: self)
```

**Android:**
```kotlin
interface FfiReconcile {
    fun reconcile(message: AppStateReconcileMessage)
}

rust.listenForUpdates(this)
```

**No significant deviation** - Both use protocol/interface for callbacks.

---

## 9. Deferred Features (Not Yet Implemented)

### 9.1 Phase 5C: Sheet & Alert System

**Status:** Next phase
**Scope:**
- Global sheet rendering in CoveApp
- QR scanner sheet for send flow
- Fee selector sheet
- Complete alert rendering for all types

**Current State:**
- Alert infrastructure exists but basic rendering only
- Sheet state exists but no content rendering
- Both have TODO markers

### 9.2 Phase 5D: Transaction Details

**Status:** Planned
**Scope:**
- Full transaction details screen
- Confirmation status indicator
- Input/output address list
- Amount breakdown

**Current State:**
- Placeholder exists in RouteView
- Navigation wired but screen is TODO

### 9.3 Phase 5E: Secret Words Viewing

**Status:** Planned
**Scope:**
- Auth-guarded secret words display
- Recovery phrase viewing for hot wallets
- Security warnings

**Current State:**
- Route exists but screen not implemented
- Can reuse RecoveryWords component

### 9.4 Phase 6: TapSigner/Hardware Wallets

**Status:** Major phase planned
**Scope:**
- 11 screens for TapSigner setup/import
- NFC integration
- PIN entry with custom number pad
- TapSigner state management
- Backup/restore flows

**Current State:**
- FFI bindings exist (cove_tap_card.kt)
- No Android NFC integration yet
- No UI screens implemented

### 9.5 Other Deferred Items

**Lock/Terms Screens:**
- Current: Simple placeholders
- Needed: Full PIN entry, biometric auth, terms display

**Security Settings:**
- PIN enable/disable
- Biometric authentication
- Decoy wallet/PIN setup

**Advanced Node Configuration:**
- Custom Electrum URL
- Custom Esplora URL
- Connection testing

**Wallet List Settings:**
- AllWallets route not implemented
- Wallet sorting/filtering

---

## Summary Statistics

### Implementation Status by Phase

| Phase | Description | Deviations | Status |
|-------|-------------|-----------|---------|
| 1 | Core Managers | 8 major | ✅ Complete |
| 2 | Wallet/Send Managers | 7 major | ✅ Complete |
| 3 | Navigation | 7 major | ✅ Complete |
| 4 | Screens/Containers | 5 major | ✅ Complete |
| 5A | Hot Wallet Flow | 4 major | ✅ Complete |
| 5B | Settings | 4 major | ✅ Complete |
| 5C | Sheets/Alerts | - | 🔜 Next |
| 5D | Transaction Details | - | 📋 Planned |
| 5E | Secret Words | - | 📋 Planned |
| 6 | TapSigner/Hardware | - | 📋 Major phase |

### Deviation Categories

- **Architectural** (17): Core patterns that differ between platforms
- **UI/UX** (12): Visual and interaction differences
- **Deferred** (9): Features not yet implemented on Android
- **FFI/Platform** (5): Platform integration differences
- **API Differences** (8): Same functionality, different APIs

**Total Documented Deviations:** 51

---

## Guidelines for Future Development

### When to Deviate from iOS

✅ **Good Reasons:**
- Platform conventions (Material Design vs Human Interface Guidelines)
- Technical constraints (no direct API equivalent)
- Performance optimization (different platform characteristics)
- Kotlin/Compose idioms (sealed classes, coroutines, etc.)
- User expectations per platform

❌ **Bad Reasons:**
- "It's easier this way" without documenting
- Breaking Rust-first architecture
- Inconsistent state management patterns
- Skipping planned features without tracking

### Documentation Requirements

**When adding new deviations:**
1. Document in this file with clear reasoning
2. Add TODO markers in code for deferred features
3. Update PLAN_COMPLETED.md for the relevant phase
4. Note if deviation is temporary vs permanent

### Cross-Platform Consistency

**Must Stay Consistent:**
- FFI API usage (same Rust calls)
- Navigation through Rust router
- State reconciliation patterns
- Manager lifecycle patterns
- Data models and business logic

**Platform-Specific is OK:**
- UI components and layouts
- Animation implementations
- Platform permissions (Camera, NFC, etc.)
- Theme/color systems
- Gesture handling

---

**Document Maintained By:** Android implementation team
**Last Major Update:** Phase 5B completion (2025-10-17)
**Next Review:** After Phase 5C completion
