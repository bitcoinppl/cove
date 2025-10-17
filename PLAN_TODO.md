# Android Implementation TODO

> **Status:** Phase 5A In Progress
> **Last Updated:** 2025-10-17

## Completed Phases

1. ✅ Bootstrap Kotlin App Shell and Core Managers
2. ✅ Implement Kotlin Counterparts for Wallet & Send Managers
3. ✅ Setup Navigation (Rust-First)
4. ✅ Wire Compose Screens to Real Managers and Routes

---

5. **Phase 5A: Hot Wallet Creation & Verification Flow** 🚧 IN PROGRESS
   **Priority:** Core user onboarding, well-defined in iOS, uses existing `PendingWalletManager`

   **IMPORTANT DISCOVERY:** Most screens already exist! Only need to wire containers and create 2 missing screens.

   **Screens Status:**
   - ✅ `HotWalletSelectScreen.kt` - EXISTS, updated to use AppManager + modal sheet for word count selection
   - ✅ `HotWalletCreateScreen.kt` - EXISTS, updated to use PendingWalletManager + HorizontalPager with save wallet functionality
   - ✅ `HotWalletVerifyScreen.kt` - **FULLY EXISTS** with 463 lines of interactive verification + flying chip animations!
   - ❌ `HotWalletImportScreen.kt` - MISSING, needs creation (text input for mnemonic import)
   - ❌ Completion/Success screen - MISSING, needs simple success screen

   **What We Did:**
   - Modified `HotWalletSelectScreen.kt` - Added modal bottom sheet, AppManager integration, RouteFactory navigation
   - Modified `HotWalletCreateScreen.kt` - Added HorizontalPager for word groups, back confirmation dialog, wallet save + navigate to verify

   **Remaining Work:**
   - Create `HotWalletImportScreen.kt` (simple text input)
   - Create success/completion screen
   - Wire `NewHotWalletContainer.kt` to route to all screens
   - Test end-to-end flow

   **Actual Effort:** Much less than estimated - ~2 screens to create, ~50 lines container wiring

6. **Phase 5B: Settings Screens**
   **Priority:** User customization, relatively independent

   **Screens to Implement:**
   - `NetworkSettingsScreen.kt` - Network selection with warning dialog
   - `AppearanceSettingsScreen.kt` - Theme selection
   - `NodeSettingsScreen.kt` - Node configuration
   - `FiatCurrencySettingsScreen.kt` - Currency picker
   - `WalletSettingsMainScreen.kt` - Main wallet settings
   - `WalletChangeNameScreen.kt` - Rename wallet
   - `WalletChangeColorScreen.kt` - Change wallet color

   **Components:** Picker/Selector component (reusable)
   **Estimated:** 7 screens, ~500-700 lines

7. **Phase 5C: Sheet & Alert System**
   **Priority:** Required for Send flow completion and error handling

   **Implementation:**
   - Global sheet rendering in `CoveApp.kt` (QR scanner, fee selector)
   - Global alert rendering in `CoveApp.kt` (all AppAlertState types)
   - SendFlow-specific sheets and alerts

   **Components:** `CoveAlertDialog.kt`, `QrScannerSheet.kt`, `FeeRateSelectorSheet.kt`
   **Estimated:** ~300-400 lines

8. **Phase 5D: Transaction Details Screen**
   **Priority:** Completes wallet transaction viewing

   **Screens:** `TransactionDetailsScreen.kt` with confirmation status, amounts, addresses
   **Components:** `ConfirmationIndicatorView.kt`, `TransactionDetailsRow.kt`
   **Estimated:** 2-3 components, ~200-300 lines

9. **Phase 5E: Secret Words Viewing**
   **Priority:** Sensitive feature, requires auth

   **Screens:** `SecretWordsScreen.kt` with auth guard, mnemonic display, warnings
   **Estimated:** 1 screen, ~150-200 lines

