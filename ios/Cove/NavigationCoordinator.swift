import Observation
import SwiftUI

private let sidebarNavigationDelayMs = 250
private let navigationSettleDelayMs = 800

protocol NavigationRouteClient: AnyObject {
    func dispatch(action: AppAction) throws
    func loadAndResetDefaultRoute(route: Route)
    func resetAfterLoading(to routes: [Route])
    func resetDefaultRouteTo(route: Route)
    func resetNestedRoutesTo(defaultRoute: Route, nestedRoutes: [Route])
}

extension FfiApp: NavigationRouteClient {}

@Observable final class NavigationCoordinator {
    typealias Sleep = @MainActor (Duration) async throws -> Void
    typealias RouteOwnershipReconciler = (Router) -> Void

    @ObservationIgnored
    private let routeClient: NavigationRouteClient
    @ObservationIgnored
    private let navigationGenerations: GenerationTrackerProtocol
    @ObservationIgnored
    private let navigationSettleDelay: Duration
    @ObservationIgnored
    private let sidebarNavigationDelay: Duration
    @ObservationIgnored
    private let sleep: Sleep
    @ObservationIgnored
    private var pendingSidebarNavigationTask: Task<Void, Never>?
    @ObservationIgnored
    private var navigationSettleTask: Task<Void, Never>?

    var isNavigationSettled = true

    init(
        routeClient: NavigationRouteClient,
        navigationGenerations: GenerationTrackerProtocol = GenerationTracker(),
        navigationSettleDelay: Duration = .milliseconds(navigationSettleDelayMs),
        sidebarNavigationDelay: Duration = .milliseconds(sidebarNavigationDelayMs),
        sleep: @escaping Sleep = { try await Task.sleep(for: $0) }
    ) {
        self.routeClient = routeClient
        self.navigationGenerations = navigationGenerations
        self.navigationSettleDelay = navigationSettleDelay
        self.sidebarNavigationDelay = sidebarNavigationDelay
        self.sleep = sleep
    }

    deinit {
        pendingSidebarNavigationTask?.cancel()
        navigationSettleTask?.cancel()
    }

    func reset() {
        pendingSidebarNavigationTask?.cancel()
        pendingSidebarNavigationTask = nil
        navigationSettleTask?.cancel()
        navigationSettleTask = nil
        advanceNavigationGeneration()
    }

    func selectWallet(
        _ id: WalletId,
        router _: Router,
        isSidebarVisible: inout Bool,
        advancesGeneration: Bool = true
    ) throws {
        if advancesGeneration {
            advanceNavigationGeneration()
        }

        try routeClient.dispatch(action: .selectWallet(id: id))
        isSidebarVisible = false
    }

    func selectLatestOrNewWallet(isSidebarVisible: inout Bool) throws {
        advanceNavigationGeneration()
        try routeClient.dispatch(action: .selectLatestOrNewWallet)
        isSidebarVisible = false
    }

    func pushRoute(
        _ route: Route,
        router: inout Router,
        isSidebarVisible: inout Bool,
        advancesGeneration: Bool = true,
        reconcileRouteOwnership: RouteOwnershipReconciler
    ) {
        if advancesGeneration, isDuplicateTopRoute(route, router: router) {
            isSidebarVisible = false
            return
        }

        if advancesGeneration {
            advanceNavigationGeneration()
        }

        isSidebarVisible = false
        guard !isDuplicateTopRoute(route, router: router) else { return }

        router.routes.append(route)
        reconcileRouteOwnership(router)
    }

    func pushRoutes(
        _ routes: [Route],
        router: inout Router,
        isSidebarVisible: inout Bool,
        advancesGeneration: Bool = true,
        reconcileRouteOwnership: RouteOwnershipReconciler
    ) {
        if advancesGeneration {
            advanceNavigationGeneration()
        }

        isSidebarVisible = false
        router.routes.append(contentsOf: routes)
        reconcileRouteOwnership(router)
    }

    func popRoute(
        router: inout Router,
        reconcileRouteOwnership: RouteOwnershipReconciler
    ) {
        advanceNavigationGeneration()

        if !router.routes.isEmpty {
            router.routes.removeLast()
            reconcileRouteOwnership(router)
        }
    }

    func setRoute(
        _ routes: [Route],
        router: inout Router,
        reconcileRouteOwnership: RouteOwnershipReconciler
    ) {
        advanceNavigationGeneration()
        router.routes = routes
        reconcileRouteOwnership(router)
    }

    func scanNfc(
        advancesGeneration: Bool = true,
        scan: () -> Void
    ) {
        if advancesGeneration {
            advanceNavigationGeneration()
        }

        scan()
    }

    @MainActor
    func resetRoute(to routes: [Route], advancesGeneration: Bool = true) {
        if advancesGeneration {
            advanceNavigationGeneration()
        }

        if routes.count > 1 {
            routeClient.resetNestedRoutesTo(defaultRoute: routes[0], nestedRoutes: Array(routes[1...]))
        } else if let route = routes.first {
            routeClient.resetDefaultRouteTo(route: route)
        }
    }

