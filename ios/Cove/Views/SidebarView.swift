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
    let wallets: [WalletMetadata]

    init(currentRoute: Route, wallets: [WalletMetadata]? = nil) {
        self.currentRoute = currentRoute
        if let wallets {
            self.wallets = wallets
            return
        }

        do {
            self.wallets = try Database().wallets().all()
            Log.debug("wallets: \(self.wallets)")
        } catch {
            Log.error("Failed to get wallets \(error)")
            self.wallets = []
        }
    }

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
            VStack {
                HStack(alignment: .top) {
                    Image(.icon)
                        .resizable()
                        .frame(width: 65, height: 65)
                        .clipShape(Circle())

                    Spacer()

                    Button(action: app.nfcReader.scan) {
                        Image(systemName: "wave.3.right")
                    }
                    .foregroundStyle(.white)
                }

                Divider()
                    .overlay(Color(.systemGray5))
                    .opacity(0.50)
                    .padding(.vertical, 22)

                HStack {
                    Text("My Wallets")
                        .font(.subheadline)
                        .fontWeight(.medium)
                        .foregroundStyle(.white)

                    Spacer()
                }
                .padding(.bottom, 16)

                GeometryReader { proxy in
                    VStack(spacing: 12) {
                        ForEach(wallets, id: \.id) { wallet in
                            Button(action: {
                                goTo(Route.selectedWallet(wallet.id))
                            }) {
                                HStack(spacing: 20) {
                                    Circle()
                                        .fill(Color(wallet.color))
                                        .frame(width: 6, height: 6)

                                    Text(wallet.name)
                                        .font(.footnote)
                                        .fontWeight(.medium)
                                        .foregroundStyle(.white)
                                        .lineLimit(1)
                                        .minimumScaleFactor(0.80)
                                }
                                .frame(width: proxy.size.width / 2, alignment: .leading)
                            }
                            .frame(maxWidth: .infinity)
                            .padding()
                            .background(Color(.systemGray5).opacity(0.06))
                            .cornerRadius(10)
                        }
                    }
                }

                Spacer()

                Divider()
                    .overlay(Color(.systemGray5))
                    .opacity(0.50)

                HStack {
                    Button(action: { goTo(RouteFactory().newWalletSelect()) }) {
                        HStack(spacing: 20) {
                            Image(systemName: "wallet.bifold")
                            Text("Add Wallet")
                        }
                        .foregroundColor(.white)
                    }

                    Spacer()
                }

                HStack {
                    Button(action: { goTo(Route.settings) }) {
                        HStack(spacing: 22) {
                            Image(systemName: "gear")
                            Text("Settings")
                        }
                        .foregroundColor(.white)
                    }

                    Spacer()
                }
            }
        }
        .padding(20)
        .frame(maxWidth: .infinity)
        .background(.midnightBtn)
    }

    func goTo(_ route: Route) {
        app.isSidebarVisible = false

        if case Route.selectedWallet = route {
            return app.loadAndReset(to: route)
        }

        if !app.hasWallets, route == Route.newWallet(.select) {
            return app.resetRoute(to: RouteFactory().newWalletSelect())
        }

        navigate(route)
    }
}

#Preview {
    HStack {
        SidebarView(
            currentRoute: Route.listWallets,
            wallets: [
                WalletMetadata("Test Wallet", preview: true),
                WalletMetadata("Second Wallet", preview: true),
                WalletMetadata("Coldcard Q1", preview: true),
            ]
        )
        .environment(MainViewModel())
        .background(Color.white)
        .frame(width: 280)

        Spacer()
    }
}
