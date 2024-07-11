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
    @Binding var isShowing: Bool
    let currentRoute: Route

    let screenWidth = UIScreen.main.bounds.width

    var walletsIsEmpty: Bool {
        if let walletsIsEmpty = try? Database().wallets().isEmpty() {
            return walletsIsEmpty
        }

        return true
    }

    func setForeground(_ route: Route) -> LinearGradient {
        if RouteFactory().isSameParentRoute(route: route, routeToCheck: currentRoute) {
            return
                LinearGradient(
                    colors: [
                        Color.blue,
                        Color.blue.opacity(0.9),
                    ],
                    startPoint: .topLeading,
                    endPoint: .bottomTrailing
                )
        } else {
            return
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
        ZStack {
            if isShowing {
                Rectangle()
                    .foregroundStyle(.background.opacity(0.9))
                    .ignoresSafeArea()
                    .onTapGesture {
                        withAnimation {
                            isShowing = false
                        }
                    }

                HStack(alignment: .top) {
                    VStack(alignment: .leading, spacing: 30) {
                        Spacer()

                        Button(action: { goTo(RouteFactory().newWalletSelect()) }) {
                            Label("New Wallet", systemImage: "wallet.pass.fill")
                                .foregroundStyle(
                                    setForeground(RouteFactory().newWalletSelect())
                                )
                                .padding(.leading, 30)
                        }

                        if !walletsIsEmpty {
                            Button(action: { goTo(Route.listWallets) }) {
                                Label("Change Wallet", systemImage: "arrow.uturn.right.square.fill")
                                    .foregroundStyle(
                                        setForeground(Route.listWallets)
                                    )
                                    .padding(.leading, 30)
                            }
                        }

                        Spacer()
                        HStack(alignment: .center) {
                            Button(action: { goTo(.settings) }, label: {
                                HStack {
                                    Image(systemName: "gear")
                                        .foregroundStyle(Color.primary.gradient.opacity(0.5))
                                    Text("Settings")
                                        .foregroundStyle(Color.primary.gradient)
                                }
                            })
                            .frame(maxWidth: screenWidth * 0.75)
                        }
                    }
                    .frame(maxWidth: screenWidth * 0.75, maxHeight: .infinity, alignment: .leading)
                    .background(.thickMaterial)

                    Spacer()
                }
                .transition(.move(edge: .leading))
            }
        }
        .enableInjection()
    }

    #if DEBUG
        @ObserveInjection var forceRedraw
    #endif

    func goTo(_ route: Route) {
        isShowing = false

        if walletsIsEmpty && route == Route.newWallet(.select) {
            return app.resetRoute(to: RouteFactory().newWalletSelect())
        } else {
            navigate(route)
        }
    }
}

#Preview {
    ZStack {
        SidebarView(isShowing: Binding.constant(true), currentRoute: Route.listWallets)
    }
    .background(Color.white)
}
