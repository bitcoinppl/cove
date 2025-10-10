# iOS Baseline

- `CoveApp` binds the SwiftUI `NavigationStack` to the Rust-backed router, so route pushes/pops always go through `app.rust` (`ios/Cove/CoveApp.swift:476`–`ios/Cove/CoveApp.swift:512`).
- `RouteView` switches on the shared Rust `Route` enum to render screens (`ios/Cove/RouteView.swift:35`–`ios/Cove/RouteView.swift:58`).
- `AppManager` owns the singleton `FfiApp`, caches the current `Router`, and implements `FfiReconcile` so Rust can broadcast router/auth updates (`ios/Cove/AppManager.swift:4`–`ios/Cove/AppManager.swift:199`).
- `AuthManager` mirrors Rust auth state and wraps the navigation UI in lock/cover flows (`ios/Cove/AuthManager.swift:7`–`ios/Cove/AuthManager.swift:116`, `ios/Cove/CoveApp.swift:476`–`ios/Cove/CoveApp.swift:519`).

# Rust Core

- All navigation types (`Route`, `Router`, helpers) live in Rust and are exported to every platform via uniffi (`rust/src/router.rs:17`–`rust/src/router.rs:195`).
- `App` and `AppAction` mutate the router and emit `AppStateReconcileMessage` callbacks (`rust/src/app.rs:32`–`rust/src/app.rs:223`).
- `FfiApp` exposes `reset_default_route_to`, `load_and_reset_default_route`, `listen_for_updates`, etc., bridging routing/auth decisions to Swift/Kotlin (`rust/src/app.rs:241`–`rust/src/app.rs:399`).
- Callback plumbing (`FfiReconcile`) allows Rust threads to notify the UI layer safely (`rust/src/app/reconcile.rs:9`–`rust/src/app/reconcile.rs:60`).

# Jetpack Compose Equivalent

- Kotlin already has the generated bindings (`FfiApp`, `Route`, `FfiReconcile`) under `android/app/src/main/java/org/bitcoinppl/cove/cove.kt:9433`, `android/app/src/main/java/org/bitcoinppl/cove/cove.kt:24431`, `android/app/src/main/java/org/bitcoinppl/cove/cove.kt:31758`, `android/app/src/main/java/org/bitcoinppl/cove/cove.kt:39483`.
- Mirror `AppManager` with an `AppCoordinator` that owns `FfiApp`, snapshots the initial router, listens for `AppStateReconcileMessage`, and exposes a `StateFlow`:

```kotlin
// android/app/src/main/java/org/bitcoinppl/cove/app/AppCoordinator.kt
class AppCoordinator(
    private val scope: CoroutineScope,
    private val ffiApp: FfiApp = FfiApp(),
) : FfiReconcile {

    data class RouterState(val defaultRoute: Route, val stack: List<Route>)

    private val _router = MutableStateFlow(
        ffiApp.state().router.let { RouterState(it.`default`, it.routes) }
    )
    val router: StateFlow<RouterState> = _router.asStateFlow()

    init {
        ffiApp.listenForUpdates(this)
    }

    override fun reconcile(message: AppStateReconcileMessage) {
        scope.launch(Dispatchers.Main) {
            when (message) {
                is AppStateReconcileMessage.DefaultRouteChanged ->
                    _router.value = RouterState(message.v0, message.v1)
                is AppStateReconcileMessage.RouteUpdated ->
                    _router.update { it.copy(stack = message.v0) }
                is AppStateReconcileMessage.PushedRoute ->
                    _router.update { it.copy(stack = it.stack + message.v0) }
                AppStateReconcileMessage.AcceptedTerms -> { /* propagate */ }
                else -> { /* handle other signals as needed */ }
            }
        }
    }

    fun push(route: Route) = ffiApp.dispatch(AppAction.PushRoute(route))
    fun setRoutes(routes: List<Route>) =
        ffiApp.dispatch(AppAction.UpdateRoute(routes))
}
```

- Compose can mirror `RouteView` with a `when` expression on the top route:

