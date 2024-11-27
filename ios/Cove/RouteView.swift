import ActivityIndicatorView
import SwiftUI

struct RouteView: View {
    @Bindable var model: MainViewModel
    @State var route: Route

    init(model: MainViewModel, route: Route? = nil) {
        self.model = model
        self.route = route ?? model.router.default
    }

    var body: some View {
        ZStack {
            if model.asyncRuntimeReady {
                routeToView(model: model, route: route)
                    .id(model.routeId)
            } else {
                VStack {
                    ActivityIndicatorView(isVisible: Binding.constant(true), type: .growingArc(.orange, lineWidth: 4))
                        .frame(width: 75, height: 75)
                        .padding(.bottom, 100)
                        .foregroundColor(.orange)
                }
            }
        }
        .onChange(of: model.router.default) { _, newRoute in
            route = newRoute
        }
        .tint(.blue)
        .accentColor(.blue)
    }
}

@MainActor @ViewBuilder
func routeToView(model: MainViewModel, route: Route) -> some View {
    switch route {
    case let .loadAndReset(resetTo: route, afterMillis: time):
        LoadAndResetView(nextRoute: route.route(), loadingTimeMs: Int(time))
    case .settings:
        SettingsScreen()
    case .listWallets:
        ListWalletsScreen(model: model)
    case let .newWallet(route: route):
        NewWalletContainer(route: route)
    case let .selectedWallet(walletId):
        SelectedWalletScreen(id: walletId)
    case let .secretWords(id: walletId):
        SecretWordsScreen(id: walletId)
    case let .transactionDetails(id: id, details: details):
        TransactionsDetailScreen(id: id, transactionDetails: details)
    case let .send(sendRoute):
        SendRouteContainer(sendRoute: sendRoute)
    }
}
