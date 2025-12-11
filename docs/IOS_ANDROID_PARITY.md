# iOS ↔ Android Parity Guide

This document covers platform-specific gotchas and patterns for achieving visual and behavioral parity between Android (Jetpack Compose) and iOS (SwiftUI).

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
- **Android**: Use `MaterialTheme.colorScheme.*` (e.g., `onSurface`, `primary`, `surfaceVariant`) or custom colors defined in `Theme.kt`.
- **iOS**: Use system colors (`.primary`, `.secondary`) or custom colors from the asset catalog.
- **Why**: Hardcoded colors break dark mode, accessibility settings, and dynamic theming. Theme colors automatically adapt to light/dark mode and user preferences.

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
