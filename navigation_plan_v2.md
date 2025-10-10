# Navigation Plan V2: Rust-First Routing for Android/Compose

This document combines the iOS baseline analysis with best practices for implementing equivalent navigation in Jetpack Compose.

## Architecture Overview

### Core Principles

1. **Rust Owns the Router**: All routing state lives in Rust (`Router` with `default` route and `routes` stack)
2. **Reconciler Pattern**: Rust broadcasts state changes to UI via `FfiReconcile` callbacks
3. **Two-way Sync**: UI can dispatch actions to Rust, Rust reconciles state back to UI
4. **Single Source of Truth**: Rust router state is authoritative; UI reflects it

### Data Flow

```
User Action → UI dispatch() → Rust mutation → reconcile() callback → UI state update → Recomposition
     ↑                                                                                    ↓
     └──────────────────────── System Back / Deep Link ────────────────────────────────┘
```

## iOS Baseline (Reference Implementation)

### SwiftUI Architecture

**Key Files:**
- `ios/Cove/CoveApp.swift:488-510` - NavigationStack binding
- `ios/Cove/RouteView.swift:36-61` - Route to View mapping
- `ios/Cove/AppManager.swift:4-279` - Singleton app coordinator with FfiReconcile
- `ios/Cove/AuthManager.swift:7-161` - Auth state wrapper

**How it Works:**
1. **NavigationStack binding**: `NavigationStack(path: $app.router.routes)` directly binds to Rust state
2. **Destination mapping**: `.navigationDestination(for: Route.self)` maps each route variant to a view
3. **Reconcile updates**: AppManager receives messages from Rust:
   - `.routeUpdated(routes)` → Replace entire stack
   - `.pushedRoute(route)` → Append single route
   - `.defaultRouteChanged(route, nestedRoutes)` → Reset base + stack
4. **User navigation**: SwiftUI auto-syncs via `onChange(of: router.routes)` → `dispatch(.updateRoute())`

### Key Rust Exports

**Types** (from `rust/src/router.rs`):
- `Route` - Sealed class with variants (ListWallets, SelectedWallet, NewWallet, etc.)
- `Router` - Data class with `app: FfiApp`, `default: Route`, `routes: List<Route>`
- `AppStateReconcileMessage` - Sealed class for reconcile callbacks

**Methods** (from `rust/src/app.rs`):
- `FfiApp.dispatch(action: AppAction)` - Send routing actions to Rust
- `FfiApp.listenForUpdates(reconciler: FfiReconcile)` - Register for state callbacks
- `FfiApp.resetDefaultRouteTo(route: Route)` - Change root route
- `FfiApp.resetNestedRoutesTo(defaultRoute, nestedRoutes)` - Reset with stack

## Android/Compose Implementation

### Option 1: AppManager with State-Driven Navigation (Recommended)

This approach mirrors iOS most closely and maintains Rust-first philosophy.

#### 1. AppManager (Coordinator)

```kotlin
// android/app/src/main/java/org/bitcoinppl/cove/AppManager.kt
@Stable
class AppManager private constructor() : FfiReconcile {
    companion object {
        val shared by lazy { AppManager() }
    }

    private val tag = "AppManager"
    val rust: FfiApp = FfiApp()

    // router state with structural equality for proper diffing
    var router by mutableStateOf(
        rust.state().router,
        policy = structuralEqualityPolicy()
    )
        private set

    var database by mutableStateOf(Database())
        private set

    // other state...
    var isTermsAccepted by mutableStateOf(Database().globalFlag().isTermsAccepted())
    var selectedNetwork by mutableStateOf(Database().globalConfig().selectedNetwork())
    var colorSchemeSelection by mutableStateOf(Database().globalConfig().colorScheme())

    init {
        Log.d(tag, "Initializing AppManager")
        rust.listenForUpdates(this)
    }

    override fun reconcile(message: AppStateReconcileMessage) {
        Log.d(tag, "Reconcile: $message")

        when (message) {
            is AppStateReconcileMessage.RouteUpdated -> {
                // create new router instance for proper state diffing
                router = Router(rust, router.`default`, message.v1.toList())
            }

            is AppStateReconcileMessage.PushedRoute -> {
                router = Router(rust, router.`default`, router.routes + message.v1)
            }

            is AppStateReconcileMessage.DefaultRouteChanged -> {
                router = Router(rust, message.v1, message.v2.toList())
            }

            is AppStateReconcileMessage.DatabaseUpdated -> {
                database = Database()
            }

            is AppStateReconcileMessage.ColorSchemeChanged -> {
                colorSchemeSelection = message.v1
            }

            is AppStateReconcileMessage.SelectedNetworkChanged -> {
                selectedNetwork = message.v1
            }

            is AppStateReconcileMessage.AcceptedTerms -> {
                isTermsAccepted = true
            }

            // ... other cases
        }
    }

    fun dispatch(action: AppAction) {
        Log.d(tag, "Dispatch: $action")
        rust.dispatch(action)
    }

    // convenience methods matching iOS
    fun pushRoute(route: Route) {
        dispatch(AppAction.PushRoute(route))
    }

    fun popRoute() {
        val newRoutes = router.routes.dropLast(1)
        dispatch(AppAction.UpdateRoute(newRoutes))
    }

    fun setRoutes(routes: List<Route>) {
        dispatch(AppAction.UpdateRoute(routes))
    }

    fun resetRoute(route: Route) {
        rust.resetDefaultRouteTo(route)
    }

    fun selectWallet(id: WalletId) {
        try {
            rust.selectWallet(id)
        } catch (e: Exception) {
            Log.e(tag, "Unable to select wallet $id", e)
        }
    }
}
```

