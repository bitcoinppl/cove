# Navigation Plan

This plan consolidates the iOS baseline work and the updated Android guidance into one Rust-first navigation reference.

## Architecture Overview

### Core Principles
- Rust owns every navigation decision (`Route`, `Router`, `AppAction`); mobile layers mirror state.
- `FfiReconcile` broadcasts router/auth/database changes; Swift/Kotlin react via immutable snapshots.
- UI dispatches navigation intents (`push`, `reset`, `loadAndReset`) and waits for Rust to echo the resulting state.
- Immutable data + structural equality policies prevent Compose/SwiftUI feedback loops.

### Data Flow

```
User Action → UI dispatch(AppAction) → Rust mutation → reconcile(message) → UI state update → Recompose
     ↑                                                                                ↓
     └──────────────────── System back / deep link / process restore ─────────────────┘
```

## Reference Implementations

### iOS Baseline
- `ios/Cove/CoveApp.swift:476-519` binds `NavigationStack(path:)` to `app.router.routes` so SwiftUI stays in sync with Rust.
- `ios/Cove/RouteView.swift:35-121` renders screens via a `switch` on the shared `Route` enum.
- `ios/Cove/AppManager.swift:4-279` owns the singleton `FfiApp`, caches the router, and implements `FfiReconcile`.
- `ios/Cove/AuthManager.swift:7-161` mirrors Rust auth state, presents lock/cover flows, and triggers app resets.

### Rust Core
- Types exported through uniffi: `Route`, `Router`, helpers (`rust/src/router.rs:17-195`).
- `App` + `AppAction` mutate the router and emit `AppStateReconcileMessage` (`rust/src/app.rs:32-223`).
- `FfiApp` bridges UI commands (`dispatch`, `reset_default_route_to`, `listen_for_updates`) (`rust/src/app.rs:241-399`).
- `FfiReconcile` plumbing keeps callbacks thread-safe (`rust/src/app/reconcile.rs:9-60`).

## Android/Compose Strategy

### Recommended: State-Driven `AppManager`
- Implement an `@Stable` singleton that wraps `FfiApp`, exposes `router` via `mutableStateOf(..., structuralEqualityPolicy())`, and mirrors other reconciled state (database, color scheme, auth flags).
- Always create new `Router` instances (`Router(ffiApp, default, routes.toList())`) inside `reconcile` so Compose detects changes.

```kotlin
// android/app/src/main/java/org/bitcoinppl/cove/app/AppManager.kt
@Stable
class AppManager private constructor() : FfiReconcile {
    companion object { val shared by lazy { AppManager() } }

    private val ffiApp = FfiApp()
    var router by mutableStateOf(ffiApp.state().router, structuralEqualityPolicy())
        private set
    var database by mutableStateOf(Database())
        private set
    var isTermsAccepted by mutableStateOf(Database().globalFlag().isTermsAccepted())
        private set

    init { ffiApp.listenForUpdates(this) }

    override fun reconcile(message: AppStateReconcileMessage) {
        when (message) {
            is AppStateReconcileMessage.RouteUpdated ->
                router = Router(ffiApp, router.`default`, message.v1.toList())
            is AppStateReconcileMessage.PushedRoute ->
                router = Router(ffiApp, router.`default`, router.routes + message.v1)
            is AppStateReconcileMessage.DefaultRouteChanged ->
                router = Router(ffiApp, message.v1, message.v2.toList())
            is AppStateReconcileMessage.DatabaseUpdated ->
                database = Database()
            is AppStateReconcileMessage.AcceptedTerms ->
                isTermsAccepted = true
            // Handle color scheme, selected network, auth, etc.
        }
    }

    fun pushRoute(route: Route) = ffiApp.dispatch(AppAction.PushRoute(route))
    fun setRoutes(routes: List<Route>) = ffiApp.dispatch(AppAction.UpdateRoute(routes))
}
```

### Route → UI Mapping
- Mirror `RouteView` with a `when` on `router.routes.lastOrNull() ?: router.default`.
- Provide helpers such as `pushRoute`, `pushRoutes`, `resetRoute`, `loadAndReset` so screens can delegate navigation through Rust.