    func resetRoute(to route: Route, advancesGeneration: Bool = true) {
        if advancesGeneration {
            advanceNavigationGeneration()
        }

        routeClient.resetDefaultRouteTo(route: route)
    }

    @MainActor
    func loadAndReset(to route: Route) {
        advanceNavigationGeneration()
        routeClient.loadAndResetDefaultRoute(route: route)
    }

    func closeSidebarThenNavigate(
        isSidebarVisible: inout Bool,
        action: @escaping @MainActor () -> Void
    ) {
        pendingSidebarNavigationTask?.cancel()
        let generation = advanceNavigationGeneration()

        withAnimation(.spring(response: 0.3, dampingFraction: 0.8)) {
            isSidebarVisible = false
        }

        pendingSidebarNavigationTask = Task { @MainActor [weak self] in
            guard let self else { return }

            do {
                try await sleep(sidebarNavigationDelay)
            } catch {
                return
            }

            guard isNavigationGenerationCurrent(generation) else { return }
            action()
        }
    }

    @MainActor
    func captureLoadAndResetGeneration() -> GenerationToken {
        navigationGenerations.capture()
    }

    @MainActor
    func startLoadAndResetTargetPrewarm(
        generation: GenerationToken,
        routes: [Route],
        prewarmSelectedWallet: @escaping @MainActor (WalletId) async -> Void
    ) {
        Task { [weak self] in
            await self?.prewarmLoadAndResetTargetIfCurrent(
                generation: generation,
                routes: routes,
                prewarmSelectedWallet: prewarmSelectedWallet
            )
        }
    }

    @MainActor
    func resetAfterLoadingIfCurrent(
        generation: GenerationToken,
        route: Route,
        nextRoute: [Route],
        router: Router
    ) {
        guard isNavigationGenerationCurrent(generation) else { return }
        guard router.default == route else { return }

        routeClient.resetAfterLoading(to: nextRoute)
    }

    func applyRouteUpdated(
        routes: [Route],
        router: inout Router,
        reconcileRouteOwnership: RouteOwnershipReconciler
    ) {
        let didChangeRoute = router.routes != routes
        router.routes = routes
        reconcileRouteOwnership(router)

        if didChangeRoute {
            scheduleNavigationSettledForCurrentGeneration()
        }
    }

    func applyPushedRoute(
        _ route: Route,
        router: inout Router,
        isSidebarVisible: inout Bool,
        reconcileRouteOwnership: RouteOwnershipReconciler
    ) {
        guard !isDuplicateTopRoute(route, router: router) else {
            isSidebarVisible = false
            return
        }

        router.routes.append(route)
        reconcileRouteOwnership(router)
        scheduleNavigationSettledForCurrentGeneration()
    }

    func applyDefaultRouteChanged(
        route: Route,
        nestedRoutes: [Route],
        router: inout Router,
        routeId: inout UUID,
        reconcileRouteOwnership: RouteOwnershipReconciler
    ) {
        router.routes = nestedRoutes
        router.default = route
        routeId = UUID()
        reconcileRouteOwnership(router)
        scheduleNavigationSettledForCurrentGeneration()
    }

    @discardableResult
    func advanceNavigationGeneration() -> GenerationToken {
        let generation = navigationGenerations.advance()
        scheduleNavigationSettled(for: generation)
        return generation
    }

    func scheduleNavigationSettledForCurrentGeneration() {
        scheduleNavigationSettled(for: navigationGenerations.capture())
    }

    func isNavigationGenerationCurrent(_ generation: GenerationToken) -> Bool {
        navigationGenerations.isCurrent(capturedToken: generation)
    }

    @MainActor
    private func prewarmLoadAndResetTargetIfCurrent(
        generation: GenerationToken,
        routes: [Route],
        prewarmSelectedWallet: @escaping @MainActor (WalletId) async -> Void
    ) async {
        guard isNavigationGenerationCurrent(generation) else { return }
        guard case let .selectedWallet(id) = routes.first else { return }

        await prewarmSelectedWallet(id)
    }

    private func scheduleNavigationSettled(for generation: GenerationToken) {
        navigationSettleTask?.cancel()
        isNavigationSettled = false

        navigationSettleTask = Task { @MainActor [weak self] in
            guard let self else { return }

            do {
                try await sleep(navigationSettleDelay)
            } catch {
                return
            }

            guard isNavigationGenerationCurrent(generation) else { return }

            isNavigationSettled = true
            navigationSettleTask = nil
        }
    }

    private func currentRoute(router: Router) -> Route {
        router.routes.last ?? router.default
    }

    private func isDuplicateTopRoute(_ route: Route, router: Router) -> Bool {
        currentRoute(router: router).isSameNavigationDestination(routeToCheck: route)
    }
}