#### 2. Route Mapping Functions

```kotlin
// android/app/src/main/java/org/bitcoinppl/cove/navigation/RouteMapping.kt
package org.bitcoinppl.cove.navigation

import android.net.Uri

/**
 * Convert Route to navigation string
 * Keep this in sync with stringToRoute()
 */
fun routeToString(route: Route): String = when (route) {
    is Route.ListWallets -> "listWallets"
    is Route.SelectedWallet -> "selectedWallet/${Uri.encode(route.v1.id)}"
    is Route.NewWallet -> "newWallet/${encodeNewWalletRoute(route.v1)}"
    is Route.Settings -> "settings/${encodeSettingsRoute(route.v1)}"
    is Route.SecretWords -> "secretWords/${Uri.encode(route.v1.id)}"
    is Route.TransactionDetails -> "txDetails/${route.v1.id}/${encodeTxDetails(route.v2)}"
    is Route.Send -> "send/${encodeSendRoute(route.v1)}"
    is Route.CoinControl -> "coinControl/${encodeCoinControlRoute(route.v1)}"
    is Route.LoadAndReset -> "loading" // special case, handled separately
}

/**
 * Convert navigation string to Route
 * Inverse of routeToString()
 */
fun stringToRoute(str: String): Route? = when {
    str == "listWallets" -> Route.ListWallets
    str.startsWith("selectedWallet/") -> {
        val id = Uri.decode(str.substringAfter('/'))
        Route.SelectedWallet(WalletId(id))
    }
    str.startsWith("newWallet/") -> {
        val encoded = str.substringAfter('/')
        Route.NewWallet(decodeNewWalletRoute(encoded))
    }
    // ... other routes
    else -> null
}

/**
 * Check if two route lists are equal
 */
fun routesEqual(a: List<Route>, b: List<Route>): Boolean {
    return a.size == b.size && a.zip(b).all { (r1, r2) ->
        routeToString(r1) == routeToString(r2)
    }
}
```

#### 3. Navigation Sync Logic (Critical!)

```kotlin
// android/app/src/main/java/org/bitcoinppl/cove/navigation/NavigationSync.kt
package org.bitcoinppl.cove.navigation

import androidx.navigation.NavHostController

/**
 * Sync NavController state to match target routes from Rust
 * Uses minimal-diff algorithm to avoid unnecessary navigation ops
 *
 * This prevents sync loops and improves performance
 */
suspend fun syncNavigation(
    nav: NavHostController,
    targetRoutes: List<Route>
) {
    // get current back stack as Route list
    val current = nav.backQueue
        .mapNotNull { it.destination.route }
        .drop(1) // drop the graph root/start destination
        .mapNotNull(::stringToRoute)

    // already in sync, nothing to do
    if (routesEqual(current, targetRoutes)) return

    // find longest common prefix (LCP) to minimize ops
    var keep = 0
    while (keep < current.size &&
           keep < targetRoutes.size &&
           routeToString(current[keep]) == routeToString(targetRoutes[keep])) {
        keep++
    }

    // pop back to the common prefix
    repeat(current.size - keep) {
        nav.popBackStack()
    }

    // push the new routes after the prefix
    targetRoutes.drop(keep).forEach { route ->
        nav.navigate(routeToString(route)) {
            launchSingleTop = true
            restoreState = true
        }
    }
}
```

#### 4. Main App Composable