```kotlin
@Composable
fun CoveAppRoot(coordinator: AppCoordinator = rememberCoordinator()) {
    val router by coordinator.router.collectAsState()
    val current = router.stack.lastOrNull() ?: router.defaultRoute

    when (current) {
        Route.ListWallets -> ListWalletsScreen(
            onSelect = { coordinator.push(Route.SelectedWallet(it)) }
        )
        is Route.SelectedWallet -> SelectedWalletScreen(current.v1)
        is Route.NewWallet -> NewWalletFlow(current.v1, coordinator::push)
        is Route.Settings -> SettingsContainer(current.v1, coordinator)
        // Handle the remaining Route variants (SecretWords, Send, CoinControl, etc.)
    }
}
```

- Alternatively, map each `Route` to a `NavHost` destination and sync the back stack from `router.stack`.
- Auth flow: wrap `CoveAppRoot` with a Compose lock surface backed by `RustAuthManager`, mirroring the Swift `LockView` behavior.
- Suggested next steps:
  1. Add `AppCoordinator` (and a matching `AuthCoordinator`) under `android/app/src/main/java/org/bitcoinppl/cove/app/`.
  2. Update `MainActivity` to host the new Compose root rather than a single screen.
  3. Flesh out each screen branch so routing parity with iOS is maintained (send flow, TapSigner, settings, etc.).

## EXTRA

Short answer: your architecture is solid and maps well from SwiftUI → Compose. A few fixes and upgrades will make it robust and “best-practice” for 2025-era Compose.

Corrections and improvements (most important first): 1. Prevent sync loops and do minimal diffs
Your LaunchedEffect(nav) blocks as written will easily loop (Rust → Compose → Rust). Diff the stacks and apply the smallest change.

@Immutable
sealed interface Route {
data object ListWallets : Route
data class SelectedWallet(val id: String) : Route
// ...
}

fun routesEqual(a: List<Route>, b: List<Route>) = a == b

suspend fun syncNavigation(nav: NavHostController, target: List<Route>) {
val current = nav.backQueue
.mapNotNull { it.destination.route }
.drop(1) // drop graph root
val currentRoutes = current.map(::stringToRoute) // your reversible mapping

    if (routesEqual(currentRoutes, target)) return

    // Find LCP (longest common prefix) to avoid full reset
    var keep = 0
    while (keep < currentRoutes.size && keep < target.size && currentRoutes[keep] == target[keep]) keep++

    // Pop back to the kept prefix
    repeat(currentRoutes.size - keep) { nav.popBackStack() }

    // Push the remainder with singleTop
    target.drop(keep).forEach { r ->
        nav.navigate(routeToString(r)) {
            launchSingleTop = true
        }
    }

}

    2.	Collect nav changes correctly

Use currentBackStackEntryFlow directly; don’t wrap it inside snapshotFlow. Also, don’t key the LaunchedEffect to currentBackStackEntry (that object changes identity unpredictably). Key to navController.

@Composable
fun CoveApp(app: AppManager = remember { AppManager.shared }) {
val navController = rememberNavController()

    // Rust -> Compose
    LaunchedEffect(app.router.routes) {
        // Ensure routes list is immutable/structurally compared each update
        syncNavigation(navController, app.router.routes)
    }

    // Compose -> Rust
    LaunchedEffect(navController) {
        navController.currentBackStackEntryFlow.collect { entry ->
            val stack = navController.backQueue
                .mapNotNull { it.destination.route }
                .drop(1)
                .map(::stringToRoute)
            // Only dispatch if it differs from Rust's view
            if (!routesEqual(stack, app.router.routes)) {
                app.dispatch(AppAction.UpdateRoute(routes = stack))
            }
        }
    }

    NavHost(
        navController = navController,
        startDestination = routeToString(app.router.default)
    ) {
        composable("listWallets") { ListWalletsScreen() }
        composable("selectedWallet/{id}") { backStack ->
            SelectedWalletScreen(backStack.arguments?.getString("id")!!)
        }
        // ...
    }

    // Android system back -> Rust (optional if you want Rust to own it)
    BackHandler {
        val newStack = app.router.routes.dropLast(1)
        app.dispatch(AppAction.UpdateRoute(routes = newStack))
    }

}

    3.	Make the router state truly state-driven

