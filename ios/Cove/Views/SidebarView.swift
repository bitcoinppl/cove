//
//  SidebarView.swift
//  Cove
//
//  Created by Praveen Perera on 7/1/24.
//

import SwiftUI

struct SidebarView: View {
    @Environment(MainViewModel.self) private var app
    @Environment(\.navigate) private var navigate

    let currentRoute: Route

    func setForeground(_ route: Route) -> LinearGradient {
        if RouteFactory().isSameParentRoute(route: route, routeToCheck: currentRoute) {
            LinearGradient(
                colors: [
                    Color.blue,
                    Color.blue.opacity(0.9),
                ],
                startPoint: .topLeading,
                endPoint: .bottomTrailing
            )
        } else {
            LinearGradient(
                colors: [
                    Color.primary.opacity(0.8), Color.primary.opacity(0.7),
                ],
                startPoint: .topLeading,
                endPoint: .bottomTrailing
            )
        }
    }

    var body: some View {
        HStack(alignment: .top) {
            VStack(spacing: 40) {
                Spacer()

                Button(action: { goTo(RouteFactory().newWalletSelect()) }) {
                    Label("Add Wallet", systemImage: "wallet.pass.fill")
                        .foregroundStyle(.white)
                        .font(.headline)
                        .background(Color.blue)
                        .cornerRadius(10)
                }

                if app.numberOfWallets > 1 {
                    Button(action: { goTo(Route.listWallets) }) {
                        Label("Change Wallet", systemImage: "arrow.uturn.right.square.fill")
                            .foregroundStyle(.white)
                            .font(.headline)
                            .background(Color.blue)
                            .cornerRadius(10)
                    }
                }

                Spacer()
                HStack(alignment: .center) {
                    Button(
                        action: { goTo(.settings) },
                        label: {
                            HStack {
                                Image(systemName: "gear")
                                    .foregroundStyle(Color.white.gradient.opacity(0.5))

                                Text("Settings")
                                    .foregroundStyle(Color.white.gradient)
                            }
                        }
                    )
                }
            }
        }
        .frame(maxWidth: .infinity)
        .background(.blue)
    }

    func goTo(_ route: Route) {
        app.isSidebarVisible = false

        if !app.hasWallets, route == Route.newWallet(.select) {
            return app.resetRoute(to: RouteFactory().newWalletSelect())
        } else {
            navigate(route)
        }
    }
}

#Preview {
    SidebarView(currentRoute: Route.listWallets)
        .environment(MainViewModel())
        .background(Color.white)
}
