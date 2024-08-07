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
            self.route = newRoute
        }
        .enableInjection()
    }

    #if DEBUG
        @ObserveInjection var forceRedraw
    #endif
}

@MainActor @ViewBuilder
func routeToView(model: MainViewModel, route: Route) -> some View {
    switch route {
    case .settings:
        SettingsView()
    case .listWallets:
        ListWalletsView(model: model)
    case let .newWallet(route: route):
        NewWalletView(route: route)
    case let .selectedWallet(walletId):
        SelectedWalletView(id: walletId)
    }
}