Use an immutable list for router.routes so Compose can diff it reliably (don’t mutate in place). Favor val router by mutableStateOf(initial, policy = structuralEqualityPolicy()).

@Stable
class AppManager : FfiReconcile {
val rust = FfiApp()
var router by mutableStateOf(rust.state().router, structuralEqualityPolicy())
private set
// ...
override fun reconcile(msg: AppStateReconcileMessage) {
when (msg) {
is AppStateReconcileMessage.RouteUpdated ->
router = router.copy(routes = msg.v1.toList()) // new instance
is AppStateReconcileMessage.DefaultRouteChanged ->
router = Router(rust, msg.v1, msg.v2.toList())
// ...
}
}
}

    4.	Argument typing and route mapping

Keep a single mapping pair to avoid drift:

fun routeToString(r: Route): String = when (r) {
Route.ListWallets -> "listWallets"
is Route.SelectedWallet -> "selectedWallet/${Uri.encode(r.id)}"
}
fun stringToRoute(s: String): Route = when {
s == "listWallets" -> Route.ListWallets
s.startsWith("selectedWallet/") -> Route.SelectedWallet(Uri.decode(s.substringAfter('/')))
else -> Route.ListWallets
}

    5.	Back stack ownership and system back

If Rust is the source of truth, always reflect system back into Rust (as above). If you prefer Android to own physical back presses, allow navController.popBackStack() and have a listener push the updated stack back to Rust (your current flow already does this). 6. Process death and state restoration
Compose Navigation restores destinations automatically; your Rust router must be rehydrated to match after app restart. On first render, do a one-time “authoritative” sync from Rust → Compose (or vice-versa) based on your product decision. Persist either a) Rust router snapshot, or b) the Compose back stack routes in SavedStateHandle and re-emit to Rust during init. 7. Multiple back stacks / bottom navigation (if relevant)
Navigation Compose supports multiple back stacks. If you later add tabs, maintain one Rust route stack per tab and a Rust “selected tab”. Map them to NavHosts with navigation() graphs or multiple NavHostControllers. Keep the same diff/sync idea per stack. 8. Deep links and external intents
When Android launches you into a deep destination, you’ll get an initial back stack from Navigation. Normalize it into your Route list and dispatch an initial UpdateRoute so Rust matches what Android displayed. 9. Performance and recomposition hygiene
Key your effects narrowly, avoid rebuilding NavHostController, and ensure route lists are small immutable values. Use long descriptive names as you prefer for clarity in reconciliation code.

Inline comparison update (yours is mostly right):

Swift/SwiftUI Kotlin/Jetpack Compose
NavigationStack(path: $routes) direct binding NavHostController + state diff (Rust routes → minimal nav ops)
@Bindable two-way binding mutableStateOf + Flow listener (currentBackStackEntryFlow)
.navigationDestination(for:) NavHost { composable(…) }
Automatic back stack sync Manual diff + popBackStack/navigate, plus BackHandler if Rust owns back
onChange(of: routes) LaunchedEffect(routes) + structural equality; ensure immutable lists

Alternatives you may consider (you might like these):
• Square’s Molecule + a lightweight back-stack model (your own stack reducer) if you want 100% control and no Nav library.
• Ark Ivanov’s Decompose for a Kotlin-multiplatform, state-first router with back stacks as pure models (clean Rust-first mental model).
• Voyager for simpler, model-driven screens if you want less boilerplate than Navigation Compose.

Verdict: your proposed “Rust-first router + Compose Navigation with state-driven sync” is still a top-tier approach. The key is the minimal-diff sync, correct currentBackStackEntryFlow collection, immutable route lists, and a clear rule for who is authoritative on startup and back. Implement those tweaks and you’re in great shape.
