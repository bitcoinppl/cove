# Android iOS Parity - Progress Tracker

## Status Legend
- ⬜ Not started
- 🔄 In progress
- ✅ Completed
- ❌ Blocked

---

## Quick Wins

| # | Issue | Status | Notes |
|---|-------|--------|-------|
| 1 | Node spinner | ✅ | Fixed: showSnackbar blocking finally block |
| 2 | Sidebar safe area | ✅ | Added WindowInsets.safeDrawing padding |
| 3 | Receive button | ✅ | Wired up existing ReceiveAddressSheet |
| 4 | Dark/Light mode | ✅ | Pass colorSchemeSelection to CoveTheme |
| 5 | Send validation | ✅ | Check balance before navigation |

## Medium Complexity

| # | Issue | Status | Notes |
|---|-------|--------|-------|
| 6 | QR dual back buttons | ✅ | Added showTopBar param to QrCodeScanView |
| 7 | QR viewfinder | ✅ | Added CropFree icon overlay |

## Complex

| # | Issue | Status | Notes |
|---|-------|--------|-------|
| 8 | Sidebar gesture | ✅ | Spring animation, wider edge threshold, state guards |
| 9 | Save wallet loop | ✅ | Added isSaving guard, disabled button while saving |
| 10 | BIP39 autocomplete | ✅ | Dropdown suggestions below input field |

---

## Change Log

### Session 1 - All Fixes Complete
- All 10 fixes implemented and build successful
- Complex fixes completed:
  - Sidebar: Spring animation, 50dp edge threshold, 30% open threshold
  - Save wallet: isSaving guard prevents multiple calls
  - Autocomplete: Dropdown with suggestions from Bip39WordSpecificAutocomplete

### Session 1 - Implementation
- Completed 7 quick/medium fixes
- Build successful after all changes

### Session 1 - Initial Planning
- Created plan from user's defect list
- Identified 10 issues
- Organized by complexity (quick wins first)
- User chose dropdown suggestions for autocomplete (not keyboard accessory)
