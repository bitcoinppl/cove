# Important Context & Learnings

> **How to Use These Plan Files:**
> - `PLAN_IMPORTANT_CONTEXT_AND_LEARNINGS.md` (this file) - Read FIRST before starting any new phase. Contains critical discoveries and patterns.
> - `PLAN_TODO.md` - Check for remaining work. Update as phases complete.
> - `PLAN_COMPLETED.md` - Historical reference only. Don't modify unless documenting completed work.
>
> **Workflow:** Read Context → Check TODO → Implement → Update TODO → Document in Completed

---

## Key Discovery: Android UI is More Complete Than Expected

**CRITICAL INSIGHT:** The Android Compose UI has significantly more components already built than initially estimated. **ALWAYS check for existing screens/components BEFORE implementing from scratch.**

### Efficiency Gains from Discovery Process
- Phase 5A estimated 5-7 new screens (~600 lines)
- Actually only needed 2 new screens (~150 lines) + wiring existing ones
- **75% reduction in work** due to existing implementations

### What Already Exists in Android Codebase

#### 1. Complete Screens with Full Functionality
- Most hot wallet flow screens exist with full functionality
- `HotWalletVerifyScreen.kt` has 463 lines with complex flying chip animations
- Verification logic with state machines already implemented
- Many settings screens likely exist (need discovery per phase)

#### 2. Reusable UI Components Available
- `RecoveryWords.kt` - Full pager with word grids, selection, dots indicator
- `DashDotsIndicator.kt` & `DotsIndicator.kt` - Pagination indicators
- `ImageButton.kt` - Styled buttons matching iOS
- `SettingsItem.kt` - Settings list items with icons/toggles
- `RoundRectIcon.kt` / `RoundRectImage.kt` - Styled icon containers
- `CustomBaseViews.kt` - ThemedSwitch, InfoRow, CardItem, SwitchRow
- `FullPageLoadingView.kt` - Loading states

#### 3. Comprehensive Theme System
- 84+ colors defined including wallet accent colors
- Comprehensive Material3 theming with dark/light mode support
- `WalletColorExt.kt` for FFI WalletColor → Compose Color conversion
- Color categories: Primary, Neutral, Wallet accents, Transaction-specific, List colors

---

## Recommended Implementation Workflow

### BEFORE Starting Any Phase:
1. **Search for existing files:** `find` or `Glob` with name patterns
2. **Read existing implementations:** Understand what's already there
3. **Check git status:** See what's been modified vs what's new
4. **Verify non-existence:** Only create new files when confirmed missing
5. **Prefer enhancement:** Modify/enhance existing screens over rewriting

### Example Search Commands:
```bash
# Find screens by pattern
find android/app/src -name "*Settings*Screen.kt"
find android/app/src -name "*Import*Screen.kt"

# Or use Glob
Glob pattern="**/flow/**/Settings*.kt"
Glob pattern="**/views/*.kt"
```

---

## Android-Specific Patterns (Compose + Rust FFI)

### 1. Manager Integration Pattern
- Screens accept `AppManager`, `WalletManager`, etc. as parameters
- No mock data or callbacks - managers provide all state
- Example: `fun MyScreen(app: AppManager, manager: WalletManager)`

### 2. Navigation Pattern
- Always use: `app.pushRoute(RouteFactory().someRoute())`
- For back: `app.popRoute()`
- Never: `NavController` or manual navigation

### 3. State Management Pattern
```kotlin
// In screens/components
var state by remember { mutableStateOf(initialValue) }

// With side effects
LaunchedEffect(key) {
    // Side effect here
}

// In managers
var observableState by mutableStateOf(value)
```

### 4. Modal Sheets Pattern
```kotlin
var showSheet by remember { mutableStateOf(false) }
val sheetState = rememberModalBottomSheetState()

if (showSheet) {
    ModalBottomSheet(
        onDismissRequest = { showSheet = false },
        sheetState = sheetState
    ) {
        // Content
    }
}
```

### 5. Alerts/Dialogs Pattern
```kotlin
var showAlert by remember { mutableStateOf(false) }

if (showAlert) {
    AlertDialog(
        onDismissRequest = { showAlert = false },
        title = { Text("Title") },
        text = { Text("Message") },
        confirmButton = { TextButton(...) },
        dismissButton = { TextButton(...) }
    )
}
```

### 6. Animations Pattern
```kotlin
val animatable = remember { Animatable(0f) }

LaunchedEffect(trigger) {
    animatable.animateTo(
        targetValue = 1f,
        animationSpec = tween(300, easing = LinearEasing)
    )
}
```

### 7. Paging Pattern
```kotlin
val pagerState = rememberPagerState(pageCount = { pages.size })

HorizontalPager(state = pagerState) { page ->
    // Page content
}
```

---

## iOS to Android Translation Quick Reference

| iOS Pattern | Android Equivalent | Notes |
|-------------|-------------------|-------|
| `TabView` | `HorizontalPager` | Need `rememberPagerState()` |
| `NavigationLink` | `app.pushRoute()` | Through Rust router |
| `.alert()` modifier | `AlertDialog` composable | With state boolean |
| `.sheet()` modifier | `ModalBottomSheet` | With state boolean |
| `@Observable` | `mutableStateOf()` | In managers |
| `Task.sleep` | `delay()` | In `LaunchedEffect` |
| `withAnimation` | `animateTo()` | Use `Animatable` |
| `.task { }` | `LaunchedEffect` | For side effects |
| `@State` | `remember { mutableStateOf() }` | Local state |
| `@Environment` | Parameter passing | Pass managers explicitly |