```kotlin
@Composable
fun CoveAppRoot(app: AppManager = remember { AppManager.shared }) {
    val router = app.router
    val current = router.routes.lastOrNull() ?: router.`default`

    when (current) {
        Route.ListWallets -> ListWalletsScreen(onWallet = { id ->
            app.pushRoute(Route.SelectedWallet(id))
        })
        is Route.SelectedWallet -> SelectedWalletScreen(current.v1)
        is Route.NewWallet -> NewWalletFlow(current.v1, app::pushRoute)
        is Route.Settings -> SettingsContainer(current.v1, app)
        // Handle SecretWords, Send, CoinControl, TapSigner, etc.
    }
}
```

### Auth & Cover Flow
- Port `AuthManager` so biometric toggles, decoy/wipe pins, and lock state match Swift.
- Wrap `CoveAppRoot` in a lock surface until terms are accepted and auth succeeds; call `ffiApp.initOnStart()` on launch.
- Ensure `AppManager.reset()` clears cached managers (wallet, send flow) before dispatching `loadAndReset` routes.

### Navigation Sync Guardrails
- Only dispatch `AppAction.UpdateRoute` when the requested stack differs from `router.routes` to avoid Rust ↔ Compose ping-pong.
- If you adopt `NavHostController`, keep reversible `routeToString` / `stringToRoute` helpers in one file and compare stacks before calling `navigate`/`popBackStack`.
- Use `BackHandler` so hardware back emits a trimmed stack (`routes.dropLast(1)`) through Rust when Rust is authoritative.
- Always copy lists (`toList()`) inside reconcile so Compose sees new references.

### Deep Links, Process Death, Multi-Stack
- On deep links or cold starts, normalize the initial Android back stack into `List<Route>` and dispatch it once so Rust matches what the user sees.
- Persist the authoritative router snapshot (Rust) for process-death restoration and rehydrate before composing UI.
- If tabs/sidebars require multiple stacks, extend `Router` with `selectedTab` and `tabStacks`, then reconcile using the same immutable snapshot approach.

### Optional: Navigation Compose Integration
- Remember the controller (`rememberNavController()`) and key `LaunchedEffect` on primitive route lists.
- Diff current vs. target stacks to calculate minimal pops/pushes:

```kotlin
suspend fun syncNavigation(nav: NavHostController, target: List<Route>) {
    val currentRoutes = nav.backQueue
        .mapNotNull { it.destination.route }
        .drop(1)
        .map(::stringToRoute)
    if (currentRoutes == target) return

    var keep = 0
    while (keep < currentRoutes.size && keep < target.size && currentRoutes[keep] == target[keep]) keep++
    repeat(currentRoutes.size - keep) { nav.popBackStack() }
    target.drop(keep).forEach { nav.navigate(routeToString(it)) { launchSingleTop = true } }
}
```

## Implementation Checklist
- [ ] Land `AppManager.kt` and `AuthManager.kt` with complete `FfiReconcile` coverage.
- [ ] Implement Compose `RouteView` parity (direct `when` or NavHost + diffed sync).
- [ ] Wire managers to screens: wallet list, selected wallet, send flow, TapSigner, settings, coin control, secret words.
- [ ] Build global alert/sheet plumbing (Compose analog to Swift `TaggedItem`).
- [ ] Support deep links, process death restoration, and hardware back routing.
- [ ] Add unit/integration tests for router sync, back handling, and send/wallet flows.
- [ ] Document Android-specific requirements (permissions, NFC lifecycle, UX gaps).

## Next Steps
1. Finalize Compose `AppManager`/`AuthManager` scaffolding and reset helpers.
2. Replace the temporary Android entry point with `CoveAppRoot` gated by auth.
3. Connect each Compose screen to real managers and verify Rust route parity end-to-end.
4. Add regression tests (unit or instrumentation) that cover stack sync, deep links, and process restoration.
5. Revisit NavHost vs. direct mapping once parity is stable and UX requirements (animations, deep links) are clear.

## Alternative Libraries (Future Exploration)
- **Decompose**: Kotlin Multiplatform, pure-state back stacks; clean Rust-first mental model.
- **Voyager**: Lightweight screen-based routing with minimal boilerplate.
- **Molecule + custom reducer**: Full control if Compose Navigation proves limiting.

Stick with the direct Rust-first approach as the baseline and layer in alternatives only if product requirements demand them.
