import ActivityIndicatorView
import SwiftUI

struct RouteView: View {
    @Bindable var manager: AppManager
    @State var route: Route

    init(manager: AppManager, route: Route? = nil) {
        self.manager = manager
        self.route = route ?? manager.router.default
    }

    var body: some View {
        ZStack {
            if manager.asyncRuntimeReady {
                routeToView(manager: manager, route: route)
                    .id(manager.routeId)
            } else {
                VStack {
                    ActivityIndicatorView(
                        isVisible: Binding.constant(true), type: .growingArc(.orange, lineWidth: 4)
                    )
                    .frame(width: 75, height: 75)
                    .padding(.bottom, 100)
                    .foregroundColor(.orange)
                }
            }
        }
        .onChange(of: manager.router.default) { _, newRoute in
            route = newRoute
        }
        .tint(.blue)
        .accentColor(.blue)
    }
}

@MainActor @ViewBuilder
func routeToView(manager: AppManager, route: Route) -> some View {
    switch route {
    case let .loadAndReset(resetTo: routes, afterMillis: time):
        LoadAndResetView(nextRoute: routes.routes, loadingTimeMs: Int(time))
    case let .walletSettings(id):
        WalletSettingsContainer(id: id)
            .environment(manager)
    case .settings:
        SettingsScreen()
    case .listWallets:
        ListWalletsScreen(manager: manager)
    case let .newWallet(route: route):
        NewWalletContainer(route: route)
    case let .selectedWallet(walletId):
        SelectedWalletContainer(id: walletId)
    case let .secretWords(id: walletId):
        SecretWordsScreen(id: walletId)
    case let .transactionDetails(id: id, details: details):
        TransactionsDetailScreen(id: id, transactionDetails: details)
    case let .send(sendRoute):
        SendRouteContainer(sendRoute: sendRoute)
    }
}