### FFI Types (Same in Both):
- `GroupedWord` - Available in Kotlin
- `PendingWalletManager` - Same pattern with reconciler
- `RouteFactory()` - Same FFI bindings
- `NumberOfBip39Words` - Same enum
- `ImportType` - Same enum

---

## Container Architecture Patterns

### Three Container Types:

#### 1. Lifecycle Containers
**Purpose:** Manage complex state with manager lifecycle

**Pattern:**
```kotlin
@Composable
fun SomeFlowContainer(app: AppManager, id: WalletId) {
    var manager by remember { mutableStateOf<SomeManager?>(null) }
    var loading by remember { mutableStateOf(true) }

    LaunchedEffect(Unit) {
        manager = app.getSomeManager(id)
        loading = false
    }

    DisposableEffect(Unit) {
        onDispose { manager?.cleanup() }
    }

    when {
        loading -> FullPageLoadingView()
        manager != null -> SomeScreen(app, manager!!)
        else -> ErrorState()
    }
}
```

**Examples:** SendFlowContainer, CoinControlContainer, SelectedWalletContainer

#### 2. Router Containers
**Purpose:** Lightweight routing, no manager initialization

**Pattern:**
```kotlin
@Composable
fun SomeRouterContainer(app: AppManager, route: SomeRoute) {
    when (route) {
        is SomeRoute.TypeA -> ScreenA(app)
        is SomeRoute.TypeB -> ScreenB(app)
        // ...
    }
}
```

**Examples:** SettingsContainer, NewWalletContainer, NewHotWalletContainer

#### 3. Hybrid Containers
**Purpose:** Router + lazy manager loading

**Pattern:**
```kotlin
@Composable
fun HybridContainer(app: AppManager, route: SomeRoute) {
    when (route) {
        is SomeRoute.NeedsManager -> {
            // Lazy load manager
            LifecycleContainer(app, route.id)
        }
        is SomeRoute.Simple -> {
            // Direct screen
            SimpleScreen(app)
        }
    }
}
```

**Examples:** WalletSettingsContainer

---

## Next Phases - Expected Findings

Based on Phase 5A discoveries, **expect these phases to have existing implementations:**

### Phase 5B (Settings):
- **Estimate:** 50%+ of screens probably exist
- **Action:** Search for `*Settings*Screen.kt` files first
- **Components:** `SettingsItem.kt` already exists

### Phase 5C (Sheets/Alerts):
- **Estimate:** Infrastructure exists, just needs wiring
- **Action:** Check `CoveApp.kt` for existing sheet/alert handling
- **Components:** AlertDialog and ModalBottomSheet patterns known

### Phase 5D (Transaction Details):
- **Estimate:** Base screen likely exists, needs enhancement
- **Action:** Search for `*Transaction*Screen.kt` files
- **Components:** May have transaction-related components

### Phase 5E (Secret Words):
- **Estimate:** Can reuse `RecoveryWords.kt` component (confirmed exists)
- **Action:** Just need auth guard + screen wrapper
- **Effort:** Minimal, mostly wiring

### Phase 6 (TapSigner):
- **Estimate:** Unknown, but NFC infrastructure exists
- **Action:** Check `cove_tap_card.kt` and search for TapSigner files
- **Components:** May have PIN input components

---

## Common Pitfalls to Avoid

### 1. Don't Assume Nothing Exists
❌ **Wrong:** "This screen isn't in the plan, so I'll create it from scratch"
✅ **Right:** "Let me search for this screen first: `find android/app/src -name "*ScreenName*.kt"`"

### 2. Don't Ignore Existing Patterns
❌ **Wrong:** Using `NavController` for navigation
✅ **Right:** Using `app.pushRoute(RouteFactory()...)`

### 3. Don't Mix State Management Patterns
❌ **Wrong:** Using `LiveData` or custom observables
✅ **Right:** Using `mutableStateOf()` in managers, `remember { mutableStateOf() }` in screens

### 4. Don't Skip Discovery Phase
❌ **Wrong:** Start coding immediately
✅ **Right:** Search → Read → Verify → Then code

### 5. Don't Forget Cleanup
❌ **Wrong:** Managers without `DisposableEffect`
✅ **Right:** Always add cleanup in `DisposableEffect` for managers

---

## Git Workflow Notes

### Check What's Modified vs New:
```bash
git status              # See modified files
git diff <file>         # See what changed
git diff --name-only    # Just file names
```

### Understanding Changes:
- Modified files = Already existed, we enhanced them
- New files = We created from scratch
- Use this to understand what was already there

---

## Success Metrics

### How to Know You're Following the Pattern:

✅ **Good Signs:**
- You searched for existing files before creating new ones
- You found 50%+ of expected screens already exist
- You're modifying/enhancing more than creating
- You're using managers passed as parameters
- You're using `app.pushRoute()` for navigation
- You're using `mutableStateOf()` for state

❌ **Warning Signs:**
- Creating many files from scratch without searching first
- Using callbacks instead of managers
- Using `NavController` or custom navigation
- Not following existing component patterns
- Ignoring existing theme colors

---

## Questions to Ask Before Implementing

Before starting any new screen/component:

1. **Does this file already exist?** (Search first!)
2. **Can I reuse an existing component?** (Check components/)
3. **What pattern does this follow?** (Lifecycle/Router/Hybrid container?)
4. **How does iOS do this?** (Check ios/Cove/Flows/)
5. **What managers do I need?** (AppManager? WalletManager? Custom?)
6. **Is there a similar screen I can reference?** (Look at existing screens)

---

**Last Updated:** 2025-10-17
**Next Review:** When starting new phase - update with new discoveries
