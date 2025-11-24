# Android iOS Parity Fixes - Implementation Plan

## Issues Summary (Quick Wins First)

| # | Issue | Complexity | Files |
|---|-------|------------|-------|
| 1 | Node spinner keeps spinning | Quick | `NodeSettingsScreen.kt` |
| 2 | Sidebar safe area overlap | Quick | `SidebarView.kt` |
| 3 | Receive button not implemented | Quick | `SelectedWalletContainer.kt` |
| 4 | Dark/Light mode not working | Medium | `MainActivity.kt`, `Theme.kt` |
| 5 | Send button missing validation | Medium | `SelectedWalletContainer.kt` |
| 6 | QR scanner dual back buttons | Medium | `QrCodeScanView.kt`, `ColdWalletQrScanScreen.kt` |
| 7 | QR scanner missing viewfinder | Medium | `QrCodeScanView.kt` |
| 8 | Sidebar gesture glitchy | Complex | `SidebarContainer.kt` |
| 9 | Save wallet infinite loop | Complex | `HotWalletCreateScreen.kt`, routing logic |
| 10 | Import wallet missing autocomplete | Complex | `HotWalletImportScreen.kt` (new component) |

---

## Fix 1: Node Spinner Keeps Spinning
**File:** `android/app/src/main/java/org/bitcoinppl/cove/settings/NodeSettingsScreen.kt`
**Action:** Investigate if there's a secondary loading state or race condition

---

## Fix 2: Sidebar Safe Area Overlap
**File:** `android/app/src/main/java/org/bitcoinppl/cove/sidebar/SidebarView.kt`
**Solution:** Add `WindowInsets.safeDrawing.asPaddingValues()` to root Column

---

## Fix 3: Receive Button Not Implemented
**Files:** `SelectedWalletContainer.kt`, `ReceiveAddressSheet.kt`
**Solution:** Add state variable and wire up existing sheet component

---

## Fix 4: Dark/Light Mode Not Working
**Files:** `MainActivity.kt`, `Theme.kt`
**Solution:** Pass `app.colorSchemeSelection` to `CoveTheme`

---

## Fix 5: Send Button Missing Validation
**File:** `SelectedWalletContainer.kt`
**Solution:** Check balance before navigating to send flow

---

## Fix 6: QR Scanner Dual Back Buttons
**Files:** `QrCodeScanView.kt`, `ColdWalletQrScanScreen.kt`
**Solution:** Remove TopAppBar from QrCodeScanView, let parents provide navigation

---

## Fix 7: QR Scanner Missing Viewfinder
**File:** `QrCodeScanView.kt`
**Solution:** Add viewfinder icon overlay centered on camera preview

---

## Fix 8: Sidebar Gesture Glitchy
**File:** `SidebarContainer.kt`
**Solution:** Replace with Material3 NavigationDrawer or debug double-trigger

---

## Fix 9: Save Wallet Infinite Loop
**Files:** `HotWalletCreateScreen.kt`, `NewHotWalletContainer.kt`, `AppManager.kt`
**Solution:** Investigate routing/reconciliation issue, compare with iOS

---

## Fix 10: Import Wallet Missing BIP39 Autocomplete
**Files:** `HotWalletImportScreen.kt`, new `Bip39AutocompleteDropdown.kt`
**Solution:** Create dropdown suggestion component using Rust BIP39 word list