10. **Phase 6: Hardware Wallet (TapSigner) Flow**
    **Priority:** Advanced feature, most complex, requires NFC

    **Implementation:** 11 screens for TapSigner setup/import, `TapSignerManager.kt`, NFC integration
    **Components:** `NumberPadPinView.kt` (PIN entry with custom number pad)
    **Estimated:** 11 screens + 3 components, ~1200-1500 lines
5. **Phase 5A: Hot Wallet Creation & Verification Flow** 🚧 IN PROGRESS
   **Priority:** Core user onboarding, well-defined in iOS, uses existing `PendingWalletManager`

   **IMPORTANT DISCOVERY:** Most screens already exist! Only need to wire containers and create 2 missing screens.

   **Screens Status:**
   - ✅ `HotWalletSelectScreen.kt` - EXISTS, updated to use AppManager + modal sheet for word count selection
   - ✅ `HotWalletCreateScreen.kt` - EXISTS, updated to use PendingWalletManager + HorizontalPager with save wallet functionality
   - ✅ `HotWalletVerifyScreen.kt` - **FULLY EXISTS** with 463 lines of interactive verification + flying chip animations!
   - ❌ `HotWalletImportScreen.kt` - MISSING, needs creation (text input for mnemonic import)
   - ❌ Completion/Success screen - MISSING, needs simple success screen

   **What We Did:**
   - Modified `HotWalletSelectScreen.kt` - Added modal bottom sheet, AppManager integration, RouteFactory navigation
   - Modified `HotWalletCreateScreen.kt` - Added HorizontalPager for word groups, back confirmation dialog, wallet save + navigate to verify

   **Remaining Work:**
   - Create `HotWalletImportScreen.kt` (simple text input)
   - Create success/completion screen
   - Wire `NewHotWalletContainer.kt` to route to all screens
   - Test end-to-end flow

   **Actual Effort:** Much less than estimated - ~2 screens to create, ~50 lines container wiring

6. **Phase 5B: Settings Screens**
   **Priority:** User customization, relatively independent

   **Screens to Implement:**
   - `NetworkSettingsScreen.kt` - Network selection with warning dialog
   - `AppearanceSettingsScreen.kt` - Theme selection
   - `NodeSettingsScreen.kt` - Node configuration
   - `FiatCurrencySettingsScreen.kt` - Currency picker
   - `WalletSettingsMainScreen.kt` - Main wallet settings
   - `WalletChangeNameScreen.kt` - Rename wallet
   - `WalletChangeColorScreen.kt` - Change wallet color

   **Components:** Picker/Selector component (reusable)
   **Estimated:** 7 screens, ~500-700 lines

7. **Phase 5C: Sheet & Alert System**
   **Priority:** Required for Send flow completion and error handling

   **Implementation:**
   - Global sheet rendering in `CoveApp.kt` (QR scanner, fee selector)
   - Global alert rendering in `CoveApp.kt` (all AppAlertState types)
   - SendFlow-specific sheets and alerts

   **Components:** `CoveAlertDialog.kt`, `QrScannerSheet.kt`, `FeeRateSelectorSheet.kt`
   **Estimated:** ~300-400 lines

8. **Phase 5D: Transaction Details Screen**
   **Priority:** Completes wallet transaction viewing

   **Screens:** `TransactionDetailsScreen.kt` with confirmation status, amounts, addresses
   **Components:** `ConfirmationIndicatorView.kt`, `TransactionDetailsRow.kt`
   **Estimated:** 2-3 components, ~200-300 lines

9. **Phase 5E: Secret Words Viewing**
   **Priority:** Sensitive feature, requires auth

   **Screens:** `SecretWordsScreen.kt` with auth guard, mnemonic display, warnings
   **Estimated:** 1 screen, ~150-200 lines

10. **Phase 6: Hardware Wallet (TapSigner) Flow**
    **Priority:** Advanced feature, most complex, requires NFC

    **Implementation:** 11 screens for TapSigner setup/import, `TapSignerManager.kt`, NFC integration
    **Components:** `NumberPadPinView.kt` (PIN entry with custom number pad)
    **Estimated:** 11 screens + 3 components, ~1200-1500 lines
