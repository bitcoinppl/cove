# iOS ↔ Android Parity Guide

This document covers platform-specific gotchas and patterns for achieving visual and behavioral parity between Android (Jetpack Compose) and iOS (SwiftUI).

## Table of Contents

- [List Key Serialization](#list-key-serialization)
- [Opacity / Alpha](#opacity--alpha)
- [Text Colors and Dark Mode](#text-colors-and-dark-mode)
- [Color Values](#color-values)
  - [Theme-Aware Custom Colors (CoveColorScheme)](#theme-aware-custom-colors-covecolorscheme)
- [Text Auto-Sizing](#text-auto-sizing)
- [Button Text Centering](#button-text-centering)
- [NFC Scanning UI](#nfc-scanning-ui)
- [Slider Step Behavior](#slider-step-behavior)
- [Cloud Backup Recovery](#cloud-backup-recovery)
- [Lifecycle and Effect Modifiers](#lifecycle-and-effect-modifiers)
  - [View Lifecycle](#view-lifecycle)
  - [Reactive Value Changes](#reactive-value-changes)
  - [State and Observation](#state-and-observation)
  - [Threading and Dispatch](#threading-and-dispatch)
  - [Sheets and Alerts](#sheets-and-alerts)
  - [Focus Management](#focus-management)
  - [Navigation](#navigation)

---

## List Key Serialization

- **Android**: `LazyColumn`'s `items(key = ...)` serializes keys to a `Bundle` for state restoration (configuration changes, process death). Keys must be primitive types (String, Int) or `Parcelable` objects.
- **iOS**: SwiftUI's `ForEach` uses `Identifiable` for in-memory diffing only—IDs are never serialized, so custom types like `TxId` work directly.
- **Guideline**: When using FFI types as list keys on Android, convert to String: `key = { it.id().toString() }`.

---

## Opacity / Alpha

- **Terminology**: Android/Compose uses `alpha`, iOS/SwiftUI uses `opacity`. Both mean the same thing (0 = transparent, 1 = opaque).
- **Container-level opacity**: On iOS, `.opacity(0.6)` applies to the entire view including its background. On Android, `Modifier.graphicsLayer { alpha = 0.6f }` only affects the composable's *content*, not modifiers like `.background()` applied to the same composable.
- **Guideline**: To match iOS's `.opacity()` behavior on Android, wrap the content in an outer Box with `graphicsLayer`:
  ```kotlin
  // Android - wrapper applies opacity to everything inside
  Box(modifier = Modifier.graphicsLayer { alpha = 0.6f }) {
      Box(modifier = Modifier.background(color)) {
          // content
      }
  }
  ```
  ```swift
  // iOS equivalent
  Box(...)
      .background(color)
      .opacity(0.6)
  ```

---

## Text Colors and Dark Mode

- **iOS/SwiftUI**: `Text` uses `.primary` foreground color by default, which automatically adapts to light/dark mode without explicit color specification.
- **Android/Compose**: `Text` uses `LocalContentColor.current` by default, but this must be provided by a parent composable. Without a provider, text may render as black regardless of theme.
- **Which composables set LocalContentColor?**
  - `Surface` → sets `LocalContentColor` to its `contentColor` parameter (defaults to `onSurface`)
  - `Scaffold` → sets appropriate content colors for each slot
  - `Column`/`Box` with `.background()` → does NOT set `LocalContentColor`
- **Guideline**: For content areas needing dark mode support, either use `Surface` instead of `Column` with `.background()`, or explicitly set `color = MaterialTheme.colorScheme.onSurface` on Text components.

---

## Color Values

- **Never hardcode colors**: Always use system-provided or theme-defined color values, never raw hex codes or Color literals.
- **Android**: Use `MaterialTheme.colorScheme.*` (e.g., `onSurface`, `primary`, `surfaceVariant`) or custom Cove colors via `MaterialTheme.coveColors.*`.
- **iOS**: Use system colors (`.primary`, `.secondary`) or custom colors from the asset catalog.
- **Why**: Hardcoded colors break dark mode, accessibility settings, and dynamic theming. Theme colors automatically adapt to light/dark mode and user preferences.

### Theme-Aware Custom Colors (CoveColorScheme)

For Cove-specific colors that need light/dark variants:

- **iOS**: Asset catalog `.colorset` files with light/dark appearances
- **Android**: `CoveColorScheme` in `Color.kt` with `LightCoveColors` and `DarkCoveColors` instances, provided via `CompositionLocal` in `CoveTheme`

**Guideline**: Add new theme-aware colors to `CoveColorScheme` in `Color.kt`. Access via `MaterialTheme.coveColors.*` (e.g., `MaterialTheme.coveColors.midnightBtn`).

---

## Text Auto-Sizing

iOS has built-in text shrinking via `minimumScaleFactor`. Android options:

### Native TextAutoSize (Preferred)

Use `BasicText` with `TextAutoSize` for simple auto-shrinking text:

```kotlin
import androidx.compose.foundation.text.BasicText
import androidx.compose.foundation.text.TextAutoSize

BasicText(
    text = "Text that shrinks to fit",
    maxLines = 1,
    autoSize = TextAutoSize.StepBased(minFontSize = 7.sp, maxFontSize = 14.sp, stepSize = 0.5.sp),
    style = TextStyle(color = LocalContentColor.current),
)
```

### Custom AutoSizeText

Use the custom implementations in `views/AutoSizeText.kt` for:
- `BalanceAutoSizeText` - balance displays with digit-based sizing
- `AutoSizeTextField` - editable auto-sizing text fields

**Requirement**: Parent must have bounded width (use `Modifier.fillMaxWidth()` on the container).

---

## Button Text Centering

- **iOS/SwiftUI**: Using `.frame(maxWidth: .infinity)` on a Text automatically centers it within the frame. Buttons styled with `PrimaryButtonStyle` get centered text by default.
- **Android/Compose**: `Modifier.fillMaxWidth()` on a Button makes it full-width, but the Text inside stays left-aligned by default.
- **Guideline**: For full-width buttons with centered text, add both properties to the Text inside the button:
  ```kotlin
  Button(
      onClick = { ... },
      modifier = Modifier.fillMaxWidth(),
  ) {
      Text(
          text = "Button Label",
          textAlign = TextAlign.Center,
          modifier = Modifier.fillMaxWidth(),
      )
  }
  ```
- **Note**: `ImageButton` handles text sizing internally using native `TextAutoSize`.

---

## NFC Scanning UI

- **iOS**: `NFCTagReaderSession` provides automatic system NFC popup. Messages display via `session.alertMessage` property.
- **Android**: `enableReaderMode` is silent—no system UI. Custom overlay required.

### Transport Protocol Messages

Both platforms implement `TapcardTransportProtocol` with `setMessage()` and `appendMessage()` (called by Rust during NFC operations to show progress):

**iOS** (`TapCardTransport` in `ios/Cove/TapSignerNFC.swift`):
```swift
func setMessage(message: String) {
    nfcSession.alertMessage = message
}

func appendMessage(message: String) {
    nfcSession.alertMessage = nfcSession.alertMessage + message
}
```

**Android** (`TapCardTransport` in `android/.../nfc/TapCardNfcManager.kt`):
```kotlin
override fun setMessage(message: String) {
    currentMessage = message
    onMessageUpdate?.invoke(currentMessage)
}

override fun appendMessage(message: String) {
    currentMessage += message
    onMessageUpdate?.invoke(currentMessage)
}
```

### Custom Overlay (Android only)

Since Android has no system NFC UI, `TapSignerScanningOverlay` composable provides visual feedback:
- NFC icon, animated "Scanning..." dots, message text, progress indicator
- Message updates via callback → `manager.scanMessage` state → recomposition
- Shown in `TapSignerContainer` when `manager.isScanning` is true

---

## Slider Step Behavior

- **iOS/SwiftUI**: `Slider(step:)` defines the **increment size** for a continuous slider
- **Android/Compose**: `Slider(steps:)` creates **discrete stop points** (N positions total)

**Critical**: These are not equivalent! Calculating `steps = (max - min) / stepSize` can create millions of discrete positions, causing severe lag/freeze.

**Guideline**: For continuous sliders matching iOS, omit `steps` entirely on Android. Handle step snapping in `onValueChange` if needed.

---

## Cloud Backup Recovery

Cloud Backup parity is behavioral rather than an attempt to hide provider
differences. iOS uses iCloud Drive and Android uses Google Drive, while Rust owns
the shared recovery contract.

### Shared ownership

- Rust owns inventory completeness, retained rows, refresh generations, enable
  lifecycle, passkey retry policy, Restore All eligibility/order/progress,
  cancellation, and row outcomes
- Swift and Kotlin project the exported state and dispatch user intent
- iOS owns FileManager, `NSMetadataQuery`, file coordination, and Authentication
  Services outcome classification
- Android owns Drive token/account binding, Drive API mechanics, and Credential
  Manager outcome classification; the generated Rust handle remains private to
  the guarded platform manager

Do not compensate for missing shared state in either UI. Add the state or action
to Rust and regenerate both platform bindings.

### Inventory parity

Both detail screens project `Checking`, `Complete`, and `Failed` without dropping
known wallet rows. Only `Complete` enables restore or destructive actions that
depend on an authoritative set. Retry requests are Rust-coordinated: one refresh
may run, one trailing request is retained, starts are at least five seconds
apart, and stale or closed-owner results are ignored.

Provider mechanics differ:

- iOS publishes a fast local snapshot as incomplete, then always completes a
  normalized FileManager-plus-metadata union
- Android's first successful Drive listing is an authoritative complete result
- Android silent startup never launches Drive consent; the explicit Restore from
  Cove Backup path may do so

Timeout, authorization, offline, and provider failures are incomplete on both
platforms and must never render as a confirmed empty backup.

### Enable and passkeys

Both platforms project the Rust-owned existing-passkey versus Create New Backup
decision. Pending namespace metadata is existing-passkey-only. Create New Backup
stages a fresh master, isolated namespace, and passkey until accepted remote
writes and durable local promotion are safe; it does not rewrap or overwrite the
retained active namespace.

Platform code must classify passkey presentation outcomes into typed results.
Only pre-presentation platform unavailability receives bounded automatic retry.
Presented failure, cancellation, mismatch, unsupported provider, invalid result,
and no credential do not retry automatically. Only the typed no-credential
result may continue into the owning enable or repair flow's explicit
registration step; every other failure stops. Interactive native sheets have
no global watchdog.

### Restore All parity

iOS and Android show the same Rust-owned inline states: Restore All with a count,
determinate completed/total progress and current wallet, cooperative Cancel, and
Retry Remaining. There is no confirmation dialog or terminal summary.

The batch uses the individual restore primitive sequentially with one
identity-aware session. Successes move immediately to their network sections;
ordinary row failures remain visible and directly retryable while later wallets
continue. Cancellation takes effect between wallets, and navigation away from
the detail screen does not cancel manager-owned work. After process death, a
namespace marker causes a refresh and Retry Remaining projection without
auto-resume or unsolicited passkey UI.

Use native controls and semantics on each platform: at least 44-point iOS and
48-dp Android targets, Dynamic Type/font scaling, non-color-only failures,
progress semantics/live updates, and combined wallet/action labels for
VoiceOver or TalkBack.

---

## Lifecycle and Effect Modifiers

SwiftUI and Compose have different APIs for lifecycle events and side effects. This section maps iOS patterns to their Android equivalents.

### View Lifecycle

| iOS (SwiftUI) | Android (Compose) | Notes |
|---------------|-------------------|-------|
| `.onAppear { }` | `LaunchedEffect(Unit) { }` | Runs once when composable enters composition |
| `.onDisappear { }` | `DisposableEffect(Unit) { onDispose { } }` | Cleanup runs when composable leaves composition |
| `.task { }` | `LaunchedEffect(Unit) { }` | For async work on appear |
| `.task(id:) { }` | `LaunchedEffect(id) { }` | Re-runs when `id` changes |

### Manager Cleanup

Route-level `DisposableEffect` cleanup should only close objects owned by that route instance. Do not close or clear managers obtained from an app-level cache such as `app.getWalletManager()` or `app.getSendFlowManager()` from route disposal. Navigation transitions can overlap old and new route entries, so an outgoing route can dispose after the incoming route has already reused the same generated UniFFI handle.

For cached managers, put cleanup at the owner/session boundary instead: clear the manager when the app route stack no longer contains the owning flow or wallet. Keep generated `manager.rust.*` calls behind platform manager wrapper methods so post-close calls become guarded no-ops or controlled failures instead of destroyed-handle crashes.

### Reactive Value Changes

| iOS (SwiftUI) | Android (Compose) | Notes |
|---------------|-------------------|-------|
| `.onChange(of: value) { }` | `LaunchedEffect(value) { }` | Runs when value changes |
| `.onChange(of: value, initial: true) { }` | `LaunchedEffect(value) { }` | LaunchedEffect always runs initially |
| `.onChange(of: value, initial: false) { }` | `LaunchedEffect` + `isFirstRun` flag | Use remembered boolean to skip initial run |

**Patterns**: To access old values, track `previousValue` in remembered state before updating.

### State and Observation

| iOS (SwiftUI) | Android (Compose) | Notes |
|---------------|-------------------|-------|
| `@State var x = ...` | `var x by remember { mutableStateOf(...) }` | Local component state |
| `@Binding var x` | `value: T, onValueChange: (T) -> Unit` | State hoisting pattern |
| `@Observable class` | `@Stable class` with `mutableStateOf` properties | Observable view model |
| `@ObservationIgnored` | Regular property (not `mutableStateOf`) | Non-observed property |
| `@Environment(\.key)` | `CompositionLocal` + `CompositionLocalProvider` | Dependency injection |

### Threading and Dispatch

| iOS (SwiftUI) | Android (Compose) | Notes |
|---------------|-------------------|-------|
| `DispatchQueue.main.async { }` | `mainScope.launch { }` | Post to main thread |
| `DispatchQueue(label:).async { }` | `launch(Dispatchers.IO) { }` | Background work |
| `Task { }` | `LaunchedEffect { }` or `rememberCoroutineScope()` | Structured concurrency |
| `Task.detached { }` | `CoroutineScope(Dispatchers.Default).launch { }` | Unstructured (avoid) |

### Sheets and Alerts

| iOS (SwiftUI) | Android (Compose) | Notes |
|---------------|-------------------|-------|
| `.sheet(isPresented:)` | `if (showSheet) ModalBottomSheet(...)` | Conditional composition |
| `.sheet(item:)` | `item?.let { ModalBottomSheet(...) }` | Item-based sheet |
| `.alert(isPresented:)` | `if (showAlert) AlertDialog(...)` | Conditional dialog |
| `.alert(item:)` | `alertItem?.let { AlertDialog(...) }` | Item-based alert |
| `.confirmationDialog()` | `DropdownMenu` or `AlertDialog` with options | Action sheet equivalent |

### Focus Management

| iOS (SwiftUI) | Android (Compose) | Notes |
|---------------|-------------------|-------|
| `@FocusState var field` | `val focusRequester = remember { FocusRequester() }` | Focus tracking |
| `.focused($field, equals: .x)` | `Modifier.focusRequester(focusRequester)` | Attach to field |
| `field = .x` | `focusRequester.requestFocus()` | Request focus |
| `.onSubmit { }` | `keyboardActions = KeyboardActions(onDone = { })` | Keyboard submit |

### Navigation

| iOS (SwiftUI) | Android (Compose) | Notes |
|---------------|-------------------|-------|
| `NavigationStack` | Navigation3 `NavDisplay` | Stack-based navigation |
| `@Environment(\.dismiss)` | `navController.popBackStack()` | Dismiss current screen |
| `.navigationDestination(for:)` | Route matching in `NavDisplay` | Type-safe routing |
