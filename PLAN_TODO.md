# Android Implementation TODO

> **Status:** Phase 5B Complete ✅ | Ready for Phase 5C
> **Last Updated:** 2025-10-17

## Completed Phases

1. ✅ Bootstrap Kotlin App Shell and Core Managers
2. ✅ Implement Kotlin Counterparts for Wallet & Send Managers
3. ✅ Setup Navigation (Rust-First)
4. ✅ Wire Compose Screens to Real Managers and Routes
5. ✅ Phase 5A: Hot Wallet Creation & Verification Flow
6. ✅ Phase 5B: Settings Screens

---

## Remaining Phases

### Phase 5C: Sheet & Alert System
**Priority:** Required for Send flow completion and error handling
**Status:** 🔜 NEXT

**Implementation:**
- Global sheet rendering in `CoveApp.kt` (QR scanner, fee selector)
- Global alert rendering in `CoveApp.kt` (all AppAlertState types)
- SendFlow-specific sheets and alerts

**Components:** `CoveAlertDialog.kt`, `QrScannerSheet.kt`, `FeeRateSelectorSheet.kt`
**Estimated:** ~300-400 lines

### Phase 5D: Transaction Details Screen
**Priority:** Completes wallet transaction viewing

**Screens:** `TransactionDetailsScreen.kt` with confirmation status, amounts, addresses
**Components:** `ConfirmationIndicatorView.kt`, `TransactionDetailsRow.kt`
**Estimated:** 2-3 components, ~200-300 lines

### Phase 5E: Secret Words Viewing
**Priority:** Sensitive feature, requires auth

**Screens:** `SecretWordsScreen.kt` with auth guard, mnemonic display, warnings
**Estimated:** 1 screen, ~150-200 lines

### Phase 6: Hardware Wallet (TapSigner) Flow
**Priority:** Advanced feature, most complex, requires NFC

**Implementation:** 11 screens for TapSigner setup/import, `TapSignerManager.kt`, NFC integration
**Components:** `NumberPadPinView.kt` (PIN entry with custom number pad)
**Estimated:** 11 screens + 3 components, ~1200-1500 lines
