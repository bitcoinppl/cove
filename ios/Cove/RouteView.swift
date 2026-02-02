import SwiftUI

struct RouteView: View {
    @Bindable var app: AppManager
    @State var route: Route

    init(app: AppManager, route: Route? = nil) {
        self.app = app
        self.route = route ?? app.router.default
    }

    var body: some View {
        ZStack {
            if app.asyncRuntimeReady {
                routeToView(app: app, route: route)
                    .id(app.routeId)
            } else {
                VStack {
                    ProgressView()
                        .progressViewStyle(CircularProgressViewStyle())
                        .scaleEffect(2)
                        .frame(width: 100, height: 100)
                        .tint(.primary)
                }
            }
        }
        .onChange(of: app.router.default) { _, newRoute in
            route = newRoute
        }
        .modifier(RouteViewTintModifier())
    }
}

@MainActor func routeToView(app: AppManager, route: Route) -> some View {
    Group {
        switch route {
        case let .loadAndReset(resetTo: routes, afterMillis: time):
            LoadAndResetContainer(nextRoute: routes.routes, loadingTimeMs: Int(time))
        case let .settings(route):
            SettingsContainer(route: route)
        case let .newWallet(route: route):
            NewWalletContainer(route: route)
        case let .selectedWallet(walletId):
            SelectedWalletContainer(id: walletId)
        case let .secretWords(id: walletId):
            SecretWordsScreen(id: walletId)
        case let .transactionDetails(id: id, details: details):
            TransactionsDetailScreen(id: id, transactionDetails: details)
        case let .send(sendRoute):
            SendFlowContainer(sendRoute: sendRoute)
        case let .coinControl(route):
            if #available(iOS 26, *) {
                CoinControlContainer(route: route)
            } else {
                CoinControlContainer(route: route)
                    .tint(.blue)
            }
        }
    }
    .environment(app)
}
