# Android iOS Parity - Context & Learnings

## Architecture Overview

- **Rust core** in `rust/` - single source of truth for wallet logic
- **UniFFI bindings** generate Swift/Kotlin from Rust
- **iOS** uses SwiftUI with `@Observable` managers
- **Android** uses Jetpack Compose with same manager pattern
- Both aim for "consistent architecture, platform-native experience"

---

## Key Files Reference

### Android
- `MainActivity.kt` - App entry, CoveTheme wrapper, sheet handling
- `AppManager.kt` - Singleton state manager, routing, reconciliation
- `SidebarContainer.kt` - Custom drawer with gesture handling
- `SidebarView.kt` - Sidebar content (logo, wallets, settings)
- `SelectedWalletContainer.kt` - Wallet screen with send/receive
- `QrCodeScanView.kt` - Camera QR scanner
- `HotWalletCreateScreen.kt` - Wallet creation flow
- `HotWalletImportScreen.kt` - Import wallet with word input
- `NodeSettingsScreen.kt` - Node selection settings
- `AppearanceSettingsScreen.kt` - Theme settings
- `Theme.kt` - CoveTheme with color schemes

### iOS (for comparison)
- `SelectedWalletContainer.swift` - iOS receive/send patterns
- `HotWalletCreateScreen.swift` - iOS save wallet flow
- `HotWalletImportScreen.swift` - iOS autocomplete implementation
- `ScannerView.swift` - iOS viewfinder implementation

---

## Key Findings

### Sidebar Issues
- Custom gesture handler with 90% dampening (SidebarContainer.kt:127)
- Edge detection from left 25dp only (SidebarContainer.kt:100)
- No WindowInsets padding - content goes under system bars
- Settings button at bottom overlaps navigation bar

### QR Scanner Issues
- Both ColdWalletQrScanScreen AND QrCodeScanView have TopAppBar
- When nested, both back buttons appear
- iOS has viewfinder icon, Android doesn't

### Theme Issues
- CoveTheme called without darkTheme parameter in MainActivity
- Uses `isSystemInDarkTheme()` default
- `app.colorSchemeSelection` tracked but never used by theme

### Receive Button
- Handler is literally `// TODO: implement receive address screen/sheet`
- ReceiveAddressSheet.kt already exists and works

### Save Wallet Loop
- Calls `app.resetRoute()` with nested routes
- May trigger re-creation of container, causing loop
- Need to compare with iOS implementation

### BIP39 Autocomplete
- iOS has `Bip39AutoComplete` class and keyboard accessory
- Android has plain BasicTextField with no suggestions
- User prefers dropdown approach (not keyboard accessory)

---

## Questions/Unknowns

1. **Node spinner** - Code looks correct with `finally { isLoading = false }`. Need to verify if there's a race condition or secondary state.

2. **Sidebar gesture** - Replace with Material3 NavigationDrawer, or debug existing? Need to verify design requirements.

3. **Save wallet loop** - Need to investigate exact routing flow and compare with iOS.

---

## Rust FFI Types Used

- `ColorSchemeSelection` - DARK, LIGHT, SYSTEM enum
- `WalletBalance` - Has `confirmedBalance()` method
- `Route`, `SendRoute`, `HotWalletRoute` - Navigation types
- `NodeSelector`, `NodeSelection` - Node settings
- BIP39 word list available from `cove-bip39` crate

---

## Android Patterns to Follow

- State: Use `mutableStateOf` with callbacks (not MutableState params)
- Sheets: `ModalBottomSheet` with `rememberModalBottomSheetState`
- Insets: `WindowInsets.safeDrawing.asPaddingValues()`
- Coroutines: `rememberCoroutineScope()` + `launch { }`
- IO work: `withContext(Dispatchers.IO) { }`