```kotlin
// android/app/src/main/java/org/bitcoinppl/cove/CoveApp.kt
package org.bitcoinppl.cove

import androidx.compose.runtime.*
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import org.bitcoinppl.cove.navigation.*

@Composable
fun CoveApp(
    app: AppManager = remember { AppManager.shared }
) {
    val navController = rememberNavController()

    // Rust → Compose: sync navigation when router state changes
    LaunchedEffect(app.router.routes) {
        syncNavigation(navController, app.router.routes)
    }

    // Compose → Rust: sync back stack changes to Rust
    LaunchedEffect(navController) {
        navController.currentBackStackEntryFlow.collect { _ ->
            val currentStack = navController.backQueue
                .mapNotNull { it.destination.route }
                .drop(1)
                .mapNotNull(::stringToRoute)

            // only dispatch if different to avoid loops
            if (!routesEqual(currentStack, app.router.routes)) {
                app.setRoutes(currentStack)
            }
        }
    }

    // handle Android system back button
    BackHandler(enabled = app.router.routes.isNotEmpty()) {
        app.popRoute()
    }

    NavHost(
        navController = navController,
        startDestination = routeToString(app.router.`default`)
    ) {
        composable("listWallets") {
            ListWalletsScreen(
                onSelectWallet = { walletId ->
                    app.pushRoute(Route.SelectedWallet(walletId))
                }
            )
        }

        composable("selectedWallet/{id}") { backStack ->
            val walletId = backStack.arguments?.getString("id")?.let(::WalletId)
            if (walletId != null) {
                SelectedWalletScreen(
                    walletId = walletId,
                    onNavigateToSend = { route ->
                        app.pushRoute(Route.Send(route))
                    }
                )
            }
        }

        composable("newWallet/{route}") { backStack ->
            // decode and show new wallet flow
            NewWalletContainer(
                onComplete = { walletId ->
                    app.selectWallet(walletId)
                }
            )
        }

        composable("settings/{route}") {
            SettingsContainer(
                onNavigateBack = { app.popRoute() }
            )
        }

        // ... other routes
    }
}
```

#### 5. Route to View Mapping (Alternative: No NavHost)

If you prefer to match iOS exactly with a simple `when` expression:

```kotlin
@Composable
fun CoveApp(app: AppManager = remember { AppManager.shared }) {
    val currentRoute = app.router.routes.lastOrNull() ?: app.router.`default`

    // direct route → view mapping like iOS RouteView
    when (currentRoute) {
        is Route.ListWallets -> {
            ListWalletsScreen(
                onSelectWallet = { app.pushRoute(Route.SelectedWallet(it)) }
            )
        }

        is Route.SelectedWallet -> {
            SelectedWalletScreen(
                walletId = currentRoute.v1,
                onNavigateToSend = { app.pushRoute(Route.Send(it)) }
            )
        }

        is Route.NewWallet -> {
            NewWalletContainer(
                route = currentRoute.v1,
                onComplete = { app.selectWallet(it) }
            )
        }

        is Route.Settings -> {
            SettingsContainer(
                route = currentRoute.v1,
                onBack = { app.popRoute() }
            )
        }

        is Route.SecretWords -> {
            SecretWordsScreen(
                walletId = currentRoute.v1,
                onBack = { app.popRoute() }
            )
        }

        is Route.TransactionDetails -> {
            TransactionDetailsScreen(
                id = currentRoute.v1,
                details = currentRoute.v2,
                onBack = { app.popRoute() }
            )
        }

        is Route.Send -> {
            SendFlowContainer(
                route = currentRoute.v1,
                onComplete = { app.popRoute() }
            )
        }

        is Route.CoinControl -> {
            CoinControlContainer(
                route = currentRoute.v1,
                onBack = { app.popRoute() }
            )
        }

        is Route.LoadAndReset -> {
            LoadingScreen(
                afterMillis = currentRoute.afterMillis.toInt()
            )
        }
    }

    // handle system back
    BackHandler(enabled = app.router.routes.isNotEmpty()) {
        app.popRoute()
    }
}
```

**Pros of this approach:**
- Exact 1:1 match with iOS implementation
- No navigation library dependency
- Simpler mental model
- No sync loops possible

**Cons:**
- No back stack animations (can add custom transitions)
- Manual state management for nested flows
- No deep link support out of the box

### Auth Integration

Wrap the app root with auth/lock screen, mirroring iOS `LockView`:

```kotlin
@Composable
fun CoveAppRoot(
    app: AppManager = remember { AppManager.shared },
    auth: AuthManager = remember { AuthManager.shared }
) {
    if (!app.isTermsAccepted) {
        TermsAndConditionsScreen(
            onAccept = { app.dispatch(AppAction.AcceptTerms) }
        )
    } else {
        LockScreen(
            lockType = auth.type,
            lockState = auth.lockState,
            onUnlock = { pin ->
                auth.handleAndReturnUnlockMode(pin) != UnlockMode.LOCKED
            }
        ) {
            // main app content
            CoveApp(app)
        }
    }
}
```

## Best Practices & Considerations

### 1. Prevent Sync Loops

**Problem:** Rust → Compose → Rust → Compose... infinite loop

**Solution:**
- Always diff before syncing: `if (!routesEqual(current, target)) return`
- Use `structuralEqualityPolicy()` for router state
- Make routes truly immutable (don't mutate in place)

### 2. State Restoration (Process Death)

When Android kills the app:

**Option A - Rust is authoritative:**
```kotlin
// on startup, always use Rust state
LaunchedEffect(Unit) {
    syncNavigation(navController, app.router.routes)
}
```

**Option B - Persist Rust router:**
```kotlin
// save router to DataStore on each change
// restore on app restart and sync to Rust
```

### 3. Deep Links

```kotlin
NavHost(
    navController = navController,
    startDestination = routeToString(app.router.`default`)
) {
    composable(
        route = "selectedWallet/{id}",
        deepLinks = listOf(navDeepLink {
            uriPattern = "cove://wallet/{id}"
        })
    ) { ... }
}

// on deep link navigation, sync to Rust
LaunchedEffect(navController) {
    navController.currentBackStackEntryFlow
        .filter { it.arguments?.containsKey("deepLink") == true }
        .collect { entry ->
            // extract route and dispatch to Rust
            val route = entryToRoute(entry)
            app.pushRoute(route)
        }
}
```

### 4. Multiple Back Stacks (Bottom Nav)

If you add tabs later:

```kotlin
data class Router(
    val app: FfiApp,
    val default: Route,
    val routes: List<Route>,
    val selectedTab: Tab,
    val tabStacks: Map<Tab, List<Route>> // one stack per tab
)

// in reconcile
is AppStateReconcileMessage.TabChanged -> {
    router = router.copy(selectedTab = message.tab)
}
```

### 5. Performance

- **Key LaunchedEffects correctly**: Use stable keys, not objects that change identity
- **Avoid rebuilding NavController**: Use `rememberNavController()`
- **Keep route lists small**: Don't accumulate hundreds of routes
- **Use immutable collections**: `toList()` creates new instances for proper diffing

### 6. Testing

```kotlin
@Test
fun `routing sync works correctly`() = runTest {
    val app = AppManager()
    val routes = listOf(
        Route.ListWallets,
        Route.SelectedWallet(WalletId("test"))
    )

    app.setRoutes(routes)

    // wait for reconcile
    advanceUntilIdle()

    assertEquals(routes, app.router.routes)
}
```

## Alternative Navigation Libraries

If you want alternatives to Compose Navigation:

### Decompose (Recommended for Complex Apps)
- Kotlin Multiplatform
- Pure state-driven routing
- Back stack as pure model
- Perfect Rust-first mental model
- https://github.com/arkivanov/Decompose

### Voyager
- Simpler than Navigation Compose
- Screen-based model
- Less boilerplate
- https://github.com/adrielcafe/voyager

### Molecule + Custom Stack
- 100% control over routing logic
- Define your own back stack reducer
- No library abstraction leakage
- More work to build

## Comparison Table

| Aspect | Swift/SwiftUI | Kotlin/Compose (NavHost) | Kotlin/Compose (Direct) |
|--------|---------------|--------------------------|------------------------|
| **Binding** | `NavigationStack(path: $routes)` | NavController + LaunchedEffect | Direct `when` expression |
| **State** | `@Bindable` two-way | mutableStateOf + Flow | mutableStateOf only |
| **Mapping** | `.navigationDestination(for:)` | `composable()` routes | `when (route) { ... }` |
| **Back Sync** | Automatic | Manual via currentBackStackEntryFlow | Manual BackHandler |
| **Updates** | `onChange(of:)` | `LaunchedEffect(routes)` | `LaunchedEffect(routes)` |
| **Loops** | None (SwiftUI prevents) | Manual diff required | No loops possible |
| **Animations** | Built-in | Built-in | Manual/Custom |
| **Deep Links** | Built-in | Built-in | Manual |

## Implementation Checklist

- [ ] Create `AppManager.kt` with FfiReconcile implementation
- [ ] Add `AuthManager.kt` for auth state
- [ ] Implement route mapping functions (`routeToString`, `stringToRoute`)
- [ ] Add `syncNavigation()` with minimal-diff algorithm
- [ ] Update `MainActivity` to use `CoveAppRoot`
- [ ] Implement all route → screen mappings
- [ ] Add BackHandler for system back
- [ ] Handle deep links (if needed)
- [ ] Add process death restoration
- [ ] Write tests for routing logic

## Next Steps

1. **Choose approach**: NavHost vs. Direct mapping (recommend Direct for iOS parity)
2. **Build AppManager**: Core coordinator with reconcile implementation
3. **Add auth layer**: LockScreen wrapper matching iOS
4. **Implement screens**: One-by-one route → composable mapping
5. **Test thoroughly**: Route transitions, back button, process death
6. **Add deep links**: If product requires them

---

**Recommendation**: Start with the **Direct mapping approach** (no NavHost) to exactly match iOS behavior. You can always add NavHost later if you need built-in animations or deep linking. The architecture is solid and production-ready.
